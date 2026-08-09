//! Stack-effect checker. Simulates a compile-time virtual stack of concrete
//! `Type`s through each word body and verifies the net effect matches the
//! declared signature.
//!
//! Every operand is checked against the type its consumer expects, so a
//! `bool` where `+` wants an `i64` is a located compile error (Forth's silent
//! coercion failure mode becomes a diagnostic here). Branch join points unify
//! on both depth and per-slot type: the `then` and `else` arms must leave the
//! same stack shape.

use std::collections::{HashMap, HashSet};

use crate::ast::{
    instantiation_symbol, intern_array_type, intern_bundle_struct, intern_owned_cell_type,
    intern_ref_type, ArrayDecl, Bound, CallInst, Clause, EnumDecl, EnumId, ExternDecl, Len, Module,
    OwnedCellDecl, PolySig, PolyType, QuotEffect, RefDecl, Span, StackEffect, StructDecl, StructId,
    Subst, Term, TermKind, Type, TypedSlot, VariantDecl, WordBody, WordDef,
};

/// A word's typed stack effect: the concrete input and output slot types,
/// deepest-first (leftmost in `( … )` is deepest on the stack).
#[derive(Debug, Clone)]
pub struct Sig {
    pub inputs: Vec<Type>,
    pub outputs: Vec<Type>,
}

/// The typed effect of a declared word.
pub fn sig_of(effect: &StackEffect) -> Sig {
    Sig {
        inputs: effect.inputs.iter().map(|s| s.ty).collect(),
        outputs: effect.outputs.iter().map(|s| s.ty).collect(),
    }
}

/// R5/R14: the polymorphic-call context threaded through the monomorphic body
/// walk: the `PolySig`s of every polymorphic word (looked up before the
/// concrete `env`), and the instantiation table each unified call site writes
/// into. A monomorphic body that never calls a polymorphic word touches
/// neither; the REPL (`infer_line`) passes an empty one, so no `repl.rs`
/// change is needed (D2).
///
/// R2b: each `PolySig` carries its generation alongside it (`None` natively,
/// `Some(g)` for a REPL word retained at generation `g`, Slice 2), so
/// `check_poly_call`'s mint reads both from one lookup with no second
/// channel.
struct PolyCtx<'a> {
    env: &'a HashMap<String, (PolySig, Option<u64>)>,
    insts: &'a mut HashMap<Span, CallInst>,
    /// Slice 6a (R18): the monomorphic quotation-taking words, keyed by name,
    /// so a call to one is intercepted and its body spliced against the live
    /// stack (the compiler's only inliner) rather than lowered to an
    /// `Instr::Call` to a word that mints no `IrFunc` (R20). Empty on the REPL
    /// paths, where defining such a word is rejected up front (R23).
    combinators: &'a HashMap<String, Combinator<'a>>,
}

/// Slice 6a (R18): one monomorphic quotation-taking word available to inline.
/// Both fields are shared references into the module, so a `Combinator` is a
/// pair of pointers (`Copy`), which lets a call site copy it out of the
/// borrowed map and then reborrow `PolyCtx` mutably for the splice.
#[derive(Clone, Copy)]
pub(crate) struct Combinator<'a> {
    word: &'a WordDef,
    terms: &'a [Term],
}

/// One simulated stack slot: its concrete `Type`, plus whether it is a bare,
/// as-yet-unconverted integer literal fresh off an `IntLit` term. `Type`
/// alone can't express D8's literal-coercion carve-out (an integer literal
/// unifies with a `usize` position without an explicit `>usize`, but a
/// *computed* `i64` may not, X10), so the checker's internal stack carries
/// this flag alongside every `Type` it already tracked. It never escapes
/// `check.rs`: every external-facing function (`infer_line`, `check_outputs`'
/// callers) still speaks plain `Type`. A shuffle (`dup`/`swap`/`over`/`rot`)
/// moves a `Slot` verbatim, so a literal duplicated by `dup` is still a
/// literal at each copy; any operator, conversion, or word call produces a
/// non-literal result (D8: no constant folding, no comptime interpreter).
/// An index into a per-check `Provenance::quotations` table (D2): a
/// quotation `Slot` carries the identity of the literal body it marks, so
/// `call`/`times` can splice that body at the consumption site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QuotId(usize);

/// The identity a quotation `Slot` carries (D2/R4). A single variant: two
/// *different* quotations at a branch join are rejected at the join (R7), so
/// no poisoned/merged marker is ever carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuotRef {
    Known(QuotId),
}

/// One interned quotation literal: its body terms (spliced at `call`/`times`)
/// and the literal's span, for a located diagnostic.
#[derive(Debug, Clone)]
struct QuotBody {
    body: Vec<Term>,
    #[allow(dead_code)]
    span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    ty: Type,
    literal: bool,
    /// The integer value of a bare `IntLit` slot (`None` for any computed
    /// value). Load-bearing for the two compile-time-count array positions:
    /// `fill`'s count `N` (M1) and a constant-index bounds check (X4, R11).
    /// Moved verbatim by a shuffle (a duped literal keeps its value), cleared
    /// by any operator/conversion/word call or branch merge (D8: no folding).
    int_val: Option<i64>,
    /// Which region this aggregate value denotes, and where this name for
    /// it was pushed.
    alias: Option<Alias>,
    /// The outstanding derivation a reference-typed value holds.
    deriv: Option<DerivId>,
    /// D2/R4: set iff this is a quotation marker, carrying the identity of the
    /// literal body it stands for. A `Cstr` placeholder `ty` no user op
    /// accepts rides alongside it; a shuffle forwards this verbatim (`Slot` is
    /// `Copy`), and `call`/`times` consume it by splicing the body.
    quot: Option<QuotRef>,
}

impl Slot {
    /// A slot holding a computed (non-literal) value of `ty`: every path but
    /// a bare `IntLit` push produces one of these.
    fn computed(ty: Type) -> Slot {
        Slot {
            ty,
            literal: false,
            int_val: None,
            alias: None,
            deriv: None,
            quot: None,
        }
    }

    /// The same value, reached through a reference derived from `deriv`: a
    /// projection's result, which keeps its parent's provenance so the
    /// place it traces back to stays findable however many steps away it is.
    fn derived(ty: Type, deriv: Option<DerivId>) -> Slot {
        Slot {
            deriv,
            ..Slot::computed(ty)
        }
    }
}

/// Whether `ty` is one of the two target-width size types (`usize`/`isize`):
/// both share the D8 literal-coercion carve-out against a bare `i64`
/// literal, so `match_slot`/`unify_pair` gate on this rather than on `Usize`
/// alone. `usize` and `isize` never coerce into *each other* here: the guard
/// only ever fires against `Type::I64`, so mixing the two size types falls
/// through to a plain mismatch, naming both backticked types.
fn is_size_type(ty: Type) -> bool {
    matches!(ty, Type::Usize | Type::Isize)
}

/// The outcome of matching one `Slot` against a single expected `Type`
/// (a word-call argument, a declared output slot, or a binary operator's
/// second operand once the first has picked a target type): exact, D8's
/// literal coercion into a `usize`/`isize` position, the specific "needs an
/// explicit conversion" diagnostic (X10) for a *computed* value in that
/// position, or a plain mismatch.
enum SlotMatch {
    Exact,
    LiteralSizeType,
    NeedsSizeConversion,
    /// R7: a `str` found where a `cstr` is wanted. Never coerces silently
    /// (there is no implicit conversion, only the explicit `cstr` word), so
    /// this is its own case rather than falling into `Mismatch`, exactly as
    /// `NeedsSizeConversion` is split from a plain mismatch above.
    NeedsStrToCstrConversion,
    Mismatch,
}

fn match_slot(found: Slot, want: Type) -> SlotMatch {
    if found.ty == want {
        return SlotMatch::Exact;
    }
    if is_size_type(want) && found.ty == Type::I64 {
        return if found.literal {
            SlotMatch::LiteralSizeType
        } else {
            SlotMatch::NeedsSizeConversion
        };
    }
    if want == Type::Cstr && found.ty == Type::Str {
        return SlotMatch::NeedsStrToCstrConversion;
    }
    SlotMatch::Mismatch
}

/// The result of unifying two `Slot`s for a homogeneous binary operator
/// (`+ - * = < > <= >= <> mod and or xor`): the operands' common `Type` once
/// D8's literal coercion is applied (a `usize`/`isize` paired with a bare
/// integer literal unifies to that size type), the X10 diagnostic's target
/// type for a size type paired with a *computed* `i64` instead, or a plain
/// mismatch.
enum PairMatch {
    Ok(Type),
    NeedsSizeConversion(Type),
    Mismatch,
}

fn unify_pair(a: Slot, b: Slot) -> PairMatch {
    if a.ty == b.ty {
        return PairMatch::Ok(a.ty);
    }
    match (a.ty, b.ty) {
        (w, Type::I64) if is_size_type(w) && b.literal => PairMatch::Ok(w),
        (Type::I64, w) if is_size_type(w) && a.literal => PairMatch::Ok(w),
        (w, Type::I64) | (Type::I64, w) if is_size_type(w) => PairMatch::NeedsSizeConversion(w),
        _ => PairMatch::Mismatch,
    }
}

/// The builtin word -> typed-effect table, as the seed of a checking env.
/// Every builtin is handled directly in `check_term`
/// (`check_shuffle`/`check_operator`): the stack shuffles, the numeric-tower
/// operators, and `.` (type-directed over any printable scalar, not a fixed
/// `( i64 -- )`) all dispatch on the concrete operand type rather than a fixed
/// signature, so this table is empty.
pub fn builtin_table() -> HashMap<String, Sig> {
    HashMap::new()
}

/// R2/R7: whether `ty` is `Copy` (freely duplicated and discarded) rather than
/// linear (used exactly once, disposed by `drop`). A struct or enum is linear
/// iff any field/variant-payload field is (transitively), so a
/// struct-of-struct-of-resource or an enum carrying one is linear too.
/// `structs`/`enums` resolve a `Type::Struct`/`Type::Enum`'s fields; neither
/// can recurse into itself (`check_recursion` rejects that first), so this
/// always terminates.
///
/// R3 (slice 8b): a struct with a user `drop` overload is linear whatever its
/// fields say — a resource wrapping one `i64` would otherwise be `Copy` by
/// the structural fold alone, and so silently duplicated and forgotten.
pub fn is_copy(ty: Type, structs: &[StructDecl], enums: &[EnumDecl], arrays: &[ArrayDecl]) -> bool {
    match ty {
        Type::Struct(id, _) if structs[id.index()].has_drop_overload => false,
        Type::Struct(id, _) => structs[id.index()]
            .fields
            .iter()
            .all(|(_, field_ty)| is_copy(*field_ty, structs, enums, arrays)),
        Type::Enum(id, _) => enums[id.index()]
            .variants
            .iter()
            .flat_map(|v| v.fields.iter())
            .all(|(_, field_ty)| is_copy(*field_ty, structs, enums, arrays)),
        Type::Array(id, _) => is_copy(arrays[id.index()].element, structs, enums, arrays),
        // Always linear regardless of payload, so no payload lookup here.
        Type::OwnedCell(_, _) => false,
        // A shared reference is freely duplicated and discarded; a mutable one
        // is not (the exclusivity rule, which `dup`'s Copy gate already enforces).
        Type::Ref(_, mutable, _) => !mutable,
        _ => true,
    }
}

/// Whether `ty` carries an exactly-once obligation: used exactly once,
/// disposed by `drop`, tracked by `Moves`. This is *not* the negation of
/// `is_copy`: `&!T` is neither `Copy` nor linear, so it is duplicated
/// by nothing and owed to nothing — a reference local expires silently at the
/// end of its block, and a reference is never dragged into move tracking.
pub fn is_linear(
    ty: Type,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> bool {
    !ty.is_ref() && !is_copy(ty, structs, enums, arrays)
}

/// Whether `ty` **transitively contains** a reference — is one itself, or
/// reaches one through a struct field, an enum variant payload, or an array
/// element. The predicate every escape rejection is stated over, so a
/// reference cannot slip into storage one level down from a declaration site.
///
/// A `^T` payload is deliberately *not* followed: a cell may close a type
/// cycle (`^List` inside `List`), so following one would not terminate.
/// `check_no_stored_references` sweeps the interned cell registry directly
/// instead, which reaches every payload shape a program can name without
/// recursing.
fn contains_reference(
    ty: Type,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> bool {
    match ty {
        Type::Ref(..) => true,
        Type::Struct(id, _) => structs[id.index()]
            .fields
            .iter()
            .any(|(_, f)| contains_reference(*f, structs, enums, arrays)),
        Type::Enum(id, _) => enums[id.index()]
            .variants
            .iter()
            .flat_map(|v| v.fields.iter())
            .any(|(_, f)| contains_reference(*f, structs, enums, arrays)),
        Type::Array(id, _) => {
            contains_reference(arrays[id.index()].element, structs, enums, arrays)
        }
        _ => false,
    }
}

/// Which region of memory an aggregate value denotes. Two slots carrying
/// the same id are two names for one address, which is what makes a mutation
/// through one silently observable through the other. `None` means "denotes a
/// region nothing else names": every value is born that way, and an aggregate
/// is given an id lazily, the first time something could alias it (a binding,
/// or a non-consuming projection out of it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegionId(u32);

/// One live name for a region, and where that name was pushed. The span is
/// what lets the alias check report a *stack-resident* alias, which has no name
/// of its own to cite: an aggregate spends most of its life on the virtual
/// stack in this language, so the ability to locate one there is the difference
/// between catching the hazard and only catching the spelling of it where
/// both ends happen to be bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Alias {
    set: AliasSetId,
    span: Span,
}

/// The regions one value may denote, interned in `Provenance` so a `Slot` can
/// carry them by id and stay `Copy`. A value denotes more than one region only
/// where control flow merged two arms that named different places: the merge
/// cannot know which arm ran, so it keeps both and the borrow check tests every
/// member. Same trick as `DerivId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AliasSetId(u32);

/// One outstanding derivation from a place, interned in `Provenance` so a
/// `Slot` can carry it by id and stay `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DerivId(u32);

/// What one live reference traces back to. Created by a fresh borrow
/// (`&v`/`&!v`), by naming a reference local (a reborrow), and by every
/// projection step, which copies its parent's chain rather than starting a new
/// one — that is what lets a check ask "does anything still trace back to this
/// place" without scanning for a particular value's identity.
#[derive(Debug, Clone)]
struct Deriv {
    /// The place the reference was taken from: an owned aggregate local for a
    /// fresh borrow, the reference local itself for a reborrow.
    place: String,
    /// The owned local at the bottom of the chain, if the chain starts at one.
    /// A reborrow of a reference *parameter* has none: its referent lives in an
    /// ancestor frame, so there is no place in this body to protect.
    owned_root: Option<String>,
    /// Whether `place` is a reference local this was reborrowed from, which is
    /// what the suspend rule is keyed on. Cleared when the derivation is bound
    /// into a local: the binding consumes the reborrow, which is why
    /// `push-byte` may name `b` three times.
    reborrow: bool,
    mutable: bool,
    /// Whether any projection step stands between the place and this
    /// reference: the path-disjointness note is only apt when one does.
    projected: bool,
    /// Where the borrow was taken, so a conflict can name both ends.
    span: Span,
}

impl Deriv {
    /// The places this derivation keeps suspended, which is what a branch
    /// join has to agree on. Both halves are consulted by a hazard check
    /// (`owned_root` by the consume/borrow-conflict scans, the reborrowed
    /// reference local by the suspend rule), so both belong in the key: a join
    /// keeps only one arm's derivation, and any place the discarded arm
    /// suspended would silently stop being protected.
    fn suspension(&self) -> (Option<&str>, Option<&str>) {
        (
            self.owned_root.as_deref(),
            (self.reborrow && self.mutable).then_some(self.place.as_str()),
        )
    }
}

/// The per-body provenance arenas: which place each live reference traces back
/// to and which region each aggregate value denotes. Threaded `&mut`
/// through the walk rather than kept in `Scope`, which an `if` arm clones: ids
/// stay unique across the arms, and a record outlives the arm that made it.
/// R6: the self-tail combinator whose body is currently being spliced. `name`
/// is the mangled combinator name (matched against a self-call inside the
/// splice); `input_count` is its declared input arity, so the back-edge can
/// find the carried state row (the non-quotation inputs) below the arguments.
#[derive(Debug, Clone)]
struct SelfTailMarker {
    name: String,
    input_count: usize,
}

#[derive(Debug, Default)]
struct Provenance {
    derivs: Vec<Deriv>,
    regions: u32,
    /// The interned region of one non-consuming projection out of a parent
    /// region, so two peeks of the same field yield one id.
    fields: HashMap<(u32, String), RegionId>,
    /// The interned region sets, indexed by `AliasSetId`.
    alias_sets: Vec<Vec<RegionId>>,
    /// Each field region's immediate parent: a name for a struct's
    /// field is still a name for part of the whole struct, so the alias check
    /// has to test region *overlap* along this chain, not bare equality.
    parents: HashMap<u32, RegionId>,
    /// R6 (slice 8b): the resolved operand type of every `drop` call site in
    /// this body, in the order the walk reaches them. Nothing in the walk
    /// reads it back; the body walkers hand it to `check`, which needs the
    /// *type* each `drop` resolves to in order to tell `drop@File` from a
    /// `drop` of a plain `i64` — a distinction no purely syntactic pass over
    /// callee names can make. It rides this arena for the same reason the
    /// arena is threaded at all: an `if` arm clones `Scope`, so an
    /// observation kept there would die with the arm.
    dropped: Vec<Type>,
    /// D2/R4: the per-check quotation-literal side table, indexed by `QuotId`.
    /// A quotation `Slot`/`Binding` carries only a `QuotId`, so the body it
    /// marks is interned here and spliced from here at `call`/`times`. Rides
    /// this arena because it is the one scratch already threaded `&mut`
    /// through the walk, so a quotation pushed in one `if` arm and read in a
    /// merge outlives the arm's cloned `Scope`.
    quotations: Vec<QuotBody>,
    /// R6/R14: the self-tail combinator currently being spliced (its name and
    /// its declared input arity), set for the duration of that body splice. A
    /// tail-position call to that same name reached inside the spliced body is
    /// the loop back-edge, not a re-splice: it discharges the two move/borrow
    /// obligations and produces the combinator's carried state, terminating
    /// the branch. Saved and restored around the splice so loops compose.
    self_tail_combinator: Option<SelfTailMarker>,
    /// R18/R21: a monotonic counter minting a fresh suffix each time a
    /// combinator body is spliced, so the callee's `| ... |` locals are
    /// alpha-renamed to names no caller local (or outer combinator, under
    /// transitive inlining) can collide with. Term-splice binds names in the
    /// caller's scope (R18: binding, not string rewriting), so without this a
    /// nested `each` inside a `map` would re-bind the outer `arr`/`f`.
    inline_uid: u32,
}

impl Provenance {
    fn fresh_region(&mut self) -> RegionId {
        let id = RegionId(self.regions);
        self.regions += 1;
        id
    }

    fn intern_alias_set(&mut self, mut regions: Vec<RegionId>) -> AliasSetId {
        regions.sort_unstable_by_key(|r| r.0);
        regions.dedup();
        if let Some(i) = self.alias_sets.iter().position(|s| *s == regions) {
            return AliasSetId(i as u32);
        }
        self.alias_sets.push(regions);
        AliasSetId((self.alias_sets.len() - 1) as u32)
    }

    fn alias_set_of(&mut self, region: RegionId) -> AliasSetId {
        self.intern_alias_set(vec![region])
    }

    fn alias_regions(&self, id: AliasSetId) -> &[RegionId] {
        &self.alias_sets[id.0 as usize]
    }

    /// Both arms' regions, since either runtime path may have produced the
    /// merged value.
    fn alias_union(&mut self, a: AliasSetId, b: AliasSetId) -> AliasSetId {
        let mut regions = self.alias_regions(a).to_vec();
        regions.extend_from_slice(self.alias_regions(b));
        self.intern_alias_set(regions)
    }

    /// Whether any region of one value overlaps any region of the other.
    fn alias_sets_overlap(&self, a: AliasSetId, b: AliasSetId) -> bool {
        self.alias_regions(a).iter().any(|x| {
            self.alias_regions(b)
                .iter()
                .any(|y| self.regions_overlap(*x, *y))
        })
    }

    /// The same field projected out of every region the parent may denote.
    fn field_alias_set(&mut self, parent: AliasSetId, segment: &str) -> AliasSetId {
        let parents = self.alias_regions(parent).to_vec();
        let mut fields = Vec::with_capacity(parents.len());
        for region in parents {
            fields.push(self.field_region(region, segment));
        }
        self.intern_alias_set(fields)
    }

    /// The region an interior value of `parent` denotes, interned per path
    /// segment, so two non-consuming projections of the same field of the same
    /// parent are recognised as two names for one address.
    fn field_region(&mut self, parent: RegionId, segment: &str) -> RegionId {
        let key = (parent.0, segment.to_string());
        if let Some(id) = self.fields.get(&key) {
            return *id;
        }
        let id = self.fresh_region();
        self.fields.insert(key, id);
        self.parents.insert(id.0, parent);
        id
    }

    /// Whether `a` and `b` denote overlapping storage — the same region,
    /// or one an ancestor of the other along the field-projection chain.
    /// Mirrors the conservative field-borrow rule on the naming side: a name
    /// for an interior is still a name for (part of) its parent, so equality
    /// alone misses the aliasing a peeked field's binding creates.
    fn regions_overlap(&self, a: RegionId, b: RegionId) -> bool {
        a == b || self.is_ancestor(a, b) || self.is_ancestor(b, a)
    }

    fn is_ancestor(&self, ancestor: RegionId, mut descendant: RegionId) -> bool {
        while let Some(&parent) = self.parents.get(&descendant.0) {
            if parent == ancestor {
                return true;
            }
            descendant = parent;
        }
        false
    }

    fn deriv(&self, id: DerivId) -> &Deriv {
        &self.derivs[id.0 as usize]
    }

    fn add(&mut self, deriv: Deriv) -> DerivId {
        let id = DerivId(self.derivs.len() as u32);
        self.derivs.push(deriv);
        id
    }

    /// A fresh borrow of an owned aggregate place.
    fn borrow(&mut self, place: &str, mutable: bool, span: Span) -> DerivId {
        self.add(Deriv {
            place: place.to_string(),
            owned_root: Some(place.to_string()),
            reborrow: false,
            mutable,
            projected: false,
            span,
        })
    }

    /// Naming a reference local reborrows it — a new chain rooted at that
    /// local, keeping whatever owned place the old chain had already reached.
    fn reborrow(
        &mut self,
        place: &str,
        held: Option<DerivId>,
        mutable: bool,
        span: Span,
    ) -> DerivId {
        let owned_root = held.and_then(|id| self.deriv(id).owned_root.clone());
        self.add(Deriv {
            place: place.to_string(),
            owned_root,
            reborrow: true,
            mutable,
            projected: false,
            span,
        })
    }

    /// One projection step — the same place, one step further from it.
    fn project(&mut self, parent: Option<DerivId>) -> Option<DerivId> {
        let deriv = Deriv {
            projected: true,
            ..self.deriv(parent?).clone()
        };
        Some(self.add(deriv))
    }

    /// Binding a reference into a local consumes the reborrow it came from,
    /// so the place it was reborrowed from is suspended no longer. The owned
    /// root survives: the local still keeps its referent borrowed.
    fn bind(&mut self, held: Option<DerivId>) -> Option<DerivId> {
        let deriv = Deriv {
            reborrow: false,
            ..self.deriv(held?).clone()
        };
        Some(self.add(deriv))
    }
}

/// R14: the move-state of one linear local, a three-value lattice. `Moved` and
/// `MaybeMoved` carry the site that consumed the value, so a later use can name
/// it; `MaybeMoved` is the join of disagreeing arms (consumed on one path only),
/// which is neither usable nor accepted as disposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveState {
    Live,
    Moved(Span),
    MaybeMoved(Span),
}

/// The move-state of every *linear* name in scope, carried by the `Scope` the
/// walker threads (R14). A Copy local never appears: it carries no ownership
/// obligation, so mentioning it twice is ordinary reuse.
#[derive(Debug, Clone, Default)]
struct Moves {
    states: HashMap<String, MoveState>,
}

impl Moves {
    /// R3 (D2): mentioning a linear local moves its value out. `Ok(())` for a
    /// Copy local (absent from the map) or a first mention; `Err(site)` names
    /// the move that already consumed it.
    fn take(&mut self, name: &str, span: Span) -> Result<(), Span> {
        match self.states.get(name) {
            None => Ok(()),
            Some(MoveState::Live) => {
                self.states.insert(name.to_string(), MoveState::Moved(span));
                Ok(())
            }
            Some(MoveState::Moved(site) | MoveState::MaybeMoved(site)) => Err(*site),
        }
    }

    /// The site that already consumed `name`, if any. Read-only companion to
    /// `take`: a borrow is not a move, but a consumed local is no longer a
    /// valid borrow root — its value, and any heap it owned, is gone.
    fn moved_site(&self, name: &str) -> Option<Span> {
        match self.states.get(name) {
            Some(MoveState::Moved(site) | MoveState::MaybeMoved(site)) => Some(*site),
            _ => None,
        }
    }

    /// The locals still holding an unconsumed value: `Live` (never mentioned)
    /// or `MaybeMoved` (consumed on one branch only), name-sorted so a scope
    /// with two of them always reports the same one.
    fn unconsumed(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .states
            .iter()
            .filter(|(_, st)| !matches!(st, MoveState::Moved(_)))
            .map(|(name, _)| name.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    /// R14: combine two `if` arms at the join. Equal states are preserved; any
    /// disagreement (`Live` vs `Moved`, or anything vs `MaybeMoved`) yields
    /// `MaybeMoved`, carrying whichever arm's move site exists, so the value is
    /// neither usable past the join nor counted as disposed at scope end. The
    /// checker never inserts a compensating drop.
    fn join(then_arm: Moves, else_arm: Moves) -> Moves {
        let mut states = then_arm.states;
        for (name, state) in states.iter_mut() {
            let other = else_arm.states[name.as_str()];
            *state = match (*state, other) {
                (MoveState::Live, MoveState::Live) => MoveState::Live,
                // Consumed on both paths (at two different sites, which is
                // still exactly once at runtime), so the join stays `Moved`.
                (MoveState::Moved(site), MoveState::Moved(_)) => MoveState::Moved(site),
                (MoveState::Moved(site) | MoveState::MaybeMoved(site), _)
                | (_, MoveState::Moved(site) | MoveState::MaybeMoved(site)) => {
                    MoveState::MaybeMoved(site)
                }
            };
        }
        Moves { states }
    }
}

/// The names in scope while a body is walked, innermost-last so that leaving a
/// block truncates back to its entry depth (R2, R9), paired with the
/// move-state of the linear ones (R14). Mid-body binding is why this is a
/// threaded `&mut` value rather than a map computed before the walk: the set of
/// names in scope changes as terms are visited.
#[derive(Debug, Clone, Default)]
struct Scope {
    bound: Vec<Binding>,
    moves: Moves,
}

/// One name in scope, with the provenance a borrow check reads off it: which
/// region an aggregate binding denotes and which derivation a reference
/// binding holds.
#[derive(Debug, Clone)]
struct Binding {
    name: String,
    ty: Type,
    aliases: Option<AliasSetId>,
    deriv: Option<DerivId>,
    /// D2/R4: a bound quotation's marker. A local read reconstructs a fresh
    /// `Slot` that drops every non-`ty` side channel, so unlike a shuffle a
    /// bind is a *second*, explicit forwarding site: this field carries the
    /// marker across the bind and back onto the reconstructed slot.
    quot: Option<QuotRef>,
}

impl Scope {
    fn depth(&self) -> usize {
        self.bound.len()
    }

    fn local(&self, name: &str) -> Option<&Binding> {
        self.bound.iter().find(|b| b.name == name)
    }

    fn local_type(&self, name: &str) -> Option<Type> {
        self.local(name).map(|b| b.ty)
    }

    /// Bring `name` into scope. A linear value also enters the move-state map,
    /// so forgetting it is caught at the end of its block. An aggregate
    /// with no region of its own gets one here: a binding is the first point at
    /// which a second name could denote the same address.
    fn bind(&mut self, name: &str, slot: Slot, linear: bool, prov: &mut Provenance) {
        if linear {
            self.moves.states.insert(name.to_string(), MoveState::Live);
        }
        let aliases = match (slot.alias, slot.ty.is_aggregate()) {
            (Some(alias), _) => Some(alias.set),
            (None, true) => {
                let region = prov.fresh_region();
                Some(prov.alias_set_of(region))
            }
            (None, false) => None,
        };
        self.bound.push(Binding {
            name: name.to_string(),
            ty: slot.ty,
            aliases,
            deriv: prov.bind(slot.deriv),
            quot: slot.quot,
        });
    }

    /// Take every name bound past `depth` out of scope, returning the first one
    /// (name-sorted, so a block leaking two always reports the same one) still
    /// holding a linear value.
    fn leave(&mut self, depth: usize) -> Option<(String, Type, MoveState)> {
        let mut leaked: Vec<(String, Type, MoveState)> = Vec::new();
        for binding in self.bound.split_off(depth) {
            match self.moves.states.remove(&binding.name) {
                Some(MoveState::Moved(_)) | None => {}
                Some(state) => leaked.push((binding.name, binding.ty, state)),
            }
        }
        leaked.sort_by(|a, b| a.0.cmp(&b.0));
        leaked.into_iter().next()
    }
}

/// Every derivation still live — held by a slot on the virtual stack, or by
/// a reference-typed local still in scope. A reference is live from the term
/// that creates it until the term that consumes its slot; a reference *local*
/// is live for the whole block.
fn live_derivs<'a>(stack: &'a [Slot], scope: &'a Scope) -> impl Iterator<Item = DerivId> + 'a {
    stack
        .iter()
        .filter_map(|slot| slot.deriv)
        .chain(scope.bound.iter().filter_map(|b| b.deriv))
}

/// The first live derivation satisfying `pred`. The scan is over
/// provenance, never over value identity: a reference two projection steps
/// removed from a place is still a derivation of that place.
fn live_deriv(
    stack: &[Slot],
    scope: &Scope,
    prov: &Provenance,
    mut pred: impl FnMut(&Deriv) -> bool,
) -> Option<DerivId> {
    live_derivs(stack, scope).find(|id| pred(prov.deriv(*id)))
}

/// A live borrow rooted at the owned place `place`, whatever its
/// mutability and however many projection steps away.
fn live_borrow_of(
    stack: &[Slot],
    scope: &Scope,
    prov: &Provenance,
    place: &str,
) -> Option<DerivId> {
    live_deriv(stack, scope, prov, |d| {
        d.owned_root.as_deref() == Some(place)
    })
}

/// The naming side: a live *mutable* borrow rooted at `place`, which any new
/// name for that place would then silently observe mutations through.
fn live_mutable_borrow_of(
    stack: &[Slot],
    scope: &Scope,
    prov: &Provenance,
    place: &str,
) -> Option<DerivId> {
    live_deriv(stack, scope, prov, |d| {
        d.mutable && d.owned_root.as_deref() == Some(place)
    })
}

/// The region a non-consuming projection out of `parent` denotes, for an
/// aggregate interior value (a scalar one is loaded into a temporary and denotes
/// no region). The parent is given a region of its own if it has none: it is
/// only here, where a second name for its interior can appear, that the
/// identity starts to matter. Both names are located at the projection, which is
/// where each of them enters play as an alias of the other.
fn peek_region(
    parent: &mut Slot,
    interior: Type,
    segment: &str,
    span: Span,
    prov: &mut Provenance,
) -> Option<Alias> {
    if !interior.is_aggregate() {
        return None;
    }
    let base = match parent.alias {
        Some(alias) => alias.set,
        None => {
            let region = prov.fresh_region();
            let set = prov.alias_set_of(region);
            parent.alias = Some(Alias { set, span });
            set
        }
    };
    Some(Alias {
        set: prov.field_alias_set(base, segment),
        span,
    })
}

/// Where a second live name for a region is, when the diagnostic has to
/// point at it. A bound local reports its name, which is what the user has to
/// change; a value still on the virtual stack has no name, so it reports the
/// site that pushed it instead.
enum AliasOrigin<'a> {
    Name(&'a str),
    Stack(Span),
}

/// Another live name denoting a region overlapping the local `place`'s —
/// the same region, or one nested inside the other's field chain (a name for
/// a field is still a name for part of the whole place). The scan covers the
/// virtual stack as well as the locals map, exactly as the reference-derivation scan does: a
/// concatenative body leaves aggregates on the stack constantly, so the
/// stack-resident alias is the *common* shape of this hazard rather than an edge
/// of it. A bound name is preferred over a stack slot when both alias, being the
/// more actionable end to report, and names are sorted so a place aliased twice
/// always reports the same one. A consumed local is not a name for anything, so
/// it never aliases.
fn aliasing_origin<'a>(
    stack: &[Slot],
    scope: &'a Scope,
    prov: &Provenance,
    place: &str,
) -> Option<AliasOrigin<'a>> {
    let set = scope.local(place)?.aliases?;
    let overlaps = |other: AliasSetId| prov.alias_sets_overlap(set, other);
    let mut names: Vec<&str> = scope
        .bound
        .iter()
        .filter(|b| {
            b.name != place
                && b.aliases.is_some_and(&overlaps)
                && scope.moves.moved_site(&b.name).is_none()
        })
        .map(|b| b.name.as_str())
        .collect();
    names.sort_unstable();
    if let Some(name) = names.into_iter().next() {
        return Some(AliasOrigin::Name(name));
    }
    stack
        .iter()
        .filter_map(|slot| slot.alias)
        .find(|alias| overlaps(alias.set))
        .map(|alias| AliasOrigin::Stack(alias.span))
}

/// Where a block's extent ended, for the scope-end linearity diagnostic (R6):
/// a word body or REPL line can only cite a line, while an `if` arm cites the
/// exact terminator token that closed it.
enum BlockEnd {
    Body(u32),
    Arm { token: &'static str, span: Span },
}

/// Error context for the shared stack simulation: a full word (with its
/// declared effect to cite) or a bare REPL line (no signature to cite).
/// Both carry the struct/enum registries `is_copy` needs to resolve a
/// `Type::Struct`/`Type::Enum`'s linearity, so `dup`/`over`/back-edge checking
/// works identically whether the caller is a compiled word or a REPL line.
enum Ctx<'a> {
    Word {
        /// Demangled, so every diagnostic that interpolates it is correct by
        /// default: `resolve` rewrites module 0's decls to `{name}__m{module}`
        /// as soon as a file has an import, and `check` runs on those names.
        /// Self-tail recognition compares against mangled *call* names, so it
        /// reads `mangled` instead.
        name: &'a str,
        mangled: &'a str,
        effect: &'a StackEffect,
        structs: &'a [StructDecl],
        enums: &'a [EnumDecl],
    },
    Line {
        structs: &'a [StructDecl],
        enums: &'a [EnumDecl],
    },
}

/// The `Ctx` for checking `word`'s body: shared by the body walkers and the
/// binding-name rejections so all of them cite the same declared effect.
fn word_ctx<'a>(word: &'a WordDef, structs: &'a [StructDecl], enums: &'a [EnumDecl]) -> Ctx<'a> {
    Ctx::Word {
        name: crate::resolve::demangle_word(&word.name),
        mangled: &word.name,
        effect: &word.effect,
        structs,
        enums,
    }
}

impl Ctx<'_> {
    fn structs(&self) -> &[StructDecl] {
        match self {
            Ctx::Word { structs, .. } | Ctx::Line { structs, .. } => structs,
        }
    }

    fn enums(&self) -> &[EnumDecl] {
        match self {
            Ctx::Word { enums, .. } | Ctx::Line { enums, .. } => enums,
        }
    }

    /// The enclosing word's name, for recognizing a self-tail-call back-edge
    /// (R15). A bare REPL line has no word to recurse into.
    fn word_name(&self) -> Option<&str> {
        match self {
            Ctx::Word { name, .. } => Some(name),
            Ctx::Line { .. } => None,
        }
    }

    fn mangled_name(&self) -> Option<&str> {
        match self {
            Ctx::Word { mangled, .. } => Some(mangled),
            Ctx::Line { .. } => None,
        }
    }

    /// R11: the enclosing word's declared output row, the context a branch join
    /// in tail position materializes its quotation arms against (the merged
    /// slot maps to the output at the same index). A bare REPL line has no
    /// declared row, so a materializing join there stays a located error.
    fn declared_outputs(&self) -> Option<&[TypedSlot]> {
        match self {
            Ctx::Word { effect, .. } => Some(&effect.outputs),
            Ctx::Line { .. } => None,
        }
    }
}

/// R1: recognize every user-defined `drop` overload -- a word literally
/// named `drop` whose declared effect is exactly one struct input and zero
/// outputs -- in its own pre-pass, before `check_types` and before any name
/// registration. Dispatch on a recognized override happens entirely through
/// the returned `StructId -> word index` table, never through a name lookup
/// on the string `"drop"`: `check_shuffle`'s `"drop"` arm (and `lower_call`'s
/// mirror of it) intercepts every `drop` call site before any name
/// resolution reaches `env`, so a word literally named `"drop"` registered
/// into `env` the ordinary way would be dead on arrival (see the Grounding
/// facts in the slice 8b spec).
///
/// This validates only the override's *declared shape*, never
/// `is_copy`/`is_linear` on the input type itself: that fold's own
/// termination argument depends on `check_recursion` having already run,
/// which happens inside `check_types`, after this pre-pass -- calling it
/// early would turn a cyclic struct declaration into a stack overflow
/// instead of a diagnostic.
///
/// A `HashMap<&str, usize>` keyed on the shared literal name `"drop"` (the
/// shape `check_tail_call_cycles`'s own `name_to_idx` uses) would silently
/// keep only the last `drop` word seen and must not be used here: the
/// registry is keyed by `StructId`, so overrides for distinct structs coexist
/// with no collision, and a second override for the *same* struct is instead
/// a located error.
pub fn find_drop_overloads(
    words: &[WordDef],
    structs: &[StructDecl],
) -> Result<HashMap<StructId, usize>, String> {
    let mut registry: HashMap<StructId, usize> = HashMap::new();
    for (idx, word) in words.iter().enumerate() {
        if word.name != "drop" {
            continue;
        }
        let id = drop_overload_struct_id(word)?;
        if registry.contains_key(&id) {
            return Err(duplicate_drop_overload_error(word, &structs[id.index()]));
        }
        registry.insert(id, idx);
    }
    Ok(registry)
}

/// R1: validate a `: drop` word's declared shape and return the struct id it
/// overrides, or a located error citing the word's own declaration --
/// modeled on `check_main_effect`'s shape (find the offending word by name,
/// report its span).
///
/// R11: the REPL calls this directly on its one entered `: drop` line, so a
/// line-at-a-time override gets exactly the declaration-shape rule a compiled
/// program's does; only the duplicate-override rejection differs, since a
/// second REPL `: drop` for one struct is a redefinition, not a collision.
pub fn drop_overload_struct_id(word: &WordDef) -> Result<StructId, String> {
    if !word.effect.outputs.is_empty() {
        return Err(drop_overload_output_error(word));
    }
    if word.effect.inputs.len() != 1 {
        return Err(drop_overload_arity_error(word));
    }
    match word.effect.inputs[0].ty {
        Type::Struct(id, _) => Ok(id),
        found => Err(drop_overload_non_struct_input_error(word, found)),
    }
}

/// R1: a `drop` overload declaring one or more outputs.
fn drop_overload_output_error(word: &WordDef) -> String {
    let span = word_span(word);
    format!(
        "error: `drop` overload (line {}, col {}) must declare zero outputs, found {}",
        span.line,
        span.col,
        effect_str(&word.effect)
    )
}

/// R1: a `drop` overload not declaring exactly one input.
fn drop_overload_arity_error(word: &WordDef) -> String {
    let span = word_span(word);
    format!(
        "error: `drop` overload (line {}, col {}) must declare exactly one input, found {}",
        span.line,
        span.col,
        effect_str(&word.effect)
    )
}

/// R1: a `drop` overload whose one input is not a `type:`-declared struct --
/// an enum, an array, a scalar, or a reference all land here.
fn drop_overload_non_struct_input_error(word: &WordDef, found: Type) -> String {
    let span = word_span(word);
    format!(
        "error: `drop` overload (line {}, col {}) must take a `type:`-declared struct, found `{}`",
        span.line, span.col, found
    )
}

/// R1: a second `drop` overload naming a struct that already has one.
fn duplicate_drop_overload_error(word: &WordDef, target: &StructDecl) -> String {
    let span = word_span(word);
    format!(
        "error: `{}` already defines its own `drop` (line {}, col {})",
        target.name, span.line, span.col
    )
}

/// Takes `&mut Module` because an array word (`fill`) interns its result
/// shape `[T N]` into `module.arrays` during checking (R3, R10): the same
/// registry `ir::lower` then reads, so the checker and the layout builder
/// share one `ArrayId` numbering. `check` runs before `lower`, so the
/// interned shapes are present when codegen consults them.
/// R7a: the type-position audit. A quotation type reaches every type position
/// the parser accepts (R2 routes it through `parse_type_expr`), but this slice
/// gives it a runtime representation at none of them: the one legal position
/// is a **direct input in a word's declared effect** (the quotation parameter
/// this slice adds). Every other position is a located rejection naming the
/// position and the offending type, pointing at slice 7 as the lift. This is
/// what makes R7's `unreachable!` arms sound rather than hopeful (the slice-4
/// audit-sweep shape, now for quotation *types*).
fn audit_quotation_type_positions(module: &Module) -> Result<(), String> {
    audit_quotation_type_registries(
        &module.structs,
        &module.enums,
        &module.arrays,
        &module.owned_cells,
        &module.refs,
    )?;
    for w in &module.words {
        audit_word_quotation_positions(w)?;
    }
    for decl in &module.externs {
        for slot in decl.effect.inputs.iter().chain(&decl.effect.outputs) {
            reject_quotation_type_position(
                slot.ty,
                &format!("an `extern:` boundary type of `{}`", decl.name),
            )?;
        }
    }
    Ok(())
}

/// R7a (REPL, item 2): the registry half of the audit, over exactly the shared
/// type registries. A quotation type never legally enters any of these (its
/// one legal home is a direct word parameter, stored in the word's `Sig`, and
/// a declared effect is interned separately), so re-scanning them per REPL
/// line is a safe, idempotent invariant. Split out so the REPL's `type:` and
/// `:` chokepoints run the same rejections as the native `check`, which the
/// REPL's `check_types`-only path skipped (a quotation in an audited position
/// then reached `ir_type_of`'s `unreachable!`, bricking the session).
pub(crate) fn audit_quotation_type_registries(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
) -> Result<(), String> {
    for s in structs {
        for (fname, fty) in &s.fields {
            // R8 (D4): a quotation type is legal as a struct field this slice
            // (a materialization boundary); the store of a literal into it is
            // checked at the constructor/setter call site (R7). Every other
            // registry position below stays rejected.
            if matches!(fty, Type::Quotation(_)) {
                continue;
            }
            reject_quotation_type_position(
                *fty,
                &format!("the field `{fname}` of struct `{}`", s.name),
            )?;
        }
    }
    for e in enums {
        for v in &e.variants {
            for (fname, fty) in &v.fields {
                reject_quotation_type_position(
                    *fty,
                    &format!(
                        "the field `{fname}` of enum variant `{}::{}`",
                        e.name, v.name
                    ),
                )?;
            }
        }
    }
    for a in arrays {
        // R8 (D4): a quotation is legal as an array element this slice (a
        // materialization boundary, checked at `fill`/`!`); a cell payload and
        // a reference referent below are not D4 boundaries and stay rejected.
        if matches!(a.element, Type::Quotation(_)) {
            continue;
        }
        reject_quotation_type_position(a.element, "an array element")?;
    }
    for c in cells {
        reject_quotation_type_position(c.payload, "an owned-cell payload")?;
    }
    for r in refs {
        reject_quotation_type_position(r.referent, "a reference referent")?;
    }
    Ok(())
}

/// R7a (REPL, item 2): the per-word half of the audit -- a quotation in a
/// word's *output* row, a clause-bodied combinator, `main` taking one, or a
/// quotation nested inside a declared effect. A direct quotation *parameter*
/// (the one legal position) is accepted here and rejected separately at the
/// REPL (R23), which discards word bodies the inliner needs.
pub(crate) fn audit_word_quotation_positions(w: &WordDef) -> Result<(), String> {
    let word = crate::resolve::demangle_word(&w.name);
    for slot in &w.effect.outputs {
        // R8 (D4): a monomorphic word may declare a `Type::Quotation` output (a
        // materialization boundary, checked at the exit row by `check_outputs`).
        // The poly path below still rejects a quotation output: polymorphic
        // quotation *values* are out of scope this slice.
        if matches!(slot.ty, Type::Quotation(_)) {
            continue;
        }
        reject_quotation_type_position(slot.ty, &format!("the output of `{word}`"))?;
    }
    // R18/R7a: a monomorphic word taking a quotation is a combinator,
    // which the inliner supports only with a *term* body (it splices the
    // body against the live stack); a clause body cannot be spliced, so
    // such a word would mint an `IrFunc` with a quotation parameter and
    // reach `ir_type_of`'s `unreachable!` arm (R7). Reject it here, with
    // the type positions, so that arm stays unreached. (A poly word's
    // effect is empty and is checked on the poly path, phase 2.)
    if w.poly.is_none()
        && matches!(w.body, WordBody::Clauses(_))
        && w.effect
            .inputs
            .iter()
            .any(|s| matches!(s.ty, Type::Quotation(_)))
    {
        return Err(clause_bodied_quotation_word_error(word));
    }
    for slot in &w.effect.inputs {
        if let Type::Quotation(eff) = slot.ty {
            // `main` takes no quotation: it is an entry point, not a
            // combinator (D6/R28).
            if w.name == "main" {
                reject_quotation_type_position(slot.ty, "an input of `main`")?;
            }
            // A quotation nested inside a quotation effect (a quotation
            // taking a quotation) is deferred to slice 7, rejected rather
            // than half-supported.
            for t in eff.inputs.iter().chain(&eff.outputs) {
                reject_quotation_type_position(*t, "nested inside a quotation effect")?;
            }
        }
    }
    // A polymorphic word carries its signature in `w.poly`, not `w.effect`
    // (which is empty), so the output-position and nested-in-effect audits
    // above never see it. Run the same rejections over the poly signature,
    // driven by one recursive enumeration (item 2): a quotation may hide in a
    // poly *array element* (`[ [ 'T -- ] 3 ]`), which the earlier shallow
    // audit never descended into.
    if let Some(sig) = &w.poly {
        for pt in &sig.outputs {
            reject_poly_quotation_anywhere(pt, sig, &format!("the output of `{word}`"))?;
        }
        for pt in &sig.inputs {
            audit_poly_input_quotation(pt, sig)?;
        }
    }
    Ok(())
}

/// R7a (poly path, item 2): audit a poly word *input*, where a direct
/// quotation is the one legal position (the combinator's parameter). The
/// parameter itself is accepted, but a quotation buried inside it -- an array
/// element (`[ [ 'T -- ] 3 ]`), or nested in the parameter's own effect rows
/// -- is rejected.
fn audit_poly_input_quotation(pt: &PolyType, sig: &PolySig) -> Result<(), String> {
    match pt {
        PolyType::Quotation(ins, outs) => {
            for t in ins.iter().chain(outs) {
                reject_poly_quotation_anywhere(t, sig, "nested inside a quotation effect")?;
            }
            Ok(())
        }
        PolyType::Array(elem, _) => reject_poly_quotation_anywhere(elem, sig, "an array element"),
        PolyType::Concrete(_) | PolyType::Var(_) => Ok(()),
    }
}

/// R7a (poly path, item 2): reject a quotation type appearing *anywhere*
/// inside `pt` -- as the whole position, as an array element, or nested in a
/// quotation effect -- naming the innermost position. Driving every poly
/// non-parameter position from one recursive enumeration is what keeps R7's
/// default-deny `unreachable!` arms sound: a quotation buried in a poly array
/// element must not slip past the audit and reach `ir_type_of`. A
/// fully-concrete quotation folds to `Concrete(Type::Quotation)`, so route
/// that through the monomorphic rejection to share the rendering.
fn reject_poly_quotation_anywhere(
    pt: &PolyType,
    sig: &PolySig,
    position: &str,
) -> Result<(), String> {
    match pt {
        PolyType::Concrete(ty) => reject_quotation_type_position(*ty, position),
        PolyType::Var(_) => Ok(()),
        PolyType::Array(elem, _) => reject_poly_quotation_anywhere(elem, sig, "an array element"),
        PolyType::Quotation(..) => Err(format!(
            "error: a quotation type `{}` cannot appear as {position}: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
            poly_type_str(pt, sig),
        )),
    }
}

/// R18/R7a: a monomorphic quotation-taking word with a clause body cannot be
/// inlined (a clause body is not a splice-able term list), so it is rejected
/// rather than left to panic at lowering. Slice 7's runtime quotation value
/// lifts it (the word would then `call` a real value, no inlining needed).
fn clause_bodied_quotation_word_error(word: &str) -> String {
    format!(
        "error: the quotation-taking word `{word}` has a clause body; a quotation parameter is only supported on a word with a term body this slice (its body is inlined at each call site, and a clause body cannot be spliced), and a runtime quotation value is slice 7",
    )
}

/// R7a: reject `ty` if it is a quotation type, naming the position and slice 7.
fn reject_quotation_type_position(ty: Type, position: &str) -> Result<(), String> {
    if let Type::Quotation(eff) = ty {
        return Err(format!(
            "error: a quotation type `{}` cannot appear as {position}: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
            eff.name_static,
        ));
    }
    Ok(())
}

pub fn check(module: &mut Module) -> Result<(), String> {
    // R1: recognized ahead of `check_types` so the ordering hazard against
    // `check_recursion` (run inside `check_types`) never arises.
    let drop_overloads = find_drop_overloads(&module.words, &module.structs)?;
    let drop_overload_indices: HashSet<usize> = drop_overloads.values().copied().collect();
    // R3: defining `drop` for a struct forces it linear, so the fact is
    // recorded on the declaration itself rather than re-derived: every
    // `is_copy` call site, `ir`'s layout fold, and the REPL's persistent
    // registries all read the same `StructDecl`.
    for id in drop_overloads.keys() {
        module.structs[id.index()].has_drop_overload = true;
    }

    check_types(
        &module.structs,
        &module.enums,
        &module.arrays,
        &module.owned_cells,
    )?;

    // R7a: a quotation type is legal only as a direct word parameter this
    // slice; reject it at every other position before layout or lowering can
    // see it, so R7's `unreachable!` mangling/`IrType` arms stay unreached.
    audit_quotation_type_positions(module)?;

    let mut env = builtin_table();
    for (name, sig) in struct_generated_sigs(&module.structs) {
        env.insert(name, sig);
    }
    for (name, sig) in enum_generated_sigs(&module.enums) {
        env.insert(name, sig);
    }

    // R1: an `extern:` declaration is registered into the same word
    // environment as any other word, so every existing arity/type check
    // applies to its call sites unchanged; but first, R1's redeclaration
    // rule and R2/R3's boundary-type rules are checked at the declaration.
    check_extern_decls(
        &module.externs,
        &module.words,
        &env,
        &module.structs,
        &module.enums,
        &module.arrays,
    )?;
    for decl in &module.externs {
        env.insert(decl.name.clone(), sig_of(&decl.effect));
    }

    // A duplicate word name in one module is rejected here, before the
    // population loop below would otherwise silently keep only the last one
    // seen and let both bodies reach codegen.
    check_duplicate_word_names(&module.words)?;

    // R1: a recognized `drop` overload is excluded from the ordinary word
    // environment -- registering it under the literal name `"drop"` would be
    // either dead (`check_shuffle`'s `"drop"` arm intercepts every call site
    // first) or, for a second overload, a name collision the checker has no
    // reason to reject, since dispatch never goes through this table.
    //
    // R5: a polymorphic word never enters the concrete `env` (its inputs are
    // not concrete `Sig` types); it lives in `poly_env` instead, and a call
    // site is intercepted there before the concrete lookup, where its
    // `PolySig` is unified against the concrete stack.
    let mut poly_env: HashMap<String, (PolySig, Option<u64>)> = HashMap::new();
    for (idx, word) in module.words.iter().enumerate() {
        if drop_overload_indices.contains(&idx) {
            continue;
        }
        if let Some(sig) = &word.poly {
            poly_env.insert(word.name.clone(), ((**sig).clone(), None));
        } else {
            env.insert(word.name.clone(), sig_of(&word.effect));
        }
    }

    check_main_effect(
        &module.words,
        &module.structs,
        &module.enums,
        &module.arrays,
    )?;

    // Split the borrow so a word body can intern into `arrays`/`owned_cells`
    // while reading `words`/`enums`/`structs`.
    let Module {
        words,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        externs: _,
        instantiations: _,
        modules: _,
    } = module;
    // R6: each body's own `drop` call sites, resolved to a concrete operand
    // type by the walk that checks it. Collected per word so the graph below
    // knows which body each site sits in.
    let mut dropped: Vec<Vec<Type>> = Vec::with_capacity(words.len());
    // R18: the monomorphic quotation-taking words, gathered once so a call to
    // one is intercepted and inlined (term-splice) rather than lowered to a
    // call. A polymorphic combinator's body is checked by the poly pass, so it
    // is not registered here; only a `WordBody::Terms` monomorphic word with a
    // `Type::Quotation` input qualifies.
    let combinators = collect_combinators(words);
    // R22 (D5): reject a cycle in the quotation-taking-word call subgraph
    // before any body is checked, so the splice below may assume acyclicity.
    // Ordered *before* `check_tail_call_cycles`: a combinator's call to
    // another combinator is inlined (spliced), never lowered as a tail call,
    // so a combinator cycle is a splice-forever error (this pass), not mutual
    // tail recursion -- running the tail-cycle pass first would misreport a
    // combinator cycle whose edges happen to sit in tail position.
    check_combinator_cycles(&combinators)?;
    // Reject mutual tail-recursion cycles (D3, X1) on the whole-module
    // tail-call graph, after signature registration and before body checking.
    check_tail_call_cycles(words, &drop_overload_indices)?;
    // R14: the per-call-site instantiation table, filled as each monomorphic
    // body's calls to polymorphic words are unified, then stored on the module
    // for lowering.
    let mut insts: HashMap<Span, CallInst> = HashMap::new();
    for word in words.iter() {
        let mut sites = Vec::new();
        if let Some(sig) = &word.poly {
            if is_combinator(word) {
                // R14-R17: a polymorphic combinator (`each`/`map`/`fold`) is
                // checked standalone by instantiating its signature at
                // concrete stand-in types and running the ordinary checker on
                // the body, which already handles the abstract quotation
                // `call`/`times` (R8/R9) and the three `times` obligations
                // (R16). It mints no `IrFunc` (R20): a call to it is inlined
                // by term-splice at its concrete call sites, so the
                // instantiation records it produces here are scratch.
                let mut scratch: HashMap<Span, CallInst> = HashMap::new();
                let mut poly = PolyCtx {
                    env: &poly_env,
                    insts: &mut scratch,
                    combinators: &combinators,
                };
                check_poly_combinator_standalone(
                    word,
                    sig,
                    enums,
                    &env,
                    arrays,
                    owned_cells,
                    refs,
                    structs,
                    &mut poly,
                )?;
            } else {
                // R7: a polymorphic body is checked over a `PolyType` stack by
                // a dedicated pass, deliberately separate from the concrete
                // walk.
                check_poly_body(word, sig, &env, structs, enums, arrays)?;
            }
        } else {
            let mut poly = PolyCtx {
                env: &poly_env,
                insts: &mut insts,
                combinators: &combinators,
            };
            check_word(
                word,
                enums,
                &env,
                arrays,
                owned_cells,
                refs,
                structs,
                &mut sites,
                &mut poly,
            )?;
        }
        dropped.push(sites);
    }

    // R6: only now, with every `drop` call site's operand type known, can the
    // `drop`-reachability graph be built.
    let word_refs: Vec<&WordDef> = words.iter().collect();
    check_drop_overload_recursion(
        &word_refs,
        structs,
        enums,
        arrays,
        owned_cells,
        &drop_overloads,
        &dropped,
    )?;

    // R8/R10: the multi-output return bundles, interned into the same
    // `module.structs` the layout pass reads, so a bundle is laid out (and
    // flagged, so no destructor is synthesized for it) like any other struct.
    // Last, after every type-level check and after `struct_generated_sigs`:
    // a bundle is an ABI detail, not a nameable type, so it takes part in
    // neither name resolution nor generated-word registration.
    intern_output_bundles(module);
    // R8/R14: each polymorphic instantiation whose resolved output count is
    // >= 2 needs the same kind of bundle a monomorphic multi-output word gets,
    // interned into the same `module.structs` (reusing the checker's earlier
    // struct interning) so lowering reads it back like any other struct. The
    // table itself is then handed to lowering on the module.
    for inst in insts.values_mut() {
        if inst.out_arity >= 2 {
            inst.bundle = Some(intern_bundle_struct(
                &mut module.structs,
                &inst.output_types,
            ));
        }
    }
    module.instantiations = insts;
    Ok(())
}

/// R10: one interned bundle struct per distinct output tuple of length >= 2,
/// over every declared word. Gated on the output count alone, not on anything
/// about the word: a `drop` overload has no outputs and an `extern:` is
/// rejected above one, so neither reaches this.
fn intern_output_bundles(module: &mut Module) {
    let tuples: Vec<Vec<Type>> = module
        .words
        .iter()
        .filter(|w| w.effect.outputs.len() >= 2)
        .map(|w| w.effect.outputs.iter().map(|s| s.ty).collect())
        .collect();
    for outputs in tuples {
        intern_bundle_struct(&mut module.structs, &outputs);
    }
}

/// R1/R2/R3/R7/R12/R13/R14: every `extern:` declaration's own checks, run
/// before its signature enters the word environment. R1's redeclaration
/// check runs against the name-dispatched builtins (`BUILTIN_WORDS`),
/// `existing` (`builtin_table`'s seed, empty today, plus the
/// struct/enum-generated words), the user's own `:` words, and every other
/// `extern:` (in that order, first match wins); R2/R3 reject each forbidden
/// boundary type at the declaration rather than at a call site; the
/// output-reference rejection reuses `check_reference_free_signature`'s
/// existing message rather than duplicating it (R3).
///
/// R14: the symbol's *shape* is checked in the parser, but nothing checks
/// that it *exists* — that needs a symbol table the compiler has no access
/// to, so a misspelled symbol is a `cc` linker error, not a diagnostic.
fn check_extern_decls(
    externs: &[ExternDecl],
    words: &[WordDef],
    existing: &HashMap<String, Sig>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    let mut seen: HashSet<&str> = HashSet::new();
    for decl in externs {
        if is_builtin_word_name(decl.name.as_str()) {
            return Err(extern_redeclaration_error(decl));
        }
        if existing.contains_key(decl.name.as_str()) {
            return Err(extern_redeclaration_error(decl));
        }
        if words.iter().any(|w| w.name == decl.name) {
            return Err(extern_redeclaration_error(decl));
        }
        if !seen.insert(decl.name.as_str()) {
            return Err(extern_redeclaration_error(decl));
        }
        if decl.effect.outputs.len() > 1 {
            return Err(extern_multi_output_error(decl));
        }
        check_reference_free_signature(&decl.name, &decl.effect, structs, enums, arrays)?;
        check_extern_boundary_types(decl)?;
    }
    Ok(())
}

/// R1: the builtin words `check_term` dispatches by name, in its probe chain,
/// *before* the word environment is consulted at all. They are absent from
/// `builtin_table` (empty today, since every builtin dispatches on the
/// concrete operand type rather than a fixed signature), so an `extern:`
/// naming one would be registered, never looked up, and silently do nothing. The `^`-led owning-cell words and the `@`/`!`/`+!` access
/// words are dispatched in the same chain but are rejected earlier, against
/// the declaration's name in the parser, so they are not repeated here.
const BUILTIN_WORDS: &[&str] = &[
    // check_shuffle
    "dup",
    "drop",
    "swap",
    "over",
    "rot", // check_operator
    "+",
    "-",
    "*",
    "/",
    "mod",
    "and",
    "or",
    "xor",
    "not",
    "shl",
    "shr",
    "=",
    "<",
    ">",
    "<=",
    ">=",
    "<>",
    ".",
    "max",
    "max-total", // check_str_word
    "len",
    "cstr", // check_array_word (`len` is shared with `check_str_word`)
    "fill",
];

/// R1: whether `name` is dispatched as a builtin ahead of any environment
/// lookup. Beyond the fixed names, `check_operator` claims every `>`-prefixed
/// name with a non-empty remainder as a numeric conversion (`>u8`), erroring
/// on an unrecognised target type rather than falling through, so no such
/// name can reach a registered signature either. Bare `>` is the comparison
/// operator, and is in the list.
fn is_builtin_word_name(name: &str) -> bool {
    BUILTIN_WORDS.contains(&name) || name.strip_prefix('>').is_some_and(|rest| !rest.is_empty())
}

/// R1: a located error for an `extern:` declaration redeclaring a name
/// already registered as a builtin, a user `:` word, or another `extern:`.
fn extern_redeclaration_error(decl: &ExternDecl) -> String {
    format!(
        "error: `extern: {}` redeclares an existing word (line {}, col {})",
        decl.name, decl.span.line, decl.span.col
    )
}

/// R8 (slice 8b): no C function returns two values, so a declared output
/// arity above one describes no callable prototype. Left unrejected it lowers
/// to a discarded result (`lower_call` binds a return only for `out_arity ==
/// 1`) and panics in the *next* consumer of the value that was never pushed,
/// which points at the wrong term entirely.
fn extern_multi_output_error(decl: &ExternDecl) -> String {
    format!(
        "error: `extern: {}` declares {} outputs (line {}, col {})\n  no C function returns more than one value; declare at most one output",
        decl.name,
        decl.effect.outputs.len(),
        decl.span.line,
        decl.span.col
    )
}

/// R2: the boundary type set an `extern:` slot may use in either position —
/// the numeric tower, `bool`, `&T`/`&!T`, and `cstr`. Each is either a scalar
/// or an opaque `Ptr` the backend already passes across a call.
///
/// `str` is excluded despite R2's list naming it, on R2's own criterion: R4
/// makes it a descriptor handle, not a scalar or a single opaque `Ptr`, so C
/// would receive a pointer to a descriptor rather than a `char*`. See
/// `extern_str_input_error`/`extern_str_output_error` for each direction.
fn is_extern_boundary_scalar(ty: Type) -> bool {
    matches!(
        ty,
        Type::Int(_)
            | Type::Float(_)
            | Type::Bool
            | Type::Usize
            | Type::Isize
            | Type::Ref(..)
            | Type::Cstr
    )
}

/// R3: each `extern:` boundary-type rejection not already covered by
/// `check_reference_free_signature` (which independently rejects any
/// reference-containing output before this ever runs). An owned aggregate
/// (struct/enum/array/`^T`) is rejected in either position: ownership across
/// the FFI boundary has no answer and no client. A `^T` specifically in
/// output position gets its own message, since forging ownership of memory
/// the allocator did not hand out is a sharper reason than the generic one.
fn check_extern_boundary_types(decl: &ExternDecl) -> Result<(), String> {
    for slot in &decl.effect.inputs {
        if matches!(slot.ty, Type::Str) {
            return Err(extern_str_input_error(decl));
        }
        if !is_extern_boundary_scalar(slot.ty) {
            return Err(extern_owned_aggregate_error(decl, slot.ty, "input"));
        }
    }
    for slot in &decl.effect.outputs {
        if is_extern_boundary_scalar(slot.ty) {
            continue;
        }
        if matches!(slot.ty, Type::Str) {
            return Err(extern_str_output_error(decl));
        }
        if matches!(slot.ty, Type::OwnedCell(..)) {
            return Err(extern_owned_pointer_output_error(decl, slot.ty));
        }
        return Err(extern_owned_aggregate_error(decl, slot.ty, "output"));
    }
    Ok(())
}

/// R2/R7: a `str` input has no C prototype (R4 makes it a descriptor handle,
/// not a scalar or a single opaque `Ptr`, so C would receive a pointer to a
/// descriptor rather than a `char*`), and the conversion that gives it one is
/// total — `cstr` is sound for every `str` under R11's static-rooting, a
/// literal being the only constructor — so the rejection names it.
fn extern_str_input_error(decl: &ExternDecl) -> String {
    format!(
        "error: `extern: {}` declares the input `str` (line {}, col {})\n  a `str` is a pointer and a length, which matches no C parameter; declare `cstr` and convert with `cstr` at the call site",
        decl.name, decl.span.line, decl.span.col
    )
}

/// R11: a returned `str` would be a `str` not built from a literal, which is
/// the invariant R10's `Copy`/non-escaping status rests on. C supplies no
/// length either, so there is nothing to build one from.
fn extern_str_output_error(decl: &ExternDecl) -> String {
    format!(
        "error: `extern: {}` cannot return a `str` (line {}, col {})\n  a `str` may point at static data only, and C supplies no length; declare `cstr`",
        decl.name, decl.span.line, decl.span.col
    )
}

fn extern_owned_aggregate_error(decl: &ExternDecl, ty: Type, position: &str) -> String {
    format!(
        "error: `extern: {}` declares the {position} `{}`, an owned aggregate (line {}, col {})\n  ownership across the C boundary has no answer and no client; only the numeric tower, `&T`/`&!T`, and `cstr` may cross",
        decl.name, ty, decl.span.line, decl.span.col
    )
}

fn extern_owned_pointer_output_error(decl: &ExternDecl, ty: Type) -> String {
    format!(
        "error: `extern: {}` cannot return the owned pointer `{}` (line {}, col {})\n  it would forge ownership of memory the allocator did not hand out",
        decl.name, ty, decl.span.line, decl.span.col
    )
}

/// R18 (phase 4 slice 5a phase 3): an exported word whose stack effect names
/// a non-primitive type of its own module that is not itself exported is a
/// declaration-site error naming the word and the private type. R15 makes a
/// type and its generated words one exported unit, so exporting the type
/// clears every word of its own module that mentions it. Runs on the raw,
/// pre-mangle module the driver assembles (`driver::assemble_module`),
/// before `resolve::resolve_modules` renames decls: the check matches a
/// word's raw name against its own module's raw `export:` list, and both
/// would already be mangled by the time `check::check` runs.
pub fn check_exported_signatures(module: &Module) -> Result<(), String> {
    for word in &module.words {
        let exports = match module.modules.get(word.module as usize) {
            Some(m) => &m.exports,
            None => continue,
        };
        if !exports.iter().any(|(n, _)| n == &word.name) {
            continue;
        }
        for ty in effect_types(word) {
            if let Some(name) = private_type_name(ty, word.module, module) {
                return Err(exported_word_names_private_type_error(word, name));
            }
        }
    }
    Ok(())
}

/// Every concrete `Type` a word's declared effect mentions: its ordinary
/// input/output slots, plus, for a polymorphic word, every `Concrete` leaf
/// its `PolySig` mentions (a type variable itself names no type, so `Var`
/// contributes nothing).
fn effect_types(word: &WordDef) -> Vec<Type> {
    let mut out: Vec<Type> = word
        .effect
        .inputs
        .iter()
        .chain(&word.effect.outputs)
        .map(|slot| slot.ty)
        .collect();
    if let Some(sig) = &word.poly {
        for t in sig.inputs.iter().chain(&sig.outputs) {
            collect_poly_concrete(t, &mut out);
        }
    }
    out
}

fn collect_poly_concrete(t: &PolyType, out: &mut Vec<Type>) {
    match t {
        PolyType::Concrete(ty) => out.push(*ty),
        PolyType::Var(_) => {}
        PolyType::Array(elem, _) => collect_poly_concrete(elem, out),
        // Slice 6a (R5): a declared quotation effect's rows may name concrete
        // types (`[ i64 -- ]`); collect them so export-privacy still sees a
        // private type mentioned inside an effect.
        PolyType::Quotation(ins, outs) => {
            for t in ins.iter().chain(outs) {
                collect_poly_concrete(t, out);
            }
        }
    }
}

/// Whether `ty` is a struct/enum owned by `owner_module` and absent from that
/// module's `export:` list, i.e. the R18 violation. A type owned by a
/// *different* module is not this rule's problem (R16 already gates whether
/// it could even be named here), and a primitive/array/etc. names no
/// declared type at all.
fn private_type_name(ty: Type, owner_module: u32, module: &Module) -> Option<&'static str> {
    let (decl_module, name) = match ty {
        Type::Struct(id, name) => (module.structs[id.index()].module, name),
        Type::Enum(id, name) => (module.enums[id.index()].module, name),
        _ => return None,
    };
    if decl_module != owner_module {
        return None;
    }
    let exports = &module.modules[decl_module as usize].exports;
    if exports.iter().any(|(n, _)| n == name) {
        return None;
    }
    Some(name)
}

/// R18: a located error naming the exported word and the private type its
/// effect mentions. Exporting the type satisfies the rule.
fn exported_word_names_private_type_error(word: &WordDef, type_name: &str) -> String {
    let span = word_span(word);
    format!(
        "error: exported word `{}` (line {}, col {}) names private type `{}`, which is not exported\n  export `{}` too, or remove it from the effect",
        word.name, span.line, span.col, type_name, type_name
    )
}

/// Phase 4 slice 5a phase 4 (R20/R15c): one selectively-imported name, carried
/// from the driver's closure assembly with the qualifier and target module it
/// came from and the span of the name in the `import:` form, for the R20/R21
/// validation. A type name exposes its generated words as one unit (R15c), so
/// only the base name appears here; a member (`Type>field`) can only collide
/// when its base does.
pub struct SelectiveName {
    pub name: String,
    pub qualifier: String,
    pub target: u32,
    pub span: Span,
}

/// R20/R21: validate every module's selective imports on the raw, pre-mangle
/// module. Each listed name must be exported by its source module (R20, the
/// R16 visibility error). No two selective imports may expose the same
/// unqualified name, and a selective name may not collide with one of the
/// importing module's own words or types (R21, a located error at the second
/// source naming both). The collision is decided on the base name because a
/// selectively imported type and its generated words are one unit (R15c) and a
/// member name collides only when its base does.
pub fn check_selective_imports(
    module: &Module,
    selective_by_module: &[Vec<SelectiveName>],
) -> Result<(), String> {
    for (m, entries) in selective_by_module.iter().enumerate() {
        let locals = local_decl_names(module, m as u32);
        // name -> the qualifier that first exposed it, for R21's both-sources error.
        let mut seen: HashMap<&str, &str> = HashMap::new();
        for entry in entries {
            let exports = &module.modules[entry.target as usize].exports;
            if !exports.iter().any(|(n, _)| n == &entry.name) {
                return Err(selective_not_exported_error(
                    &entry.name,
                    &entry.qualifier,
                    entry.span,
                ));
            }
            if locals.contains(entry.name.as_str()) {
                return Err(selective_collides_with_local_error(
                    &entry.name,
                    &entry.qualifier,
                    entry.span,
                ));
            }
            if let Some(first) = seen.insert(entry.name.as_str(), entry.qualifier.as_str()) {
                return Err(selective_collision_error(
                    &entry.name,
                    first,
                    &entry.qualifier,
                    entry.span,
                ));
            }
        }
    }
    Ok(())
}

/// Every raw decl name owned by module `m`: its structs, enums, words, and
/// externs, for R21's selective-vs-local collision check. Runs pre-mangle, so
/// the names are the source spellings a selective import would collide with.
fn local_decl_names(module: &Module, m: u32) -> HashSet<&str> {
    let mut names = HashSet::new();
    for s in &module.structs {
        if s.module == m {
            names.insert(s.name.as_str());
        }
    }
    for e in &module.enums {
        if e.module == m {
            names.insert(e.name.as_str());
        }
    }
    for w in &module.words {
        if w.module == m {
            names.insert(w.name.as_str());
        }
    }
    for x in &module.externs {
        if x.module == m {
            names.insert(x.name.as_str());
        }
    }
    names
}

/// R20: a selectively imported name absent from its source module's `export:`
/// list is the R16 visibility error, same wording as a qualified private
/// reference.
pub(crate) fn selective_not_exported_error(name: &str, qualifier: &str, span: Span) -> String {
    format!(
        "error: `{name}` is not exported from module `{qualifier}` at line {}, col {}",
        span.line, span.col
    )
}

/// R21: a second selective import exposing a name a prior one already exposed,
/// naming both source modules. No precedence, no shadowing: the collision is
/// the error.
fn selective_collision_error(name: &str, first: &str, second: &str, span: Span) -> String {
    format!(
        "error: selective import of `{name}` from module `{second}` (line {}, col {}) collides with the selective import of `{name}` from module `{first}`",
        span.line, span.col
    )
}

/// R21: a selective import exposing a name the importing module already defines
/// locally, naming the source module and the local definition.
fn selective_collides_with_local_error(name: &str, qualifier: &str, span: Span) -> String {
    format!(
        "error: selective import of `{name}` from module `{qualifier}` (line {}, col {}) collides with a local definition of `{name}`",
        span.line, span.col
    )
}

/// Type-level checks that must pass before any generated-word signature or
/// word body is type-checked: no two `type:` declarations share a name across
/// the combined struct+enum registries, and no struct or enum contains itself
/// by value, directly or transitively, through the combined type graph (D9,
/// D10, R8, R10).
pub fn check_types(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
) -> Result<(), String> {
    check_duplicate_type_names(structs, enums)?;
    check_recursion(structs, enums, arrays)?;
    check_no_stored_references(structs, enums, arrays, cells)?;
    check_no_linear_array_elements(structs, enums, arrays)?;
    Ok(())
}

/// The declaration-site half of the no-stored-reference rule: a struct field,
/// an enum variant payload field,
/// an interned array element, or an interned cell payload whose type
/// transitively contains a reference is a located error. Runs after
/// `check_recursion`, so the field-graph walk `contains_reference` performs is
/// guaranteed acyclic. The two *construction* sites (`fill`'s element, `^`'s
/// payload) are rejected separately in the body walk: both accept whatever
/// type is on the stack with no declaration in sight.
fn check_no_stored_references(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
) -> Result<(), String> {
    for decl in structs {
        for (field, ty) in &decl.fields {
            if contains_reference(*ty, structs, enums, arrays) {
                return Err(stored_reference_error(
                    &format!("field `{field}` of type `{}`", decl.name),
                    *ty,
                    Some(decl.span),
                ));
            }
        }
    }
    for decl in enums {
        for variant in &decl.variants {
            for (field, ty) in &variant.fields {
                if contains_reference(*ty, structs, enums, arrays) {
                    return Err(stored_reference_error(
                        &format!(
                            "payload field `{field}` of variant `{}` of type `{}`",
                            variant.name, decl.name
                        ),
                        *ty,
                        Some(variant.span),
                    ));
                }
            }
        }
    }
    for decl in arrays {
        if contains_reference(decl.element, structs, enums, arrays) {
            return Err(stored_reference_error(
                &format!("element of array type `{}`", decl.name_static),
                decl.element,
                None,
            ));
        }
    }
    for decl in cells {
        if contains_reference(decl.payload, structs, enums, arrays) {
            return Err(stored_reference_error(
                &format!("payload of cell type `{}`", decl.name_static),
                decl.payload,
                None,
            ));
        }
    }
    Ok(())
}

/// The one wording every escape rejection shares. `position` names the
/// storage slot the reference tried to reach; an array or cell shape has no
/// declared name and so no span to cite.
fn stored_reference_error(position: &str, ty: Type, span: Option<Span>) -> String {
    let located = match span {
        Some(span) => format!(" (line {}, col {})", span.line, span.col),
        None => String::new(),
    };
    format!(
        "error: a reference cannot be stored: {position} has type `{ty}`{located}\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow"
    )
}

/// Arrays of linear elements are not supported yet: rejected here, over the
/// module's interned array registry, rather than in the parser, because
/// linearity (`is_copy`) is only answerable once every struct/enum field list
/// is resolved, which happens after the whole module is parsed. Every array
/// type named anywhere (a word signature slot, a struct field, an enum
/// variant field) is interned into this one registry, and `is_copy` already
/// walks an array's element transitively, so this single sweep catches a
/// direct `[LinearStruct N]` and an indirect one alike. Runs after
/// `check_recursion`, which rules out a self-referential struct/enum/array
/// first, so `is_copy`'s recursion over the field graph is guaranteed to
/// terminate. `ArrayDecl` carries no span (an array shape has no declared
/// name a pre-pass could register), so the error names the array/element
/// types rather than inventing a wrong line number.
fn check_no_linear_array_elements(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    for decl in arrays {
        if !is_copy(decl.element, structs, enums, arrays) {
            return Err(format!(
                "error: linear array elements are not supported yet: array type `{}` has element `{}`, which is linear and has no `Copy` instance",
                decl.name_static,
                decl.element.name(),
            ));
        }
    }
    Ok(())
}

/// The struct-only projection of `check_types` (no enums/arrays), for callers
/// that don't yet declare either.
pub fn check_structs(structs: &[StructDecl]) -> Result<(), String> {
    check_types(structs, &[], &[], &[])
}

/// A duplicate `type:` name is a sharp located error naming the type. R12
/// (phase 4 slice 5a): the check is per-module, so two modules each declaring
/// `Point` is not a duplicate. Keyed by `(module, name_static)`: `name_static`
/// stays the raw surface name even after the resolver mangles `name` for
/// symbol disambiguation, so the error still reads `Point`, not `Point__m1`.
fn check_duplicate_struct_names(structs: &[StructDecl]) -> Result<(), String> {
    let mut seen: HashMap<(u32, &str), ()> = HashMap::new();
    for decl in structs {
        if seen.insert((decl.module, decl.name_static), ()).is_some() {
            return Err(format!(
                "error: duplicate type `{}` (line {}, col {})",
                decl.name_static, decl.span.line, decl.span.col
            ));
        }
    }
    Ok(())
}

/// A duplicate type name across the *combined* struct + enum registries
/// (D10, X2) is a sharp located error naming the type: a name used by two
/// structs, two enums, or one of each. Delegates the struct-only pass to
/// `check_duplicate_struct_names` (also called directly by struct-only
/// callers, e.g. the REPL, which doesn't yet declare enums) rather than
/// re-scanning `structs` twice.
fn check_duplicate_type_names(structs: &[StructDecl], enums: &[EnumDecl]) -> Result<(), String> {
    check_duplicate_struct_names(structs)?;
    let mut seen: HashMap<(u32, &str), ()> = structs
        .iter()
        .map(|decl| ((decl.module, decl.name_static), ()))
        .collect();
    for decl in enums {
        if seen.insert((decl.module, decl.name_static), ()).is_some() {
            return Err(format!(
                "error: duplicate type `{}` (line {}, col {})",
                decl.name_static, decl.span.line, decl.span.col
            ));
        }
    }
    Ok(())
}

/// A duplicate word name within one module leaks past every existing check
/// straight to the linker's bare `symbol already defined` error: nothing
/// before this rejects a repeat `word.name` the way `check_duplicate_type_names`
/// already does for structs/enums, so the word-environment population loop in
/// `check` silently keeps only the last one seen and both bodies still lower
/// to codegen. Keyed by `(module, name)`, mirroring that check exactly, so two
/// modules each declaring `push` is not a duplicate (`resolve::mangle` already
/// disambiguates that pair's symbols; by the time this runs post-`resolve`,
/// their `name`s already differ) while two `push`es in one module still
/// mangle identically and collide here.
///
/// `drop`-named words are skipped entirely, not treated as exempt from the
/// rule: `find_drop_overloads` (run earlier, unconditionally, as the first
/// step of `check`) already owns every `drop` word's multiplicity, keyed by
/// the struct id it overrides rather than by the shared literal name `"drop"`
/// -- two overloads for two distinct structs are not a duplicate and must
/// coexist, while a second overload for the *same* struct already failed
/// there, before this ever runs. Re-checking `drop` here by name alone would
/// reject that legitimate multi-type overloading (Phase 3 slice 8b) as a
/// false positive. `main` gets no such carve-out: nothing else validates a
/// repeat `main` within one module, so it is an ordinary word for this check.
fn check_duplicate_word_names(words: &[WordDef]) -> Result<(), String> {
    let mut seen: HashMap<(u32, &str), Span> = HashMap::new();
    for word in words {
        if word.name == "drop" {
            continue;
        }
        let span = word_span(word);
        if let Some(first) = seen.insert((word.module, word.name.as_str()), span) {
            return Err(format!(
                "error: duplicate word `{}` (line {}, col {}); first defined at line {}, col {}",
                crate::resolve::demangle_word(&word.name),
                span.line,
                span.col,
                first.line,
                first.col
            ));
        }
    }
    Ok(())
}

/// Whether a struct's field-type graph node has been visited by
/// `check_struct_recursion`'s DFS: `InProgress` marks an ancestor on the
/// current path (finding one again is a cycle), `Done` marks a node already
/// proven acyclic. Every node is visited at most once each way, so the DFS
/// always terminates: it never loops on a self- or mutually-recursive
/// `type:`.
#[derive(Clone, Copy, PartialEq)]
enum VisitState {
    Unvisited,
    InProgress,
    Done,
}

/// A node in the combined struct+enum value-containment graph (D9, R10): a
/// struct or an enum, by registry index.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeNode {
    Struct(usize),
    Enum(usize),
    Array(usize),
}

/// Detect a struct or enum that contains itself by value, directly or
/// transitively, via cycle detection over the *combined* type graph (D9): a
/// struct's field types and an enum's variant field types are edges, so a
/// struct-of-enum-of-struct cycle is caught the same as a pure-struct one.
fn check_recursion(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    let mut st = RecursionState {
        sstate: vec![VisitState::Unvisited; structs.len()],
        estate: vec![VisitState::Unvisited; enums.len()],
        astate: vec![VisitState::Unvisited; arrays.len()],
        path: Vec::new(),
    };
    for start in 0..structs.len() {
        if st.sstate[start] == VisitState::Unvisited {
            visit_recursion(TypeNode::Struct(start), structs, enums, arrays, &mut st)?;
        }
    }
    for start in 0..enums.len() {
        if st.estate[start] == VisitState::Unvisited {
            visit_recursion(TypeNode::Enum(start), structs, enums, arrays, &mut st)?;
        }
    }
    for start in 0..arrays.len() {
        if st.astate[start] == VisitState::Unvisited {
            visit_recursion(TypeNode::Array(start), structs, enums, arrays, &mut st)?;
        }
    }
    Ok(())
}

/// The per-node visit state + current DFS path, bundled so the traversal
/// signature stays readable now that three registries (struct/enum/array)
/// contribute nodes.
struct RecursionState {
    sstate: Vec<VisitState>,
    estate: Vec<VisitState>,
    astate: Vec<VisitState>,
    path: Vec<TypeNode>,
}

/// The frontend `Type` of a field, mapped to a graph node (a scalar has no
/// edge). By-value containment is the only edge kind this graph models: a
/// struct field, enum variant field or array element of type `T` makes `T`
/// part of the enclosing type's size, so a cycle through any of them is
/// infinite size. `OwnedCell` is excluded **deliberately, not by
/// fall-through**: a `^T` field is a heap pointer, not an inline copy of
/// `T`, so it can close a cycle without making the type infinite, and the
/// recursion rule is exactly "every cycle passes through at least one `^`".
fn type_node(ty: &Type) -> Option<TypeNode> {
    match ty {
        Type::Struct(id, _) => Some(TypeNode::Struct(id.index())),
        Type::Enum(id, _) => Some(TypeNode::Enum(id.index())),
        Type::Array(id, _) => Some(TypeNode::Array(id.index())),
        Type::OwnedCell(_, _) => None,
        // A reference is a pointer, not an inline copy, so it closes no
        // by-value cycle — and the no-stored-reference rule keeps one out of
        // every field position
        // anyway.
        Type::Ref(..) => None,
        Type::Int(_)
        | Type::Float(_)
        | Type::Bool
        | Type::Usize
        | Type::Isize
        | Type::Str
        | Type::Cstr
        // Slice 6a: a quotation type has no runtime layout (D6), so it is not
        // a value-containment node; like a reference it closes no size cycle.
        | Type::Quotation(_) => None,
    }
}

/// The value-containment edges out of a node: a struct's field types, or every
/// variant field type of an enum.
fn node_edges(
    node: TypeNode,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Vec<TypeNode> {
    match node {
        TypeNode::Struct(i) => structs[i]
            .fields
            .iter()
            .filter_map(|(_, ty)| type_node(ty))
            .collect(),
        TypeNode::Enum(i) => enums[i]
            .variants
            .iter()
            .flat_map(|v| v.fields.iter())
            .filter_map(|(_, ty)| type_node(ty))
            .collect(),
        // An array's single containment edge is to its element type (M3): a
        // `[T N]` contains a `T` by value, so a cycle through an array element
        // is caught exactly as a struct/enum one, and a nested array bottoms
        // out at a scalar so the DFS terminates.
        TypeNode::Array(i) => type_node(&arrays[i].element).into_iter().collect(),
    }
}

fn node_state(node: TypeNode, st: &mut RecursionState) -> &mut VisitState {
    match node {
        TypeNode::Struct(i) => &mut st.sstate[i],
        TypeNode::Enum(i) => &mut st.estate[i],
        TypeNode::Array(i) => &mut st.astate[i],
    }
}

fn node_name<'a>(
    node: TypeNode,
    structs: &'a [StructDecl],
    enums: &'a [EnumDecl],
    arrays: &'a [ArrayDecl],
) -> &'a str {
    match node {
        TypeNode::Struct(i) => structs[i].name.as_str(),
        TypeNode::Enum(i) => enums[i].name.as_str(),
        TypeNode::Array(i) => arrays[i].name_static,
    }
}

fn visit_recursion(
    node: TypeNode,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    st: &mut RecursionState,
) -> Result<(), String> {
    *node_state(node, st) = VisitState::InProgress;
    st.path.push(node);
    for child in node_edges(node, structs, enums, arrays) {
        match *node_state(child, st) {
            VisitState::Unvisited => visit_recursion(child, structs, enums, arrays, st)?,
            VisitState::InProgress => {
                let cycle_start = st.path.iter().position(|&x| x == child).unwrap();
                let mut names: Vec<&str> = st.path[cycle_start..]
                    .iter()
                    .map(|&n| node_name(n, structs, enums, arrays))
                    .collect();
                names.push(node_name(child, structs, enums, arrays));
                // Key the wording on the repeated node's kind so a pure-struct
                // cycle keeps its Slice 3 message, an enum cycle names an enum
                // (X3), and an array cycle names the array (X5).
                let kind = match child {
                    TypeNode::Struct(_) => "struct",
                    TypeNode::Enum(_) => "enum",
                    TypeNode::Array(_) => "array",
                };
                return Err(format!(
                    "error: recursive {kind} definition (infinite size): {}",
                    names.join(" -> ")
                ));
            }
            VisitState::Done => {}
        }
    }
    st.path.pop();
    *node_state(node, st) = VisitState::Done;
    Ok(())
}

/// Synthesize the generated-word `Sig`s for every registered struct, in
/// declared field order (first field deepest): a constructor
/// `S ( T1 … Tn -- S )`, a destructure `S> ( S -- T1 … Tn )`, and per field a
/// getter `S>fi ( S -- Ti )` and a functional setter `S<fi ( S Ti -- S )`. A
/// zero-field struct registers only the constructor and destructure. These
/// join the env alongside user words, so applying one to the wrong arity or
/// operand type is caught by the same arity/type-mismatch path as any other
/// word call.
pub fn struct_generated_sigs(structs: &[StructDecl]) -> Vec<(String, Sig)> {
    let mut sigs = Vec::new();
    for (idx, decl) in structs.iter().enumerate() {
        let struct_ty = Type::Struct(StructId::from_index(idx), decl.name_static);
        let field_types: Vec<Type> = decl.fields.iter().map(|(_, ty)| *ty).collect();

        sigs.push((
            decl.name.clone(),
            Sig {
                inputs: field_types.clone(),
                outputs: vec![struct_ty],
            },
        ));
        sigs.push((
            format!("{}>", decl.name),
            Sig {
                inputs: vec![struct_ty],
                outputs: field_types.clone(),
            },
        ));
        for (field_name, field_ty) in &decl.fields {
            sigs.push((
                format!("{}>{}", decl.name, field_name),
                Sig {
                    inputs: vec![struct_ty],
                    outputs: vec![*field_ty],
                },
            ));
            sigs.push((
                format!("{}<{}", decl.name, field_name),
                Sig {
                    inputs: vec![struct_ty, *field_ty],
                    outputs: vec![struct_ty],
                },
            ));
        }
    }
    sigs
}

/// Synthesize the generated-word `Sig` for every registered enum variant
/// (D2, R9): a constructor `Variant ( T1 … Tn -- Enum )`, fields in declared
/// order (first field deepest), a zero-field variant being `Variant ( --
/// Enum )`. Unlike a struct, a variant has no destructure/getter/setter
/// (D2: not a standalone type; elimination is clause-style, Phase 4). These
/// join the env alongside user words and struct-generated words, so a
/// constructor's arity/field-type misuse (X9) falls out of the existing
/// call-check path.
pub fn enum_generated_sigs(enums: &[EnumDecl]) -> Vec<(String, Sig)> {
    let mut sigs = Vec::new();
    for (idx, decl) in enums.iter().enumerate() {
        let enum_ty = Type::Enum(EnumId::from_index(idx), decl.name_static);
        for variant in &decl.variants {
            let field_types: Vec<Type> = variant.fields.iter().map(|(_, ty)| *ty).collect();
            sigs.push((
                variant.name.clone(),
                Sig {
                    inputs: field_types,
                    outputs: vec![enum_ty],
                },
            ));
        }
    }
    sigs
}

/// Check a single word definition against an external env, seeding the env with
/// the word's own signature so self-recursion type-checks. `enums` is the
/// registry the clause-style checks (coverage, scrutinee type, variant-name
/// collision) consult.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_def(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    poly_env: &HashMap<String, (PolySig, Option<u64>)>,
    combinators: &HashMap<String, Combinator>,
) -> Result<HashMap<Span, CallInst>, String> {
    let (_sites, insts) = check_def_collecting_drop_sites(
        word,
        enums,
        env,
        arrays,
        cells,
        refs,
        structs,
        poly_env,
        combinators,
    )?;
    Ok(insts)
}

/// R6/R11: `check_def`'s own body-check, but returning this one word's
/// recorded `drop` call sites instead of discarding them. The REPL keeps the
/// result cached per override (`Session::drop_dropped_sites`) so a later
/// line's reachability query (`check_drop_overload_reachability`) never has
/// to re-check an *earlier* override's body against a *later* line's env --
/// the same stale-env hazard R11.2/R11.3 already fixed for lowering. A
/// `drop` call site's resolved operand type does not change once recorded;
/// only whether that type is *currently* overridden can, and that question
/// is answered fresh, from `structs`, every time the graph is built.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_def_collecting_drop_sites(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    poly_env: &HashMap<String, (PolySig, Option<u64>)>,
    combinators: &HashMap<String, Combinator>,
) -> Result<(Vec<Type>, HashMap<Span, CallInst>), String> {
    let mut env = env.clone();
    env.insert(word.name.clone(), sig_of(&word.effect));
    let mut sites = Vec::new();
    // R5 (Slice 2): the session poly-env threads through so a defined word's
    // own body can call a retained polymorphic word; the REPL drop-overload
    // collector passes the empty map (a `drop` overload is never polymorphic),
    // keeping the reachability walk byte-identical on the concrete path (D2).
    let mut insts: HashMap<Span, CallInst> = HashMap::new();
    // R3 (Slice 6c): the session's retained combinators thread through so a
    // defined word's body can call one and have it inlined, exactly as native
    // inlines one drawn from `module.words`. The build path and unit tests
    // pass the empty map, keeping the concrete path byte-identical.
    let mut poly = PolyCtx {
        env: poly_env,
        insts: &mut insts,
        combinators,
    };
    check_word(
        word, enums, &env, arrays, cells, refs, structs, &mut sites, &mut poly,
    )?;
    Ok((sites, insts))
}

/// R6/R11: the REPL's own whole-session call to `check_drop_overload_recursion`,
/// asked over every override currently live in the session (the new one
/// already included) and each one's *cached* `drop` call sites
/// (`check_def_collecting_drop_sites`, recorded once per override, at the
/// line that defined it) rather than a re-check of every body.
pub fn check_drop_overload_reachability(
    overrides: &[(StructId, &WordDef, &[Type])],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
) -> Result<(), String> {
    let words: Vec<&WordDef> = overrides.iter().map(|&(_, word, _)| word).collect();
    let overloads: HashMap<StructId, usize> = overrides
        .iter()
        .enumerate()
        .map(|(i, &(id, _, _))| (id, i))
        .collect();
    let dropped: Vec<Vec<Type>> = overrides
        .iter()
        .map(|&(_, _, sites)| sites.to_vec())
        .collect();
    check_drop_overload_recursion(&words, structs, enums, arrays, cells, &overloads, &dropped)
}

/// Infer the net effect of a bare line: simulate the typed stack from
/// `entry_stack` (the carried slot types) and return the resulting typed stack.
/// A type mismatch or underflow against the carried stack is a reported error.
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_line(
    terms: &[Term],
    entry_stack: &[Type],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    poly_env: &HashMap<String, (PolySig, Option<u64>)>,
    combinators: &HashMap<String, Combinator>,
) -> Result<(Vec<Type>, HashMap<Span, CallInst>), String> {
    let initial: Vec<Slot> = entry_stack.iter().map(|ty| Slot::computed(*ty)).collect();
    // A line is one block: names it binds die with it, so its end is a scope
    // end like any other. It is not a word body, so nothing in it is in tail
    // position.
    let ctx = Ctx::Line { structs, enums };
    let mut scope = Scope::default();
    let mut prov = Provenance::default();
    // R5 (Slice 2): the session poly-env threads through so a bare line can
    // call a retained polymorphic word; the filled instantiation table is
    // relayed to the caller for lowering. A `build`-path caller passes the
    // empty map (Slice 1's D2 behaviour).
    let mut insts: HashMap<Span, CallInst> = HashMap::new();
    // R3 (Slice 6c): the session's retained combinators thread through so a
    // bare line can call one and have it inlined, exactly as native inlines one
    // drawn from `module.words`. The build path and unit tests pass empty.
    let mut poly = PolyCtx {
        env: poly_env,
        insts: &mut insts,
        combinators,
    };
    let final_stack = check_terms(
        terms, initial, &ctx, env, arrays, cells, refs, &mut prov, &mut scope, false, &mut poly,
    )?;
    let line = terms.last().map(|t| t.span.line).unwrap_or(0);
    leave_block(&ctx, &mut scope, 0, BlockEnd::Body(line))?;
    // R19: a REPL line has no declared outputs (so R10's route never runs),
    // yet the session carries its residual stack into the next line while the
    // `quot` side channel dies at the boundary and lowering has pushed a
    // phantom the spill would marshal. Reject a quotation left here.
    if final_stack.iter().any(|s| s.quot.is_some()) {
        return Err(
            "error: a quotation cannot be left on the stack at the end of a line: the session carries it into the next line, and only `call` and `times` accept a quotation (a runtime quotation value is slice 7)".to_string(),
        );
    }
    // The sixth position of the no-stored-reference rule: the session's
    // inter-line stack outlives this line's
    // locals, so a reference that survived to here would outlive its referent.
    if let Some(slot) = final_stack
        .iter()
        .find(|s| contains_reference(s.ty, structs, enums, arrays))
    {
        return Err(format!(
            "error: a reference cannot be stored: the line leaves `{}` on the stack, which the session carries into the next line\n  a `&T`/`&!T` borrows a local of this line, and this line's locals are gone by then",
            slot.ty
        ));
    }
    Ok((final_stack.into_iter().map(|s| s.ty).collect(), insts))
}

/// `main` is the program's entry point: nothing in the program calls it, so
/// a linear value in its declared effect either leaks past the program
/// boundary unnoticed (an output) or runs a destructor over an
/// uninitialised ABI register (an input). A non-empty Copy-typed effect on
/// `main` stays legal; only a non-Copy type in either side is rejected.
fn check_main_effect(
    words: &[WordDef],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    let Some(main) = words.iter().find(|w| w.name == "main") else {
        return Ok(());
    };
    let offending = main
        .effect
        .inputs
        .iter()
        .chain(&main.effect.outputs)
        .map(|slot| slot.ty)
        .find(|ty| is_linear(*ty, structs, enums, arrays));
    let Some(ty) = offending else {
        return Ok(());
    };
    let span = word_span(main);
    Err(format!(
        "error: `main` (line {}) cannot declare a linear type `{}` in its stack effect\n  note: declared {}",
        span.line, ty, effect_str(&main.effect)
    ))
}

fn effect_str(effect: &StackEffect) -> String {
    let ins: Vec<String> = effect.inputs.iter().map(|s| s.ty.to_string()).collect();
    let outs: Vec<String> = effect.outputs.iter().map(|s| s.ty.to_string()).collect();
    let mut parts = vec!["--".to_string()];
    if !outs.is_empty() {
        parts.push(outs.join(" "));
    }
    if !ins.is_empty() {
        parts.insert(0, ins.join(" "));
    }
    format!("( {} )", parts.join(" "))
}

/// Whether `name` is a registered variant name of any enum (the D8 backstop's
/// lookup set).
fn is_registered_variant(name: &str, enums: &[EnumDecl]) -> bool {
    enums
        .iter()
        .any(|e| e.variants.iter().any(|v| v.name == name))
}

/// D8's clause-vs-binding disambiguation is global (`is_variant_name` scans
/// every enum in scope), so a `|` followed by a registered variant always
/// opens the next clause and is never read as a binding (R8). Every clause
/// the parser produces leads with a registered name, so this note states the
/// rule that was applied wherever a clause's variant is rejected.
fn clause_variant_ambiguity_note(name: &str) -> String {
    format!(
        "\n  note: `| {name}` is read as a clause because `{name}` is a variant name; a binding may not lead with one"
    )
}

/// A parameter / word-entry / clause-body binding name equal to a registered
/// variant name is a sharp error (D8 backstop, X12): it would make the
/// clause-vs-locals `|` disambiguation ambiguous.
fn reject_variant_local(ctx: &Ctx, name: &str, kind: &str) -> Result<(), String> {
    if !is_registered_variant(name, ctx.enums()) {
        return Ok(());
    }
    Err(match ctx {
        Ctx::Word {
            name: word_name, ..
        } => format!(
            "error: {kind} `{name}` in `{word_name}` collides with the variant name `{name}`"
        ),
        Ctx::Line { .. } => {
            format!("error: {kind} `{name}` collides with the variant name `{name}`")
        }
    })
}

/// A name repeated in a binding list (`| a a |`) collapses to last-wins when
/// zipped into the name -> type map, so the earlier binding (and any linear
/// value held in it) is tracked by nothing and never disposed. Reject
/// unconditionally, regardless of the bound type.
fn reject_duplicate_local<'a>(
    ctx: &Ctx,
    name: &'a str,
    span: Span,
    seen: &mut HashSet<&'a str>,
) -> Result<(), String> {
    if seen.insert(name) {
        return Ok(());
    }
    Err(match ctx {
        Ctx::Word {
            name: word_name, ..
        } => format!(
            "error: duplicate local `{name}` in `{word_name}` (line {})\n  `{name}` is bound twice; the second binding shadows the first and silently drops it",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: duplicate local `{name}` (line {})\n  `{name}` is bound twice; the second binding shadows the first and silently drops it",
            span.line
        ),
    })
}

/// The output-count / output-type mismatch check shared by a term body and a
/// clause body (M6, X8): `final_stack` must match the declared outputs.
/// Honors D8's literal coercion (a bare integer literal satisfies a declared
/// `usize` output) and reports the X10 diagnostic for a computed one.
fn check_outputs(
    word: &WordDef,
    final_stack: &[Slot],
    declared: &[Type],
    line: u32,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    // R10: a quotation left on the exit stack gets its own diagnostic, ahead
    // of both the arity and type-mismatch routes. On a *matching* count the
    // ordinary mismatch would otherwise fire and leak the `Cstr` placeholder
    // spelling; a quotation cannot be a declared output regardless of count.
    if final_stack.iter().any(|s| s.quot.is_some()) {
        return Err(format!(
            "error: `{}` (line {}) leaves a quotation on the stack; a quotation cannot be a declared output",
            word.name, line
        ));
    }
    if final_stack.len() != declared.len() {
        // R13/R2: a *linear* surplus value is the forgotten-disposal case, so it
        // gets the disposal wording (and names its type) before the generic
        // arity error a surplus Copy value keeps.
        if let Some(slot) = final_stack
            .get(declared.len()..)
            .unwrap_or_default()
            .iter()
            .find(|s| is_linear(s.ty, structs, enums, arrays))
        {
            return Err(surplus_linear_value_error(word, slot.ty, line));
        }
        return Err(format!(
            "error: stack effect mismatch in `{}` (line {})\n  body leaves {} values, but ( … ) declares {} outputs\n  note: declared {}",
            crate::resolve::demangle_word(&word.name), line, final_stack.len(), declared.len(), effect_str(&word.effect),
        ));
    }
    for (found, want) in final_stack.iter().zip(declared) {
        match match_slot(*found, *want) {
            SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
            SlotMatch::NeedsSizeConversion => {
                return Err(format!(
                    "error: type mismatch in `{}` (line {})\n  body leaves a computed `i64` where the declaration requires `{}`: convert it explicitly with `>{}` first (a bare integer literal coerces automatically, a computed value does not)\n  note: declared {}",
                    word.name, line, want, want, effect_str(&word.effect),
                ));
            }
            SlotMatch::NeedsStrToCstrConversion => {
                return Err(format!(
                    "error: type mismatch in `{}` (line {})\n  body leaves `str` where the declaration requires `cstr`: convert it explicitly with `cstr` first (there is no implicit `str` -> `cstr` conversion)\n  note: declared {}",
                    word.name, line, effect_str(&word.effect),
                ));
            }
            SlotMatch::Mismatch => {
                return Err(format!(
                    "error: type mismatch in `{}` (line {})\n  body leaves `{}` where the declaration requires `{}`\n  note: declared {}",
                    word.name, line, found.ty, want, effect_str(&word.effect),
                ));
            }
        }
    }
    Ok(())
}

/// R1 (D2, D7): the callee names of every tail-position call in a word body.
///
/// Tail position is a purely *syntactic* property: a call is in tail position
/// iff it is the final term of a terms body, the final term of a clause body,
/// or the final term of an arm of a *terminal* `if` (an `if` that is itself
/// the final term hands tail position to the last term of both arms,
/// recursively). Any term after a call, arithmetic, a shuffle, a consumer, or
/// another call, breaks tail position, and a call inside a non-terminal `if`
/// is not tail. Output-equality with the declared outputs is a *consequence*
/// of this rule for a well-typed final call, not a second check.
///
/// Shared by the checker (R2 predicate, R3 tail-call graph); the lowerer
/// re-encodes the same syntactic rule via positional `tail` threading in
/// `lower_terms` (src/ir.rs), which a name list can't express. The two must
/// stay in lockstep if the tail rule changes.
pub fn tail_position_calls(body: &WordBody) -> Vec<&str> {
    let mut out = Vec::new();
    match body {
        WordBody::Terms { terms, .. } => collect_tail_calls(terms, &mut out),
        WordBody::Clauses(clauses) => {
            for clause in clauses {
                collect_tail_calls(&clause.body, &mut out);
            }
        }
    }
    out
}

fn collect_tail_calls<'a>(terms: &'a [Term], out: &mut Vec<&'a str>) {
    let Some(last) = terms.last() else {
        return;
    };
    match &last.kind {
        TermKind::Call(name) => out.push(name.as_str()),
        TermKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_tail_calls(then_branch, out);
            collect_tail_calls(else_branch, out);
        }
        _ => {}
    }
}

/// R2 (M1): whether a word contains at least one tail-position call to itself.
/// The lowerer uses this to decide whether to build the loop shape at all.
///
/// A `drop` overload never self-tail-calls, whatever its body's last term is:
/// `drop` is intercepted as a builtin before any name resolution, at both
/// check and lowering, so a trailing `drop` in a `: drop ( T -- )` body is a
/// disposal of whatever is on top (typically some `Copy` scalar), not a call
/// to the enclosing word. Without this, the dogfood's own
/// `| f | f File>fd close drop ;` would lower to a back-edge loop instead of
/// closing the fd.
pub fn has_self_tail_call(word: &WordDef) -> bool {
    word.name != "drop"
        && tail_position_calls(&word.body)
            .iter()
            .any(|&callee| callee == word.name)
}

/// A word's location, derived from the first term (or clause) of its body,
/// for locating a whole-word diagnostic like X1.
pub(crate) fn word_span(word: &WordDef) -> Span {
    match &word.body {
        WordBody::Terms { terms, .. } => terms.first().map(|t| t.span).unwrap_or_default(),
        WordBody::Clauses(clauses) => clauses.first().map(|c| c.span).unwrap_or_default(),
    }
}

/// R3/R4 (D3, X1): build the whole-module tail-call graph (an edge `A -> B`
/// iff `A` has a tail-position call to user word `B`) and reject any cycle of
/// length >= 2. A self-loop (`A -> A`) is tier-1 self-tail-recursion and
/// allowed; only mutual cycles are the error. Builtins, generated words, and
/// non-tail calls contribute no edge, so a pair of words that mutually call
/// each other in non-tail position never false-positives.
fn check_tail_call_cycles(
    words: &[WordDef],
    drop_overload_indices: &HashSet<usize>,
) -> Result<(), String> {
    // A recognized `drop` overload is not callable by name (`check_shuffle`'s
    // `"drop"` arm intercepts every call site first), so it contributes no
    // edge in either direction: a body's trailing `drop` of a scalar would
    // otherwise register a tail call *to* the overload and fabricate a cycle.
    // Keyed by registry membership, not the literal name, matching every
    // other exclusion in this pass.
    let name_to_idx: HashMap<&str, usize> = words
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop_overload_indices.contains(i))
        .map(|(i, w)| (w.name.as_str(), i))
        .collect();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for (i, word) in words.iter().enumerate() {
        for callee in tail_position_calls(&word.body) {
            if let Some(&j) = name_to_idx.get(callee) {
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
    }

    let mut color = vec![0u8; words.len()];
    let mut path: Vec<usize> = Vec::new();
    for start in 0..words.len() {
        if color[start] == 0 {
            if let Some(cycle) = find_tail_cycle(start, &adj, &mut color, &mut path) {
                return Err(mutual_tail_recursion_error(words, &cycle));
            }
        }
    }
    Ok(())
}

/// DFS from `u` over the tail-call graph, returning the members (in order) of
/// the first cycle of length >= 2 reached. A self-edge (`v == u`) is skipped:
/// tier-1 self-tail-recursion is allowed. `color`: 0 unvisited, 1 on the
/// current path, 2 finished.
fn find_tail_cycle(
    u: usize,
    adj: &[Vec<usize>],
    color: &mut [u8],
    path: &mut Vec<usize>,
) -> Option<Vec<usize>> {
    color[u] = 1;
    path.push(u);
    for &v in &adj[u] {
        if v == u {
            continue;
        }
        if color[v] == 1 {
            let start = path.iter().position(|&x| x == v).unwrap();
            return Some(path[start..].to_vec());
        }
        if color[v] == 0 {
            if let Some(cycle) = find_tail_cycle(v, adj, color, path) {
                return Some(cycle);
            }
        }
    }
    path.pop();
    color[u] = 2;
    None
}

/// X1: a located mutual-tail-recursion error naming the cycle members in
/// order, closing the loop back to the first (e.g. `` `a` -> `b` -> `a` ``).
fn mutual_tail_recursion_error(words: &[WordDef], cycle: &[usize]) -> String {
    let mut chain: Vec<&str> = cycle
        .iter()
        .map(|&i| crate::resolve::demangle_word(words[i].name.as_str()))
        .collect();
    chain.push(chain[0]);
    let rendered = chain
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(" -> ");
    let span = word_span(&words[cycle[0]]);
    format!(
        "error: mutual tail recursion {} (line {}, col {})",
        rendered, span.line, span.col
    )
}

/// R6 (D4, slice 8b): reject a `drop` overload that can reach itself. Per
/// override, the question is only whether `drop@T`'s own word is reachable
/// from itself through any sequence of calls, direct or indirect -- a bare
/// self-call is the cycle of length one, a chain through helpers the general
/// case, and a `drop` of some *other* aggregate merely containing a `T` is
/// the same question again, since disposing that aggregate runs `T`'s
/// override through its own generic field glue.
///
/// This cannot be a sibling of `check_tail_call_cycles`, run before body
/// checking: resolving *which* override a `drop` call site dispatches to
/// needs the operand's static type, and nothing computes that before
/// `check_word`'s per-term stack simulation. A purely syntactic pass over
/// callee names (`check_tail_call_cycles`'s own shape) could not tell
/// `drop@File` from the `drop` of the `i64` that `close` returns, and so
/// would reject the dogfood outright.
///
/// **Known, accepted limitation:** reachability is not data-flow, so it is
/// context-insensitive. A helper called from `drop@T` that is *separately*
/// reachable back to `drop@T` only down a branch never taken from there
/// still reads as a cycle -- the same false positive the tail-cycle pass
/// already accepts, with the same remedy: factor out a distinct helper.
fn check_drop_overload_recursion(
    words: &[&WordDef],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    overloads: &HashMap<StructId, usize>,
    dropped: &[Vec<Type>],
) -> Result<(), String> {
    if overloads.is_empty() {
        return Ok(());
    }
    let adj = drop_reachability_graph(words, structs, enums, arrays, cells, overloads, dropped);
    // Sorted by struct id, so a program with two offending overloads always
    // reports the same one.
    let mut targets: Vec<(StructId, usize)> = overloads.iter().map(|(&id, &i)| (id, i)).collect();
    targets.sort_by_key(|(id, _)| id.index());
    for (id, idx) in targets {
        let mut visited = vec![false; words.len()];
        visited[idx] = true;
        let mut chain = vec![idx];
        if reaches_start(idx, &adj, &mut visited, &mut chain) {
            return Err(recursive_drop_overload_error(
                words, structs, overloads, id, &chain,
            ));
        }
    }
    Ok(())
}

/// R6: the whole-program graph the reachability question is asked over. Two
/// kinds of edge out of a word `A`:
///
/// - an ordinary call anywhere in `A`'s body resolving to a user word `B`
///   (**any** position, unlike `tail_position_calls`, which only ever reads
///   `terms.last()`);
/// - `A -> drop@T` for a `drop` call site in `A` whose recorded operand type
///   either *is* the overridden struct `T`, or is an aggregate with no
///   override of its own whose linear fields reach `T` through ordinary,
///   non-overridden composition.
///
/// Every edge is resolved through the `StructId`-keyed override table, never
/// through a name-keyed map: the literal name `"drop"` is shared by every
/// override and says nothing about which one a site dispatches to.
fn drop_reachability_graph(
    words: &[&WordDef],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    overloads: &HashMap<StructId, usize>,
    dropped: &[Vec<Type>],
) -> Vec<Vec<usize>> {
    // An override is not callable by name (every `drop` call site is
    // intercepted before name resolution reaches `env`), so it contributes no
    // name edge in either direction: its only incoming edges are `drop` sites.
    let overload_words: HashSet<usize> = overloads.values().copied().collect();
    let name_to_idx: HashMap<&str, usize> = words
        .iter()
        .enumerate()
        .filter(|(i, _)| !overload_words.contains(i))
        .map(|(i, w)| (w.name.as_str(), i))
        .collect();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for (i, word) in words.iter().enumerate() {
        for callee in all_calls(&word.body) {
            if let Some(&j) = name_to_idx.get(callee) {
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
        for &ty in &dropped[i] {
            let mut targets = Vec::new();
            collect_drop_targets(
                ty,
                structs,
                enums,
                arrays,
                cells,
                overloads,
                &mut Vec::new(),
                &mut targets,
            );
            for j in targets {
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
    }
    adj
}

/// R6: the override bodies one `drop` call site can run, given its operand
/// type. A check-side fold over `StructDecl` fields, shaped like `is_copy`'s,
/// because there is no `StructLayout` to walk yet -- `build_registries` runs
/// inside `ir::lower`, after `check` entirely.
///
/// An overridden struct is where the walk stops, the same boundary R7 applies
/// to the fused-loop search in `ir::expand_path`, and that one stop covers
/// both of R6's cases:
///
/// - at the root, it *is* case (a): dropping an overridden `B` runs `B`'s own
///   body, so the edge goes there and reachability continues from `B`'s own
///   recorded call sites during the DFS. Descending into `B`'s fields as well
///   would inspect field glue that never runs, and fabricate an edge.
/// - below the root, it is case (b)'s boundary: a non-overridden aggregate is
///   disposed by generic field glue, which calls each linear field's own
///   destructor, so every override reachable through that composition really
///   does run -- but the composition stops at the first override, for the
///   same reason.
///
/// A `Copy` type is a dead end because nothing disposes it at all. `seen` is
/// monotone (never popped) since the answer is a *set* of reachable
/// overrides, not a path, and a `^T` payload may close a type cycle the
/// struct and enum registries cannot.
#[allow(clippy::too_many_arguments)]
fn collect_drop_targets(
    ty: Type,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    overloads: &HashMap<StructId, usize>,
    seen: &mut Vec<Type>,
    found: &mut Vec<usize>,
) {
    if is_copy(ty, structs, enums, arrays) || seen.contains(&ty) {
        return;
    }
    seen.push(ty);
    let descend = |field: Type, seen: &mut Vec<Type>, found: &mut Vec<usize>| {
        collect_drop_targets(field, structs, enums, arrays, cells, overloads, seen, found)
    };
    match ty {
        Type::Struct(id, _) => {
            if let Some(&idx) = overloads.get(&id) {
                if !found.contains(&idx) {
                    found.push(idx);
                }
                return;
            }
            for (_, field_ty) in &structs[id.index()].fields {
                descend(*field_ty, seen, found);
            }
        }
        Type::Enum(id, _) => {
            for variant in &enums[id.index()].variants {
                for (_, field_ty) in &variant.fields {
                    descend(*field_ty, seen, found);
                }
            }
        }
        Type::Array(id, _) => descend(arrays[id.index()].element, seen, found),
        Type::OwnedCell(id, _) => descend(cells[id.index()].payload, seen, found),
        _ => {}
    }
}

/// R6: every callee name a body mentions, in any position -- the whole-body
/// sibling of `tail_position_calls`, which only ever reads `terms.last()`.
/// Both `if` arms and every clause body are visited.
///
/// A local's own name reads as a `Call` term too, so a local sharing a word's
/// name contributes an edge that no call justifies. That over-approximation
/// can only add edges, never lose one, and is the same one
/// `check_tail_call_cycles` already lives with.
fn all_calls(body: &WordBody) -> Vec<&str> {
    let mut out = Vec::new();
    match body {
        WordBody::Terms { terms } => collect_all_calls(terms, &mut out),
        WordBody::Clauses(clauses) => {
            for clause in clauses {
                collect_all_calls(&clause.body, &mut out);
            }
        }
    }
    out
}

fn collect_all_calls<'a>(terms: &'a [Term], out: &mut Vec<&'a str>) {
    for term in terms {
        match &term.kind {
            TermKind::Call(name) => out.push(name.as_str()),
            TermKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                collect_all_calls(then_branch, out);
                collect_all_calls(else_branch, out);
            }
            _ => {}
        }
    }
}

/// Whether `start` is reachable from the last word on `chain`, growing
/// `chain` into the route that gets there. A node is marked on the way down
/// and never unmarked: if it could reach `start`, the search from it already
/// said so, so skipping it on a later branch cannot lose a cycle.
fn reaches_start(
    start: usize,
    adj: &[Vec<usize>],
    visited: &mut [bool],
    chain: &mut Vec<usize>,
) -> bool {
    let u = *chain.last().expect("reachability chain is never empty");
    for &v in &adj[u] {
        if v == start {
            return true;
        }
        if !visited[v] {
            visited[v] = true;
            chain.push(v);
            if reaches_start(start, adj, visited, chain) {
                return true;
            }
            chain.pop();
        }
    }
    false
}

/// R6: a located error naming the whole cycle in order, closing back to the
/// override it started from, and naming `T>` as the remedy -- modeled on
/// `mutual_tail_recursion_error`'s shape. An override has no callable name of
/// its own, so it is rendered as the declaration the user wrote.
fn recursive_drop_overload_error(
    words: &[&WordDef],
    structs: &[StructDecl],
    overloads: &HashMap<StructId, usize>,
    id: StructId,
    chain: &[usize],
) -> String {
    let render = |i: usize| match overloads.iter().find(|(_, &w)| w == i) {
        Some((sid, _)) => format!("`drop ( {} -- )`", structs[sid.index()].name),
        None => format!("`{}`", words[i].name),
    };
    let mut rendered: Vec<String> = chain.iter().map(|&i| render(i)).collect();
    rendered.push(render(chain[0]));
    let name = &structs[id.index()].name;
    let span = word_span(words[overloads[&id]]);
    format!(
        "error: recursive `drop` overload for `{}`: {} (line {}, col {})\n  a `drop` body cannot dispose its own receiver, directly or through any chain of calls; destructure it with `{}>` and dispose the fields instead",
        name,
        rendered.join(" -> "),
        span.line,
        span.col,
        name
    )
}

#[allow(clippy::too_many_arguments)]
fn check_word(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    // A parameter name equal to a registered variant name is rejected (X12)
    // regardless of body form.
    let ctx = word_ctx(word, structs, enums);
    for slot in &word.effect.inputs {
        if let Some(name) = &slot.name {
            reject_variant_local(&ctx, name, "parameter")?;
        }
    }
    check_reference_free_signature(&word.name, &word.effect, structs, enums, arrays)?;
    match &word.body {
        WordBody::Terms { terms } => check_terms_word(
            word, enums, terms, env, arrays, cells, refs, structs, dropped, poly,
        ),
        WordBody::Clauses(clauses) => check_clause_word(
            word, enums, clauses, env, arrays, cells, refs, structs, dropped, poly,
        ),
    }
}

/// The effect-signature half of the no-stored-reference rule: no declared
/// **output** may transitively
/// contain a reference (returning one would outlive the frame that owns the
/// referent), and an **input** may only be a reference at the top level — a
/// type that merely *contains* one nested inside an array or a cell is
/// rejected there too, so the carve-out stays closed if a future aggregate
/// constructor arrives.
fn check_reference_free_signature(
    name: &str,
    effect: &StackEffect,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    for slot in &effect.outputs {
        if contains_reference(slot.ty, structs, enums, arrays) {
            return Err(format!(
                "error: a reference cannot be stored: `{}` declares the output `{}`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead",
                name, slot.ty
            ));
        }
    }
    for slot in &effect.inputs {
        if !slot.ty.is_ref() && contains_reference(slot.ty, structs, enums, arrays) {
            return Err(format!(
                "error: a reference cannot be stored: `{}` declares the input `{}`, which contains a reference\n  an input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate",
                name, slot.ty
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_terms_word(
    word: &WordDef,
    enums: &[EnumDecl],
    terms: &[Term],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    // R3: a binding is an ordinary term, but the *entry* one keeps its own
    // diagnostic. Only there is the declared effect the frame, so only there
    // can the message cite it; the generic underflow of R5 covers every other
    // position.
    let inputs = word.effect.inputs.len();
    if let Some(TermKind::Bind(names)) = terms.first().map(|t| &t.kind) {
        if names.len() > inputs {
            return Err(format!(
                "error: stack effect mismatch in `{}`\n  locals bind {} value(s), but only {} input(s) are declared\n  note: declared {}",
                crate::resolve::demangle_word(&word.name),
                names.len(),
                inputs,
                effect_str(&word.effect),
            ));
        }
    }

    // The declared inputs are the word's whole frame: it cannot see beneath
    // them (R5), and an entry binding pops from them like any other binding.
    let initial: Vec<Slot> = word
        .effect
        .inputs
        .iter()
        .map(|s| Slot::computed(s.ty))
        .collect();

    let ctx = word_ctx(word, structs, enums);
    let mut scope = Scope::default();
    let mut prov = Provenance::default();
    let mut final_stack = check_terms(
        terms, initial, &ctx, env, arrays, cells, refs, &mut prov, &mut scope, true, poly,
    )?;

    let declared: Vec<Type> = word.effect.outputs.iter().map(|s| s.ty).collect();
    let line = terms.last().map(|t| t.span.line).unwrap_or(0);
    // R7/D4: a declared `Type::Quotation` output is a materialization boundary.
    // Materialize each non-capturing `Known` literal the body leaves there
    // (reject a capturing one naming 7b, R12) before `check_outputs`, whose
    // bare-quotation guard would otherwise reject it outright.
    for (i, want) in declared.iter().enumerate() {
        if let Type::Quotation(eff) = *want {
            if let Some(QuotRef::Known(id)) = final_stack.get(i).and_then(|s| s.quot) {
                let span = prov.quotations[id.0].span;
                final_stack[i] = materialize_quotation_at_boundary(
                    id,
                    eff,
                    "be returned",
                    &word.name,
                    span,
                    &ctx,
                    env,
                    arrays,
                    cells,
                    refs,
                    &mut prov,
                    &mut scope,
                    poly,
                )?;
            }
        }
    }
    dropped.append(&mut prov.dropped);

    check_outputs(word, &final_stack, &declared, line, structs, enums, arrays)?;
    leave_block(&ctx, &mut scope, 0, BlockEnd::Body(line))
}

/// Check a clause-style word (D4, D5, D7, M6, R11): the top input must be an
/// enum (X7), the clauses must cover every variant exactly once (X4/X5/X6),
/// and every clause body must leave the word's single declared output effect
/// (X8).
#[allow(clippy::too_many_arguments)]
fn check_clause_word(
    word: &WordDef,
    enums: &[EnumDecl],
    clauses: &[Clause],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    // The top input may be a plain enum (value mode) or a reference to
    // one (reference mode, `&Enum`/`&!Enum`) — the mode follows the declared
    // type, never inferred. `ref_mutable` is `None` in value mode, `Some`
    // (carrying the reference's mutability) in reference mode.
    let (enum_id, ref_mutable) = match word.effect.inputs.last().map(|s| s.ty) {
        Some(Type::Enum(id, _)) => (id, None),
        Some(Type::Ref(rid, mutable, _)) => match refs[rid.index()].referent {
            Type::Enum(id, _) => (id, Some(mutable)),
            _ => {
                return Err(format!(
                    "error: clause-style body on `{}` whose top input is not an enum\n  note: declared {}",
                    crate::resolve::demangle_word(&word.name),
                    effect_str(&word.effect),
                ));
            }
        },
        _ => {
            return Err(format!(
                "error: clause-style body on `{}` whose top input is not an enum\n  note: declared {}",
                crate::resolve::demangle_word(&word.name),
                effect_str(&word.effect),
            ));
        }
    };
    let enum_decl = &enums[enum_id.index()];
    let enum_name = enum_decl.name.as_str();

    let n_inputs = word.effect.inputs.len();
    let below: Vec<Type> = word.effect.inputs[..n_inputs - 1]
        .iter()
        .map(|s| s.ty)
        .collect();
    let declared: Vec<Type> = word.effect.outputs.iter().map(|s| s.ty).collect();

    // Validate every clause's variant identity and uniqueness before checking
    // any body (R8): a clause-body binding that leads with a registered
    // variant name is silently reparsed as the next clause, and if that
    // reparse produces an unknown-variant or duplicate-clause problem, it
    // must be reported before a downstream sibling body is checked against
    // the terms the reparse ate out from under it.
    let mut seen: HashMap<&str, ()> = HashMap::new();
    let mut variant_indices = Vec::with_capacity(clauses.len());
    for clause in clauses {
        let Some(vi) = enum_decl
            .variants
            .iter()
            .position(|v| v.name == clause.variant)
        else {
            return Err(format!(
                "error: unknown variant `{}` of enum `{}` in clause-style `{}` (line {}){}",
                clause.variant,
                enum_name,
                crate::resolve::demangle_word(&word.name),
                clause.span.line,
                clause_variant_ambiguity_note(&clause.variant),
            ));
        };
        if seen.insert(clause.variant.as_str(), ()).is_some() {
            return Err(format!(
                "error: duplicate clause for variant `{}` of enum `{}` in `{}` (line {}){}",
                clause.variant,
                enum_name,
                crate::resolve::demangle_word(&word.name),
                clause.span.line,
                clause_variant_ambiguity_note(&clause.variant),
            ));
        }
        variant_indices.push(vi);
    }
    // Exhaustiveness is part of that same pre-pass: a clause body that ate a
    // sibling's terms (a misspelt variant name reads as a binding, D8) leaves
    // that sibling missing, and "missing variant `B`" names the real fault
    // where the swollen body's own arity failure would misattribute it.
    for variant in &enum_decl.variants {
        if !seen.contains_key(variant.name.as_str()) {
            return Err(format!(
                "error: non-exhaustive clause-style `{}`: missing variant `{}` of enum `{}`",
                word.name, variant.name, enum_name
            ));
        }
    }
    for (clause, &vi) in clauses.iter().zip(&variant_indices) {
        check_clause_body(
            word,
            enums,
            clause,
            &below,
            &enum_decl.variants[vi],
            &declared,
            env,
            arrays,
            cells,
            refs,
            structs,
            ref_mutable,
            dropped,
            poly,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_clause_body(
    word: &WordDef,
    enums: &[EnumDecl],
    clause: &Clause,
    below: &[Type],
    variant: &VariantDecl,
    declared: &[Type],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    ref_mutable: Option<bool>,
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    let ctx = word_ctx(word, structs, enums);
    let mut seen_locals = HashSet::new();
    for name in &clause.locals {
        reject_variant_local(&ctx, name, "local")?;
        reject_duplicate_local(&ctx, name, clause.span, &mut seen_locals)?;
    }

    // The clause consumes the scrutinee and pushes the variant's fields
    // (first field deepest) atop any inputs below it. In reference mode
    // every field arrives as a reference inheriting the scrutinee's
    // mutability, projecting through it exactly as a struct-field projection
    // would — the payload is never owned, so it is never moved or freed.
    let mut initial = below.to_vec();
    for (_, ty) in &variant.fields {
        let field_ty = match ref_mutable {
            Some(mutable) => intern_ref_type(refs, *ty, mutable),
            None => *ty,
        };
        initial.push(field_ty);
    }

    // Clause-body `| names |` bind the top N (payload then below), leftmost
    // deepest, reusing the word-entry local-binding shape.
    let n = clause.locals.len();
    if n > initial.len() {
        return Err(format!(
            "error: stack effect mismatch in `{}` (line {})\n  clause `{}` binds {} value(s), but only {} are available\n  note: declared {}",
            word.name, clause.span.line, clause.variant, n, initial.len(), effect_str(&word.effect),
        ));
    }
    let split = initial.len() - n;
    let mut scope = Scope::default();
    let mut prov = Provenance::default();
    for (name, ty) in clause.locals.iter().zip(&initial[split..]) {
        let linear = is_linear(*ty, structs, enums, arrays);
        scope.bind(name, Slot::computed(*ty), linear, &mut prov);
    }
    let stack_after_bind: Vec<Slot> = initial[..split]
        .iter()
        .map(|ty| Slot::computed(*ty))
        .collect();

    let final_stack = check_terms(
        &clause.body,
        stack_after_bind,
        &ctx,
        env,
        arrays,
        cells,
        refs,
        &mut prov,
        &mut scope,
        true,
        poly,
    )?;
    dropped.append(&mut prov.dropped);
    let line = clause
        .body
        .last()
        .map(|t| t.span.line)
        .unwrap_or(clause.span.line);
    check_outputs(word, &final_stack, declared, line, structs, enums, arrays)?;
    leave_block(&ctx, &mut scope, 0, BlockEnd::Body(line))
}

/// R6: whether a concrete type satisfies an `Ord` bound. The numeric tower
/// (every integer width, `usize`/`isize`, and both floats) is totally ordered
/// for the comparison operators; nothing else is (`bool`, a struct, an array).
/// `max`'s float carve-out (X9) lives at its own builtin arm, not here.
fn is_ord(ty: Type) -> bool {
    ty.is_numeric()
}

/// R7: whether a `PolyType` slot is `Copy`. A bare variable answers *only*
/// from its bound set (never a concrete-type predicate), a concrete slot
/// delegates to `is_copy`, and an array is `Copy` iff its element is.
fn poly_is_copy(
    pt: &PolyType,
    sig: &PolySig,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> bool {
    match pt {
        PolyType::Concrete(t) => is_copy(*t, structs, enums, arrays),
        PolyType::Var(v) => sig.has_bound(*v, Bound::Copy),
        PolyType::Array(elem, _) => poly_is_copy(elem, sig, structs, enums, arrays),
        // Slice 6a (D3): a quotation parameter is always `Copy`, so it may be
        // called repeatedly and carries no move obligation.
        PolyType::Quotation(..) => true,
    }
}

/// R7 companion to the monomorphic `Scope`/`Moves`: the locals a polymorphic
/// body binds, paired with the move state of the ones that are not `Copy`. A
/// `Copy` local is read freely and never enters `moves`; a non-`Copy` local
/// (a bare variable with no `Copy` bound, or a concrete linear slot) is
/// consumed on its first read, so a second read is use-after-move and never
/// reading it leaks at the word's end (nothing is dropped for you).
#[derive(Debug, Clone, Default)]
struct PolyScope {
    locals: HashMap<String, PolyType>,
    moves: Moves,
}

impl PolyScope {
    /// The non-`Copy` locals still holding an unconsumed value, name-sorted so
    /// a body with two of them always reports the same one. A `MaybeMoved`
    /// local (consumed on one `if` arm only) counts as still-unconsumed here,
    /// which is the whole point of tracking three move states (D2).
    fn unconsumed(&self) -> Vec<&str> {
        self.moves.unconsumed()
    }

    /// The names bound before an `if` arm is walked; a name absent from this
    /// set after the arm was bound inside it. `PolyScope` has no depth concept
    /// and reports leaks name-sorted, so a keys-snapshot is the faithful twin
    /// of `Scope::depth` here, not an ordered `Vec`.
    fn snapshot(&self) -> HashSet<String> {
        self.locals.keys().cloned().collect()
    }

    /// `leave_block`'s poly twin: reject an arm-local non-`Copy` value never
    /// consumed inside the arm, then drop every arm-local from scope so the two
    /// arms' name sets agree at the join and `Moves::join` cannot panic on a
    /// key mismatch. `token` names the arm's closing keyword ("else" or "end")
    /// for the diagnostic. The removal happens whether or not a leak fired, so
    /// a successful arm always leaves the pre-`if` name set behind.
    fn leave_arm(
        &mut self,
        before: &HashSet<String>,
        token: &str,
        ctx: &Ctx,
        span: Span,
        sig: &PolySig,
    ) -> Result<(), String> {
        let mut arm_locals: Vec<String> = self
            .locals
            .keys()
            .filter(|k| !before.contains(k.as_str()))
            .cloned()
            .collect();
        arm_locals.sort_unstable();
        let unconsumed: HashSet<String> = self
            .moves
            .unconsumed()
            .iter()
            .map(|s| s.to_string())
            .collect();
        // An arm-local cannot be `MaybeMoved` (it does not exist in the other
        // arm), so its leak is always a plain unconsumed; take the first in
        // sort order, capturing its type before the removal drops it.
        let leak = arm_locals
            .iter()
            .find(|n| unconsumed.contains(n.as_str()))
            .map(|n| (n.clone(), self.locals[n].clone()));
        for name in &arm_locals {
            self.locals.remove(name);
            self.moves.states.remove(name);
        }
        if let Some((name, pt)) = leak {
            return Err(poly_arm_local_unconsumed_error(
                ctx, span, &name, &pt, token, sig,
            ));
        }
        Ok(())
    }
}

/// R14-R17: check a polymorphic combinator standalone by instantiating its
/// signature at concrete stand-in types and running the ordinary concrete
/// checker on the body. `i64` is Copy/Ord/numeric, so a body that only moves,
/// reads, and hands an element to its quotation parameter checks exactly as it
/// will at every Copy instantiation; the abstract `call`/`times` paths (R8/R9)
/// type `f call`/`f times` against the declared effect, and the three `times`
/// obligations (R16) fall out of the ordinary `times` check at the def site.
/// Instantiating every type variable at the same `i64` cannot mask a real
/// error the library relies on: the combinators never combine two distinct
/// element/accumulator variables directly (that arithmetic lives in the
/// caller's literal), and a type-specific misuse in some other combinator's
/// body is caught at its concrete splice site, the same place obligation 2's
/// borrow re-check lands (D4/R21). The array length is irrelevant to type
/// checking (`times` supplies a runtime index), so any value serves.
#[allow(clippy::too_many_arguments)]
fn check_poly_combinator_standalone(
    word: &WordDef,
    sig: &PolySig,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    poly: &mut PolyCtx,
) -> Result<(), String> {
    const STANDALONE_LEN: u32 = 4;
    let ctx = word_ctx(word, structs, enums);
    let span = word_span(word);
    let mut subst = Subst::default();
    for v in 0..sig.ty_var_names.len() as u32 {
        subst.ty.push((v, Type::I64));
    }
    for ln in 0..sig.len_var_names.len() as u32 {
        subst.len.push((ln, STANDALONE_LEN));
    }
    let mut inputs = Vec::with_capacity(sig.inputs.len());
    for pty in &sig.inputs {
        let ty = apply_subst(sig, pty, &subst, &word.name, span, &ctx, arrays)?;
        inputs.push(TypedSlot { name: None, ty });
    }
    let mut outputs = Vec::with_capacity(sig.outputs.len());
    for pty in &sig.outputs {
        let ty = apply_subst(sig, pty, &subst, &word.name, span, &ctx, arrays)?;
        outputs.push(TypedSlot { name: None, ty });
    }
    let terms = match &word.body {
        WordBody::Terms { terms } => terms.clone(),
        WordBody::Clauses(_) => {
            return Err(format!(
                "error: `{}` combines a clause-style body with a polymorphic signature, which is not supported",
                crate::resolve::demangle_word(&word.name)
            ));
        }
    };
    // A concrete stand-in for the combinator, checked by the ordinary path.
    let concrete = WordDef {
        name: word.name.clone(),
        effect: StackEffect { inputs, outputs },
        body: WordBody::Terms { terms },
        poly: None,
        module: word.module,
    };
    let mut dropped = Vec::new();
    check_word(
        &concrete,
        enums,
        env,
        arrays,
        cells,
        refs,
        structs,
        &mut dropped,
        poly,
    )
}

/// R9 (Slice 6c): the REPL's entry to the standalone poly-combinator check.
/// Builds a scratch `PolyCtx` (the instantiation records a spliced combinator
/// produces are never lowered, R20) around the session's poly-env and combinator
/// view, so `eval_combinator_def` need not name the private `PolyCtx`. Mirrors
/// native's `is_combinator` branch in `check`, deliberately bypassing
/// `eval_poly_def`'s `>= 2`-output deferral: a combinator is spliced inline and
/// never lowered to a bundle-returning `IrFunc`, so that gate cannot fire.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_poly_combinator_repl(
    word: &WordDef,
    sig: &PolySig,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    poly_env: &HashMap<String, (PolySig, Option<u64>)>,
    combinators: &HashMap<String, Combinator>,
) -> Result<(), String> {
    let mut scratch: HashMap<Span, CallInst> = HashMap::new();
    let mut poly = PolyCtx {
        env: poly_env,
        insts: &mut scratch,
        combinators,
    };
    check_poly_combinator_standalone(
        word, sig, enums, env, arrays, cells, refs, structs, &mut poly,
    )
}

/// R7: check a polymorphic word's body once, over a virtual stack of
/// `PolyType` (never the concrete `Slot` stack, S1/R4). Seeded from the
/// declared fixed inputs; the input row variable is an opaque below-stack
/// marker (the stack beneath the fixed inputs is passed through untouched, so
/// nothing is pushed for it and the residual stack is compared against the
/// declared fixed outputs). A bare variable supports only the five shuffles,
/// an operation its bound set permits (`dup`/`over` need `Copy`, the
/// comparisons need `Ord`), local bind/read, and being returned; every other
/// type-directed operation on it is a located error naming the variable, so a
/// body a real instantiation would reject can never slip through.
pub fn check_poly_body(
    word: &WordDef,
    sig: &PolySig,
    env: &HashMap<String, Sig>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    let ctx = word_ctx(word, structs, enums);
    let terms = match &word.body {
        WordBody::Terms { terms } => terms,
        WordBody::Clauses(_) => {
            return Err(format!(
                "error: `{}` combines a clause-style body with a polymorphic signature, which is not supported",
                crate::resolve::demangle_word(&word.name)
            ));
        }
    };
    let stack = sig.inputs.clone();
    let mut scope = PolyScope::default();
    let residual = poly_walk(
        terms, stack, &mut scope, sig, &ctx, env, structs, enums, arrays,
    )?;
    if residual != sig.outputs {
        return Err(poly_output_mismatch_error(word, sig, &residual));
    }
    // A non-`Copy` local never read still holds its value here; nothing is
    // auto-dropped, so it leaks. The monomorphic sibling rejects the same
    // shape at `leave_block`; the residual check above cannot see a value
    // parked in a local.
    if let Some(local) = scope.unconsumed().first().map(|s| s.to_string()) {
        let pt = scope.locals[&local].clone();
        return Err(poly_local_unconsumed_error(word, sig, &local, &pt));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn poly_walk(
    terms: &[Term],
    mut stack: Vec<PolyType>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<Vec<PolyType>, String> {
    for term in terms {
        stack = poly_term(term, stack, scope, sig, ctx, env, structs, enums, arrays)?;
    }
    Ok(stack)
}

#[allow(clippy::too_many_arguments)]
fn poly_term(
    term: &Term,
    mut stack: Vec<PolyType>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<Vec<PolyType>, String> {
    let span = term.span;
    match &term.kind {
        TermKind::IntLit(_) => stack.push(PolyType::Concrete(Type::I64)),
        TermKind::FloatLit(_) => stack.push(PolyType::Concrete(Type::F64)),
        TermKind::BoolLit(_) => stack.push(PolyType::Concrete(Type::Bool)),
        TermKind::StrLit(_) => stack.push(PolyType::Concrete(Type::Str)),
        TermKind::Bind(names) => {
            if stack.len() < names.len() {
                let op = format!("| {} |", names.join(" "));
                return Err(underflow_error(ctx, span, &op, names.len(), stack.len()));
            }
            // R4 twin of the monomorphic binder: a duplicate name inside this
            // one bind group would orphan the earlier binding, and re-binding a
            // name still in scope from an earlier group would do the same; a
            // non-`Copy` value parked in either could then never be consumed (a
            // silent leak).
            let mut seen = HashSet::new();
            for name in names {
                reject_variant_local(ctx, name, "local")?;
                reject_duplicate_local(ctx, name, span, &mut seen)?;
                if scope.locals.contains_key(name) {
                    return Err(rebound_local_error(ctx, span, name));
                }
            }
            let bound = stack.split_off(stack.len() - names.len());
            for (name, pt) in names.iter().zip(bound) {
                // A non-`Copy` binding carries a consume-exactly-once
                // obligation tracked in `moves`; a `Copy` one does not.
                if !poly_is_copy(&pt, sig, structs, enums, arrays) {
                    scope.moves.states.insert(name.clone(), MoveState::Live);
                }
                scope.locals.insert(name.clone(), pt);
            }
        }
        TermKind::If {
            then_branch,
            else_branch,
            else_span,
            end_span: _,
        } => {
            // Mirrors the monomorphic arm minus all quotation handling (D3): a
            // `PolyType` is provably never a quotation (the literal is rejected
            // eagerly at the `Quotation` arm above), so the condition-pop skips
            // `reject_quotation_operand` and the join skips the two
            // quotation-identity cases.
            let cond = stack
                .pop()
                .ok_or_else(|| underflow_error(ctx, span, "if", 1, 0))?;
            if cond != PolyType::Concrete(Type::Bool) {
                return Err(match cond {
                    PolyType::Concrete(t) => type_mismatch_error(ctx, span, "if", Type::Bool, t),
                    other => poly_op_on_variable_error(ctx, span, "if", &other, sig),
                });
            }
            // R14/R2: each arm advances its own copy of the move-state and its
            // own name set; the join reconciles the moves into `MaybeMoved`
            // wherever they disagree, and `leave_arm` drops each arm's locals
            // so the two name sets agree at the join.
            let before = scope.snapshot();
            let mut then_scope = scope.clone();
            let mut else_scope = scope.clone();
            let then_stack = poly_walk(
                then_branch,
                stack.clone(),
                &mut then_scope,
                sig,
                ctx,
                env,
                structs,
                enums,
                arrays,
            )?;
            let then_token = if else_span.is_some() { "else" } else { "end" };
            then_scope.leave_arm(&before, then_token, ctx, span, sig)?;
            let else_stack = poly_walk(
                else_branch,
                stack,
                &mut else_scope,
                sig,
                ctx,
                env,
                structs,
                enums,
                arrays,
            )?;
            else_scope.leave_arm(&before, "end", ctx, span, sig)?;
            scope.moves = Moves::join(then_scope.moves, else_scope.moves);
            if then_stack.len() != else_stack.len() {
                return Err(poly_branch_mismatch_error(
                    ctx,
                    span,
                    then_stack.len(),
                    else_stack.len(),
                ));
            }
            for (t_then, t_else) in then_stack.iter().zip(&else_stack) {
                if t_then != t_else {
                    return Err(poly_branch_type_mismatch_error(
                        ctx, span, t_then, t_else, sig,
                    ));
                }
            }
            stack = then_stack;
        }
        TermKind::Call(name) => {
            return poly_call_term(
                name, span, stack, scope, sig, ctx, env, structs, enums, arrays,
            );
        }
        // R5p: a quotation in a polymorphic body is rejected eagerly at the
        // literal. `poly_term`'s stack is `Vec<PolyType>`, not `Vec<Slot>`, so
        // there is nowhere to hang the `quot` marker, and D1 forbids a
        // `PolyType` variant; pushing a placeholder would erase the identity
        // into output unification/`Subst`/mangling. Mirrors the
        // `if`-in-a-polymorphic-body rejection above.
        TermKind::Quotation(_) => {
            return Err(format!(
                "error: a quotation in the polymorphic body of `{}` (line {}) is not yet supported",
                ctx.word_name().unwrap_or("<line>"),
                span.line
            ));
        }
    }
    Ok(stack)
}

#[allow(clippy::too_many_arguments)]
fn poly_call_term(
    name: &str,
    span: Span,
    mut stack: Vec<PolyType>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<Vec<PolyType>, String> {
    // A named local reads back its bound `PolyType`. A non-`Copy` local is
    // consumed on read (R3/D2): a second read is use-after-move, exactly as
    // the monomorphic checker treats a linear local; a `Copy` local carries no
    // such obligation and is absent from `moves`.
    if let Some(pt) = scope.locals.get(name).cloned() {
        scope
            .moves
            .take(name, span)
            .map_err(|site| poly_use_after_move_error(ctx, span, name, site))?;
        stack.push(pt);
        return Ok(stack);
    }
    let need = |n: usize, holds: usize| underflow_error(ctx, span, name, n, holds);
    // The five core shuffles move `PolyType` slots verbatim; `dup`/`over` gate
    // on `Copy` (a bare variable answers from its bound set, X7).
    match name {
        "dup" => {
            let top = stack.last().ok_or_else(|| need(1, stack.len()))?.clone();
            poly_copy_gate(&top, "dup", sig, ctx, span, structs, enums, arrays)?;
            stack.push(top);
            return Ok(stack);
        }
        "over" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(2, n));
            }
            let below = stack[n - 2].clone();
            poly_copy_gate(&below, "over", sig, ctx, span, structs, enums, arrays)?;
            stack.push(below);
            return Ok(stack);
        }
        "swap" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(2, n));
            }
            stack.swap(n - 1, n - 2);
            return Ok(stack);
        }
        "rot" => {
            let n = stack.len();
            if n < 3 {
                return Err(need(3, n));
            }
            let a = stack.remove(n - 3);
            stack.push(a);
            return Ok(stack);
        }
        "drop" => {
            stack.pop().ok_or_else(|| need(1, 0))?;
            return Ok(stack);
        }
        "len" => {
            let top = stack.last().ok_or_else(|| need(1, stack.len()))?;
            match top {
                PolyType::Array(..) | PolyType::Concrete(Type::Array(..)) => {
                    // Non-consuming: the array stays, `len` folds to `usize`.
                    stack.push(PolyType::Concrete(Type::Usize));
                }
                PolyType::Concrete(Type::Str) => {
                    stack.pop();
                    stack.push(PolyType::Concrete(Type::Usize));
                }
                _ => return Err(poly_op_on_variable_error(ctx, span, "len", top, sig)),
            }
            return Ok(stack);
        }
        _ => {}
    }
    // Comparisons: on a bare variable they need `Ord` (X8); on two concrete
    // operands they delegate to the ordinary operator check below.
    if matches!(name, "=" | "<" | ">" | "<=" | ">=" | "<>") {
        let n = stack.len();
        if n >= 2 {
            let a = stack[n - 2].clone();
            let b = stack[n - 1].clone();
            let (av, bv) = (poly_var_id(&a), poly_var_id(&b));
            if av.is_some() || bv.is_some() {
                match (av, bv) {
                    (Some(v), Some(w)) if v == w => {
                        if !sig.has_bound(v, Bound::Ord) {
                            return Err(poly_ord_body_error(
                                ctx,
                                span,
                                name,
                                &sig.ty_var_names[v as usize],
                            ));
                        }
                    }
                    _ => return Err(poly_op_operand_mismatch_error(ctx, span, name, &a, &b, sig)),
                }
                stack.truncate(n - 2);
                stack.push(PolyType::Concrete(Type::Bool));
                return Ok(stack);
            }
        }
    }
    // A monomorphic word: its concrete inputs must be met by concrete slots;
    // a bare variable passed to a concrete-typed argument is a located error.
    if let Some(msig) = env.get(name) {
        let n_in = msig.inputs.len();
        if stack.len() < n_in {
            return Err(need(n_in, stack.len()));
        }
        let base = stack.len() - n_in;
        for (i, inp) in msig.inputs.iter().enumerate() {
            match &stack[base + i] {
                PolyType::Concrete(t) if t == inp => {}
                PolyType::Var(v) => {
                    return Err(poly_var_to_concrete_error(
                        ctx,
                        span,
                        name,
                        &sig.ty_var_names[*v as usize],
                        *inp,
                    ));
                }
                other => {
                    return Err(poly_op_on_variable_error(ctx, span, name, other, sig));
                }
            }
        }
        stack.truncate(base);
        for out in &msig.outputs {
            stack.push(PolyType::Concrete(*out));
        }
        return Ok(stack);
    }
    // Everything else is an ordinary operator over concrete operands. Extract
    // the maximal concrete suffix, run the concrete check, reflect it back; a
    // variable operand (a too-short suffix) surfaces as the op's own error.
    if let Some(next) = poly_delegate_op(name, span, &mut stack, ctx)? {
        return Ok(next);
    }
    Err(unknown_word_error(ctx, span, name))
}

/// The variable id of a bare `PolyType::Var`, else `None` (a concrete or
/// array slot is not a bare variable).
fn poly_var_id(pt: &PolyType) -> Option<u32> {
    match pt {
        PolyType::Var(v) => Some(*v),
        _ => None,
    }
}

/// R7: `dup`/`over`'s `Copy` gate on a `PolyType` slot. A bare variable
/// missing the `Copy` bound is X7 (naming the variable and the missing bound,
/// with the linear-spine reason); a concrete linear slot reuses the ordinary
/// `cannot_copy` diagnostic.
#[allow(clippy::too_many_arguments)]
fn poly_copy_gate(
    pt: &PolyType,
    op: &str,
    sig: &PolySig,
    ctx: &Ctx,
    span: Span,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    if poly_is_copy(pt, sig, structs, enums, arrays) {
        return Ok(());
    }
    match pt {
        PolyType::Var(v) => Err(poly_copy_body_error(
            ctx,
            span,
            op,
            &sig.ty_var_names[*v as usize],
        )),
        PolyType::Concrete(t) => Err(cannot_copy_error(ctx, span, op, *t)),
        // A variable-bearing array is non-`Copy` exactly when its element is
        // (a length-variable array is never interned, so the declaration-time
        // `check_no_linear_array_elements` never sees it). Recurse so the
        // error names the real offending element, an unbounded variable or a
        // linear concrete type, never a fabricated one.
        PolyType::Array(elem, _) => {
            poly_copy_gate(elem, op, sig, ctx, span, structs, enums, arrays)
        }
        // Unreachable: `poly_is_copy` returns `true` for a quotation effect
        // (D3), so this gate returns above before reaching the error arm.
        PolyType::Quotation(..) => {
            unreachable!("a quotation effect is always Copy (D3); the gate returns above")
        }
    }
}

/// Delegate an operator whose operands are concrete: run it over the maximal
/// concrete suffix of the `PolyType` stack, then map the result back to
/// concrete slots. `None` if the name is not a concrete operator (the caller
/// then reports an unknown word).
fn poly_delegate_op(
    name: &str,
    span: Span,
    stack: &mut Vec<PolyType>,
    ctx: &Ctx,
) -> Result<Option<Vec<PolyType>>, String> {
    let mut split = stack.len();
    while split > 0 {
        if matches!(stack[split - 1], PolyType::Concrete(_)) {
            split -= 1;
        } else {
            break;
        }
    }
    let mut cstack: Vec<Slot> = stack[split..]
        .iter()
        .map(|pt| match pt {
            PolyType::Concrete(t) => Slot::computed(*t),
            _ => unreachable!("suffix is all concrete by construction"),
        })
        .collect();
    let handled = if let Some(s) = check_operator(name, span, &mut cstack, ctx)? {
        cstack = s;
        true
    } else if let Some(s) = check_str_word(name, span, &mut cstack, ctx)? {
        cstack = s;
        true
    } else {
        false
    };
    if !handled {
        return Ok(None);
    }
    stack.truncate(split);
    for slot in cstack {
        stack.push(PolyType::Concrete(slot.ty));
    }
    Ok(Some(std::mem::take(stack)))
}

/// R5/R6/R14: a call to a polymorphic word from a concrete body. Unifies the
/// word's `PolySig` against the concrete top of stack (deepest-first),
/// building the ground substitution `θ`, checks `θ` against the declared
/// bounds (X5/X6), records the per-call-site `CallInst` for lowering (R14),
/// and pushes the substituted concrete outputs. The row variable is a pure
/// pass-through: the stack beneath the fixed inputs is untouched, so `θ`
/// carries no row types and the word's ABI never sees the caller's deeper
/// stack (S2 rejected the carried runtime stack).
fn check_poly_call(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
    poly: &mut PolyCtx,
) -> Result<Vec<Slot>, String> {
    let (sig, generation) = poly
        .env
        .get(name)
        .expect("caller checked membership")
        .clone();
    let n_in = sig.inputs.len();
    if stack.len() < n_in {
        return Err(underflow_error(ctx, span, name, n_in, stack.len()));
    }
    let base = stack.len() - n_in;
    let mut subst = Subst::default();
    for i in 0..n_in {
        // R9p: `unify_poly_input` binds a `Var` to *any* concrete type, so a
        // quotation would silently bind `'T` to the placeholder and
        // monomorphize a call over a phantom. Reject before unification.
        if stack[base + i].quot.is_some() {
            return Err(reject_quotation_argument(ctx, span, name));
        }
        let slot_ty = stack[base + i].ty;
        unify_poly_input(
            &sig,
            &sig.inputs[i],
            slot_ty,
            name,
            span,
            ctx,
            arrays,
            &mut subst,
        )?;
    }
    // R6: each declared bound must hold of the concrete type `θ` bound the
    // variable to.
    for (v, bound) in &sig.bounds {
        let Some(ty) = subst.ty_of(*v) else { continue };
        let ok = match bound {
            Bound::Copy => is_copy(ty, ctx.structs(), ctx.enums(), arrays),
            Bound::Ord => is_ord(ty),
        };
        if !ok {
            let var = &sig.ty_var_names[*v as usize];
            return Err(match bound {
                Bound::Copy => poly_copy_bound_error(ctx, span, name, var, ty),
                Bound::Ord => poly_ord_bound_error(ctx, span, name, var, ty),
            });
        }
    }
    let mut outputs: Vec<Type> = Vec::with_capacity(sig.outputs.len());
    for pty in &sig.outputs {
        outputs.push(apply_subst(&sig, pty, &subst, name, span, ctx, arrays)?);
    }
    // R14: record the instantiation for lowering, keyed by the call-site span.
    // The bundle is filled later (a resolved output count >= 2 interns one).
    let symbol = instantiation_symbol(name, &subst, generation);
    poly.insts.insert(
        span,
        CallInst {
            callee: name.to_string(),
            subst,
            symbol,
            out_arity: outputs.len(),
            output_types: outputs.clone(),
            bundle: None,
            generation,
        },
    );
    stack.truncate(base);
    for ty in outputs {
        stack.push(Slot::computed(ty));
    }
    Ok(std::mem::take(stack))
}

/// R5: unify one declared input `PolyType` against a concrete slot type,
/// extending `subst`. A repeated variable forced to two concretes is X4; a
/// non-array where an array is declared, or a mismatched concrete, is the
/// ordinary type-mismatch error.
#[allow(clippy::too_many_arguments)]
fn unify_poly_input(
    sig: &PolySig,
    pty: &PolyType,
    slot_ty: Type,
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    subst: &mut Subst,
) -> Result<(), String> {
    match pty {
        PolyType::Concrete(t) => {
            if *t != slot_ty {
                return Err(type_mismatch_error(ctx, span, name, *t, slot_ty));
            }
        }
        PolyType::Var(v) => {
            if let Some(prev) = subst.ty_of(*v) {
                if prev != slot_ty {
                    return Err(poly_var_conflict_error(
                        ctx,
                        span,
                        name,
                        &sig.ty_var_names[*v as usize],
                        prev,
                        slot_ty,
                    ));
                }
            } else {
                subst.ty.push((*v, slot_ty));
            }
        }
        PolyType::Array(elem, len) => {
            let Type::Array(id, _) = slot_ty else {
                return Err(poly_array_expected_error(ctx, span, name, slot_ty));
            };
            let (elem_ty, count) = (arrays[id.index()].element, arrays[id.index()].count);
            unify_poly_input(sig, elem, elem_ty, name, span, ctx, arrays, subst)?;
            match len {
                Len::Concrete(k) => {
                    if *k != count {
                        return Err(poly_array_expected_error(ctx, span, name, slot_ty));
                    }
                }
                Len::Var(ln) => {
                    if let Some(prev) = subst.len_of(*ln) {
                        if prev != count {
                            return Err(poly_len_conflict_error(
                                ctx,
                                span,
                                name,
                                &sig.len_var_names[*ln as usize],
                                prev,
                                count,
                            ));
                        }
                    } else {
                        subst.len.push((*ln, count));
                    }
                }
            }
        }
        // Slice 6a (R6): a declared quotation parameter unifies against a
        // concrete quotation slot by matching rows pointwise, binding any
        // variable a row mentions (`[ 'T -- ]` against `[ i64 -- ]` binds
        // `'T = i64`). Equal arity is required on both sides; else it is a
        // located mismatch, never a silent bind.
        PolyType::Quotation(ins, outs) => {
            let Type::Quotation(eff) = slot_ty else {
                return Err(type_mismatch_error(
                    ctx,
                    span,
                    name,
                    crate::ast::quotation_type(Vec::new(), Vec::new()),
                    slot_ty,
                ));
            };
            if ins.len() != eff.inputs.len() || outs.len() != eff.outputs.len() {
                return Err(type_mismatch_error(
                    ctx,
                    span,
                    name,
                    poly_quotation_concrete_hint(ins, outs, subst),
                    slot_ty,
                ));
            }
            for (p, c) in ins.iter().zip(&eff.inputs) {
                unify_poly_input(sig, p, *c, name, span, ctx, arrays, subst)?;
            }
            for (p, c) in outs.iter().zip(&eff.outputs) {
                unify_poly_input(sig, p, *c, name, span, ctx, arrays, subst)?;
            }
        }
    }
    Ok(())
}

/// A best-effort concrete rendering of a declared quotation effect for an
/// arity-mismatch diagnostic: any already-bound variable is shown resolved,
/// an unbound one falls back to a nil-row placeholder so the message names a
/// real `Type`.
fn poly_quotation_concrete_hint(ins: &[PolyType], outs: &[PolyType], subst: &Subst) -> Type {
    let ground = |row: &[PolyType]| -> Vec<Type> {
        row.iter()
            .map(|p| match p {
                PolyType::Concrete(t) => *t,
                PolyType::Var(v) => subst.ty_of(*v).unwrap_or(Type::I64),
                _ => Type::I64,
            })
            .collect()
    };
    crate::ast::quotation_type(ground(ins), ground(outs))
}

/// R5: apply the ground `θ` to a declared output `PolyType`, yielding a
/// concrete `Type`. A variable-bearing array folds to a concrete interned
/// array shape. A variable the inputs never bound is an under-determined
/// signature (a located error rather than a panic).
fn apply_subst(
    sig: &PolySig,
    pty: &PolyType,
    subst: &Subst,
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
) -> Result<Type, String> {
    match pty {
        PolyType::Concrete(t) => Ok(*t),
        PolyType::Var(v) => subst.ty_of(*v).ok_or_else(|| {
            poly_unbound_output_error(ctx, span, name, &sig.ty_var_names[*v as usize])
        }),
        PolyType::Array(elem, len) => {
            let elem_ty = apply_subst(sig, elem, subst, name, span, ctx, arrays)?;
            let count = match len {
                Len::Concrete(k) => *k,
                Len::Var(ln) => subst.len_of(*ln).ok_or_else(|| {
                    poly_unbound_output_error(ctx, span, name, &sig.len_var_names[*ln as usize])
                })?,
            };
            Ok(intern_array_type(arrays, elem_ty, count))
        }
        // Slice 6a (R6): substitute both rows of a declared quotation effect,
        // yielding a concrete `Type::Quotation`.
        PolyType::Quotation(ins, outs) => {
            let mut cins = Vec::with_capacity(ins.len());
            for p in ins {
                cins.push(apply_subst(sig, p, subst, name, span, ctx, arrays)?);
            }
            let mut couts = Vec::with_capacity(outs.len());
            for p in outs {
                couts.push(apply_subst(sig, p, subst, name, span, ctx, arrays)?);
            }
            Ok(crate::ast::quotation_type(cins, couts))
        }
    }
}

/// R7 twin of `linear_local_unconsumed_error` for the polymorphic body
/// checker: a local bound to a non-`Copy` slot still holds its value at the
/// word's end. Names the local and its slot so the diagnostic matches the one
/// a concrete instantiation would already get from the monomorphic checker.
fn poly_local_unconsumed_error(
    word: &WordDef,
    sig: &PolySig,
    local: &str,
    pt: &PolyType,
) -> String {
    format!(
        "error: linear value `{}` is never consumed in `{}`\n  `{}` has type `{}`, which is linear: drop it or return it (nothing is dropped for you)",
        local,
        crate::resolve::demangle_word(&word.name),
        local,
        poly_type_str(pt, sig),
    )
}

/// R7 twin of `use_after_move_error` for the polymorphic body checker: a
/// non-`Copy` local read again after its first read (which consumed it),
/// citing the earlier read site.
fn poly_use_after_move_error(ctx: &Ctx, span: Span, local: &str, site: Span) -> String {
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: use after move in `{where_}` (line {})\n  local `{local}` is linear and was moved at line {}, col {}, so it is used exactly once",
        span.line, site.line, site.col,
    )
}

/// R14 twin of `branch_mismatch_error` for the polymorphic body checker: the
/// two `if` arms leave stacks of different depth. Takes `usize` depths, so no
/// `PolyType` argument is needed here.
fn poly_branch_mismatch_error(ctx: &Ctx, span: Span, d_then: usize, d_else: usize) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `if` branches leave different stack depths (then: {}, else: {})\n  note: declared {}",
            name, span.line, d_then, d_else, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `if` branches leave different stack depths (then: {d_then}, else: {d_else})"
        ),
    }
}

/// R14 twin of `branch_type_mismatch_error`: the two `if` arms leave differing
/// types in the same slot. Compares `PolyType`, which is why the monomorphic
/// `Type`-taking sibling cannot be reused; `sig` renders each variable's
/// surface spelling.
fn poly_branch_type_mismatch_error(
    ctx: &Ctx,
    span: Span,
    t_then: &PolyType,
    t_else: &PolyType,
    sig: &PolySig,
) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `if` branches leave different types (then: `{}`, else: `{}`)\n  note: declared {}",
            name,
            span.line,
            poly_type_str(t_then, sig),
            poly_type_str(t_else, sig),
            effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `if` branches leave different types (then: `{}`, else: `{}`)",
            poly_type_str(t_then, sig),
            poly_type_str(t_else, sig),
        ),
    }
}

/// R14/R2 twin of `linear_local_out_of_scope_error`: a non-`Copy` local bound
/// inside one `if` arm is never consumed before that arm ends. `token` names
/// the arm's closing keyword ("else" or "end").
fn poly_arm_local_unconsumed_error(
    ctx: &Ctx,
    span: Span,
    local: &str,
    pt: &PolyType,
    token: &str,
    sig: &PolySig,
) -> String {
    let where_ = ctx.word_name().unwrap_or("<line>");
    match ctx {
        Ctx::Word { .. } => format!(
            "error: linear value `{}` is never consumed in `{}` (line {})\n  local `{}` (`{}`) is never consumed, and its scope ends at the `{}` (nothing is dropped for you)",
            local,
            where_,
            span.line,
            local,
            poly_type_str(pt, sig),
            token,
        ),
        Ctx::Line { .. } => format!(
            "error: local `{}` (`{}`) is never consumed, and its scope ends at the `{}`",
            local,
            poly_type_str(pt, sig),
            token,
        ),
    }
}

fn poly_copy_body_error(ctx: &Ctx, span: Span, op: &str, var: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: cannot `{op}` the type variable `{var}` in `{where_}` (line {})\n  `{var}` has no `Copy` bound, and a linear value cannot be duplicated; declare `{var}: Copy` if every instantiation is `Copy`",
        span.line
    )
}

fn poly_ord_body_error(ctx: &Ctx, span: Span, op: &str, var: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{op}` on the type variable `{var}` in `{where_}` (line {}) requires an `Ord` bound\n  declare `{var}: Ord` so every instantiation is comparable",
        span.line
    )
}

fn poly_op_on_variable_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    pt: &PolyType,
    sig: &PolySig,
) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    let what = match pt {
        PolyType::Var(v) => format!("the type variable `{}`", sig.ty_var_names[*v as usize]),
        PolyType::Array(..) => "an array with a variable".to_string(),
        PolyType::Concrete(t) => format!("`{t}`"),
        PolyType::Quotation(..) => "a quotation".to_string(),
    };
    format!(
        "error: `{op}` is not permitted on {what} in `{where_}` (line {})",
        span.line
    )
}

fn poly_op_operand_mismatch_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    a: &PolyType,
    b: &PolyType,
    sig: &PolySig,
) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{op}` in `{where_}` (line {}) needs two operands of one type, found `{}` and `{}`",
        span.line,
        poly_type_str(a, sig),
        poly_type_str(b, sig),
    )
}

fn poly_var_to_concrete_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    expected: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{callee}` in `{where_}` (line {}) expects `{expected}`, but the type variable `{var}` is not a concrete type",
        span.line
    )
}

fn poly_output_mismatch_error(word: &WordDef, sig: &PolySig, residual: &[PolyType]) -> String {
    let got: Vec<String> = residual.iter().map(|pt| poly_type_str(pt, sig)).collect();
    let want: Vec<String> = sig
        .outputs
        .iter()
        .map(|pt| poly_type_str(pt, sig))
        .collect();
    format!(
        "error: stack effect mismatch in `{}`\n  body leaves `{}`, but the declared outputs are `{}`",
        crate::resolve::demangle_word(&word.name),
        got.join(" "),
        want.join(" "),
    )
}

fn poly_copy_bound_error(ctx: &Ctx, span: Span, callee: &str, var: &str, ty: Type) -> String {
    let callee = crate::resolve::demangle_call(callee);
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: cannot instantiate `{var}` of `{callee}` with `{ty}` in `{name}` (line {})\n  `{ty}` is linear and has no `Copy` instance, so a linear value cannot be duplicated; `{var}: Copy` is unsatisfied",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: cannot instantiate `{var}` of `{callee}` with linear type `{ty}`: `{var}: Copy` is unsatisfied"
        ),
    }
}

fn poly_ord_bound_error(ctx: &Ctx, span: Span, callee: &str, var: &str, ty: Type) -> String {
    let callee = crate::resolve::demangle_call(callee);
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: cannot instantiate `{var}` of `{callee}` with `{ty}` in `{name}` (line {})\n  `{ty}` is not `Ord`; `{var}: Ord` is unsatisfied",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: cannot instantiate `{var}` of `{callee}` with `{ty}`: `{var}: Ord` is unsatisfied"
        ),
    }
}

fn poly_var_conflict_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    a: Type,
    b: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let line = span.line;
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: `{callee}` in `{name}` (line {line}) resolved `{var}` to both `{a}` and `{b}`"
        ),
        Ctx::Line { .. } => {
            format!("error: `{callee}` resolved `{var}` to both `{a}` and `{b}`")
        }
    }
}

fn poly_len_conflict_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    a: u32,
    b: u32,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let line = span.line;
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: `{callee}` in `{name}` (line {line}) resolved length `{var}` to both `{a}` and `{b}`"
        ),
        Ctx::Line { .. } => {
            format!("error: `{callee}` resolved length `{var}` to both `{a}` and `{b}`")
        }
    }
}

fn poly_array_expected_error(ctx: &Ctx, span: Span, callee: &str, found: Type) -> String {
    let callee = crate::resolve::demangle_call(callee);
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: type mismatch in `{name}` (line {})\n  `{callee}` expected an array operand, found `{found}`",
            span.line
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{callee}` expected an array operand, found `{found}`")
        }
    }
}

fn poly_unbound_output_error(ctx: &Ctx, span: Span, callee: &str, var: &str) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{callee}` in `{where_}` (line {}) has output variable `{var}` that no input binds",
        span.line
    )
}

/// Render a `PolyType` for a diagnostic: a variable by its declared spelling,
/// a concrete type by its name, an array structurally.
pub(crate) fn poly_type_str(pt: &PolyType, sig: &PolySig) -> String {
    match pt {
        PolyType::Concrete(t) => t.name().to_string(),
        PolyType::Var(v) => sig.ty_var_names[*v as usize].clone(),
        PolyType::Array(elem, len) => {
            let l = match len {
                Len::Concrete(n) => n.to_string(),
                Len::Var(id) => sig.len_var_names[*id as usize].clone(),
            };
            format!("[{} {}]", poly_type_str(elem, sig), l)
        }
        PolyType::Quotation(ins, outs) => {
            let row = |r: &[PolyType]| {
                r.iter()
                    .map(|p| poly_type_str(p, sig))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let (i, o) = (row(ins), row(outs));
            match (i.is_empty(), o.is_empty()) {
                (true, true) => "[ -- ]".to_string(),
                (true, false) => format!("[ -- {o} ]"),
                (false, true) => format!("[ {i} -- ]"),
                (false, false) => format!("[ {i} -- {o} ]"),
            }
        }
    }
}

fn unknown_word_error(ctx: &Ctx, span: Span, name: &str) -> String {
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: unknown word `{}` in `{}` (line {})",
            name, wname, span.line
        ),
        Ctx::Line { .. } => format!("error: unknown word `{name}`"),
    }
}

fn underflow_error(ctx: &Ctx, span: Span, op: &str, needs: usize, holds: usize) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `{}` needs {} values, but the stack holds {}\n  note: declared {}",
            name, span.line, op, needs, holds, effect_str(effect),
        ),
        Ctx::Line { .. } => format!("error: stack underflow: needs {needs} values, but the stack holds {holds}"),
    }
}

/// R7: `str` -> `cstr` is an explicit word, never an implicit conversion; a
/// `str` where a `cstr` is wanted names the fix rather than a plain
/// mismatch, mirroring `size_conversion_needed_error`'s shape.
fn str_needs_cstr_conversion_error(ctx: &Ctx, span: Span, op: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` wants `cstr`, found `str`: convert it explicitly with `cstr` first (there is no implicit `str` -> `cstr` conversion)\n  note: declared {}",
            name, span.line, op, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` wants `cstr`, found `str`: convert it explicitly with `cstr` first"
        ),
    }
}

fn type_mismatch_error(ctx: &Ctx, span: Span, op: &str, expected: Type, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` expected `{}`, found `{}`\n  note: declared {}",
            name, span.line, op, expected, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` expected `{expected}`, found `{found}`")
        }
    }
}

/// Both-operand type mismatch for a homogeneous operator (`+ - * = < >`):
/// mixed int/float, mixed integer widths/signs, mixed float widths, or a
/// `bool` operand, name both operand types (X1, X2).
fn operand_pair_mismatch_error(ctx: &Ctx, span: Span, op: &str, a: Type, b: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires two operands of the same numeric type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, op, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` requires two operands of the same numeric type, found `{a}` and `{b}`"
        ),
    }
}

/// `/` applied to a non-float or mixed-float-type pair (X3): `/` is
/// float-only, integer division is unsupported.
fn div_requires_float_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `/` requires two operands of the same float type (integer division is unsupported), found `{}` and `{}`\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `/` requires two operands of the same float type (integer division is unsupported), found `{a}` and `{b}`"
        ),
    }
}

/// `mod` applied to a non-integer or mixed-integer-type pair (X4): `mod`
/// stays integer-only.
fn mod_requires_int_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `mod` requires two operands of the same integer type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `mod` requires two operands of the same integer type, found `{a}` and `{b}`"
        ),
    }
}

/// `max` applied to a float operand (X9): `max` is integer-only (D6);
/// naming `max-total` is the point of the message, not just the mismatch.
fn max_over_float_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `max` does not support float operands (found `{}` and `{}`); use `max-total` for a total-ordered float maximum\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `max` does not support float operands (found `{a}` and `{b}`); use `max-total` for a total-ordered float maximum"
        ),
    }
}

/// `max-total` applied to a non-float or mixed-float-type pair (X10):
/// `max-total` is float-only; naming `max` is the point of the message.
fn max_total_requires_float_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `max-total` requires two operands of the same float type, found `{}` and `{}`; use `max` for integers\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `max-total` requires two operands of the same float type, found `{a}` and `{b}`; use `max` for integers"
        ),
    }
}

/// `and`/`or`/`xor` applied to a non-integer/non-bool or mixed-type pair:
/// bitwise ops are homogeneous over the integer types and `bool`, same shape
/// as `mod_requires_int_error`.
fn bitwise_pair_mismatch_error(ctx: &Ctx, span: Span, op: &str, a: Type, b: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires two operands of the same integer or bool type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, op, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` requires two operands of the same integer or bool type, found `{a}` and `{b}`"
        ),
    }
}

/// `not` applied to a non-integer, non-bool operand.
fn bitwise_not_requires_int_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `not` requires an integer or bool operand, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `not` requires an integer or bool operand, found `{found}`"
        ),
    }
}

/// `shl`/`shr` applied to a non-integer value operand.
fn shift_value_requires_int_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an integer value operand, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` requires an integer value operand, found `{found}`"
        ),
    }
}

/// `shl`/`shr` applied to a shift count that is not `i64`.
fn shift_count_requires_i64_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an `i64` shift count, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` requires an `i64` shift count, found `{found}`"
        ),
    }
}

/// A conversion word (`>iN`/`>uN`/`>f32`/`>f64`) applied to a non-numeric
/// (`bool`) source (X5).
fn conversion_source_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires a numeric source, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` requires a numeric source, found `{found}`")
        }
    }
}

/// `.` applied to a non-printable value. Every current frontend `Type` (the
/// integer tower, the float tower, `bool`) is printable, so this path has no
/// reachable golden yet; it exists for the day a non-printable scalar (e.g. a
/// future `Ptr`) enters the type system.
fn print_requires_printable_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `.` requires a printable scalar, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `.` requires a printable scalar, found `{found}`")
        }
    }
}

/// `cstr` applied to something other than `str` (R7): the only legal source
/// for the discard-the-length conversion, so the error names it by name
/// rather than as a generic type mismatch.
fn cstr_conversion_source_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `cstr` converts a `str`, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `cstr` converts a `str`, found `{found}`")
        }
    }
}

/// R4 (D3): `dup`/`over` applied to a non-`Copy` value, in the DESIGN.md form.
/// A linear value has no bits to copy: the only ways to get a second one are to
/// thread this one through or to acquire another explicitly.
///
/// R4 (slice 8b): the linear cause names the `drop` overload when that is what
/// made the type linear. An all-`Copy`-fields resource struct told only that it
/// "has no bits to copy" points at nothing the reader can act on — its bits are
/// plainly copyable, and its own `: drop` declaration is the reason they may not
/// be.
fn cannot_copy_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    let defines_drop =
        matches!(found, Type::Struct(id, _) if ctx.structs()[id.index()].has_drop_overload);
    // A reference is neither `Copy` nor linear, so the ownership wording below
    // would tell the reader the opposite of the type rule.
    let why = if found.is_ref() {
        format!(
            "`{found}` is exclusive: at most one may be live for a place, so copying it would make a second one; use it where it is, or borrow again once it is consumed"
        )
    } else if defines_drop {
        format!(
            "`{found}` is linear because it defines `drop`: its own destructor runs exactly once, so a copy would run it twice; thread the value through instead"
        )
    } else {
        format!(
            "`{found}` is linear: it owns a resource and has no `Copy` instance, so there are no bits to copy; thread the value through instead"
        )
    };
    match ctx {
        Ctx::Word { name, effect, .. } => {
            format!(
            "error: cannot `{}` a value of type `{}` in `{}` (line {})\n  {}\n  note: declared {}",
            op, found, name, span.line, why, effect_str(effect),
        )
        }
        Ctx::Line { .. } => format!("error: cannot `{op}` a value of type `{found}`: {why}"),
    }
}

/// R3 (D2): a linear local mentioned again after its value was moved out, the
/// diagnostic naming the earlier move site.
fn use_after_move_error(ctx: &Ctx, span: Span, local: &str, ty: Type, site: Span) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: use after move in `{}` (line {})\n  local `{}` of type `{}` was moved at line {}, col {}; `{}` is linear, so it is used exactly once\n  note: declared {}",
            name, span.line, local, ty, site.line, site.col, ty, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: use after move: local `{local}` of type `{ty}` was moved at line {}, col {}",
            site.line, site.col
        ),
    }
}

/// R13/R14: a linear local still holding a value at the end of its scope,
/// either never mentioned or consumed on one branch only. Nothing is
/// auto-dropped, so this is an error rather than a compiler-inserted disposal.
fn linear_local_unconsumed_error(ctx: &Ctx, local: &str, ty: Type, line: u32) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear value `{}` is never consumed in `{}` (line {})\n  `{}` has type `{}`, which is linear: drop it or return it (nothing is dropped for you)\n  note: declared {}",
            local, name, line, local, ty, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear value `{local}` is never consumed (line {line})\n  `{local}` has type `{ty}`, which is linear: drop it or leave it on the stack (nothing is dropped for you)"
        ),
    }
}

/// R13/R14: a linear local consumed on one `if` arm but not the other. Unlike
/// `linear_local_unconsumed_error` (never touched at all), this local WAS
/// disposed on one path; the bug is the other arm forgetting it, so the
/// message points at the divergence rather than implying nothing happened.
fn linear_local_maybe_moved_error(ctx: &Ctx, local: &str, ty: Type, line: u32) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear value `{}` is not consumed on every path in `{}` (line {})\n  `{}` has type `{}`, which is linear: it is consumed on one `if` arm but not the other, so drop it (or return it) on every path\n  note: declared {}",
            local, name, line, local, ty, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear value `{local}` is not consumed on every path (line {line})\n  `{local}` has type `{ty}`, which is linear: it is consumed on one `if` arm but not the other, so drop it on every path"
        ),
    }
}

/// R6: a linear value bound inside a block and still holding its value when the
/// block ended. The word-end twins above can only cite the word; a block ends
/// at a token, so this one names it, because that token is where the value
/// became unreachable and the fix belongs before it.
fn linear_local_out_of_scope_error(
    ctx: &Ctx,
    local: &str,
    ty: Type,
    every_path: bool,
    token: &str,
    span: Span,
) -> String {
    let cause = match every_path {
        true => "is not consumed on every path",
        false => "is never consumed",
    };
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear value `{}` {} in `{}` (line {})\n  `{}` has type `{}`, which is linear, and its scope ends at the `{}` on line {}, col {}: consume it before then (nothing is dropped for you)\n  note: declared {}",
            local, cause, name, span.line, local, ty, token, span.line, span.col, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear value `{local}` {cause} (line {})\n  `{local}` has type `{ty}`, which is linear, and its scope ends at the `{token}` on line {}, col {}: consume it before then (nothing is dropped for you)",
            span.line, span.line, span.col,
        ),
    }
}

/// R13 (D7): a linear value left on the stack beyond the declared outputs. The
/// generic arity error (`check_outputs`) already rejects it, but a linear
/// surplus gets its own wording: the fix is disposal, not an extra output slot.
fn surplus_linear_value_error(word: &WordDef, ty: Type, line: u32) -> String {
    format!(
        "error: linear value left on the stack in `{}` (line {})\n  body leaves a `{}` beyond the {} declared output(s): a linear value must be consumed exactly once, so `drop` it or return it\n  note: declared {}",
        crate::resolve::demangle_word(&word.name),
        line,
        ty,
        word.effect.outputs.len(),
        effect_str(&word.effect),
    )
}

/// R15 (D8): a linear value live across the self-tail-call back-edge, which the
/// loop lowering would carry into the next iteration with nobody responsible
/// for disposing it. Deferred to a later Phase 3 slice, as a located error
/// rather than silence. Copy loops are untouched.
fn linear_across_back_edge_error(ctx: &Ctx, span: Span, callee: &str, ty: Type) -> String {
    let callee = crate::resolve::demangle_call(callee);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear values across a loop are not supported yet in `{}` (line {})\n  a `{}` is live across the self-tail-call back-edge to `{}`: consume it before the recursive call\n  note: declared {}",
            name, span.line, ty, callee, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear values across a loop are not supported yet: a `{ty}` is live across the back-edge to `{callee}`"
        ),
    }
}

/// A reference argument to a self-tail-call whose provenance traces to an
/// owned local of *this* frame — a `place` naming an actual
/// `Deriv::owned_root` — crosses a loop iteration boundary. Locals rebind at
/// the loop header (`carried_slots`), so the storage that local
/// named this iteration is not the storage the same name denotes next
/// iteration, and a reference into it would alias a reused slot. A reference
/// *parameter*, or one derived from it by projection, has no owned root
/// (`owned_root` is `None`, the accept-case) and may cross freely — its
/// referent lives in an ancestor frame that outlives every iteration, which is
/// what keeps `walk ( &!List -- ) ... walk ;` legal.
fn reference_across_back_edge_error(ctx: &Ctx, span: Span, callee: &str, place: &str) -> String {
    let callee = crate::resolve::demangle_call(callee);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: a reference to a local cannot cross a loop in `{}` (line {})\n  a reference derived from `{place}`, a local of this frame, crosses the self-tail-call back-edge to `{callee}`: that local's storage does not survive to the next iteration\n  note: declared {}",
            name, span.line, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: a reference to a local cannot cross a loop: a reference derived from `{place}` crosses the back-edge to `{callee}`"
        ),
    }
}

/// Reject a reference argument to the recursive call whose derivation's
/// owned root is a local of this frame. Scanned over the call's own arguments
/// (`args`, i.e. `stack[base..]` before the call truncates it) — the values
/// that actually cross the back-edge, as opposed to `check_linear_across_back_edge`'s
/// `below_args`, the values stranded beneath them.
fn check_reference_across_back_edge(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    args: &[Slot],
    prov: &Provenance,
) -> Result<(), String> {
    for slot in args {
        if let Some(id) = slot.deriv {
            if let Some(place) = &prov.deriv(id).owned_root {
                return Err(reference_across_back_edge_error(ctx, span, callee, place));
            }
        }
    }
    Ok(())
}

/// R15: reject a linear value that would survive the back-edge of a
/// self-tail-call, either stranded on the stack below the call's arguments or
/// held by a local that was never consumed. A value *moved into* the call's
/// arguments is forwarded, not live across the edge, so it stays legal.
fn check_linear_across_back_edge(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    below_args: &[Slot],
    scope: &Scope,
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    if let Some(slot) = below_args
        .iter()
        .find(|s| is_linear(s.ty, ctx.structs(), ctx.enums(), arrays))
    {
        return Err(linear_across_back_edge_error(ctx, span, callee, slot.ty));
    }
    if let Some(local) = scope.moves.unconsumed().first() {
        let ty = scope
            .local_type(local)
            .expect("a tracked local is in scope");
        return Err(linear_across_back_edge_error(ctx, span, callee, ty));
    }
    Ok(())
}

/// R4: a binding naming something already in scope. For a linear value the
/// rejection is forced (the earlier binding would become unreachable, and its
/// value could then never be consumed), and applying it to Copy values too
/// keeps one rule and one message instead of two.
/// `call` reached without a statically-known quotation literal on top (D4):
/// the value there is not traceable to a single literal.
fn call_needs_quotation_error(ctx: &Ctx, span: Span) -> String {
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: `call` in `{}` (line {}) expects a quotation on the stack (a quotation cannot be a runtime value; a runtime quotation value is slice 7)",
            name, span.line
        ),
        Ctx::Line { .. } => format!(
            "error: `call` (line {}) expects a quotation on the stack (a quotation cannot be a runtime value; a runtime quotation value is slice 7)",
            span.line
        ),
    }
}

/// R8: check a call of an *abstract* quotation (one typed only by a declared
/// `Type::Quotation` parameter, with no known literal body) against its
/// declared effect: consume `eff.inputs` deepest-first, then push
/// `eff.outputs`. No splice happens; the declared effect *is* the contract.
/// This is how a quotation-taking word's own body type-checks at its
/// definition site (D4), independent of any call site's literal.
fn check_abstract_quotation_call(
    eff: &QuotEffect,
    span: Span,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    op: &str,
) -> Result<Vec<Slot>, String> {
    let n = eff.inputs.len();
    if stack.len() < n {
        return Err(underflow_error(ctx, span, op, n, stack.len()));
    }
    let base = stack.len() - n;
    for (i, want) in eff.inputs.iter().enumerate() {
        let found = stack[base + i];
        match match_slot(found, *want) {
            SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
            _ => return Err(type_mismatch_error(ctx, span, op, *want, found.ty)),
        }
    }
    stack.truncate(base);
    for out in &eff.outputs {
        stack.push(Slot::computed(*out));
    }
    Ok(stack)
}

/// R9: check `f times` for an *abstract* quotation `f`. The count is already
/// verified as an `i64` by the caller path's guard below; here the declared
/// effect must be row-preserving with a trailing `i64` index
/// (`inputs == outputs ++ [i64]`), and the row on the stack is left unchanged.
fn check_abstract_quotation_times(
    eff: &QuotEffect,
    span: Span,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
) -> Result<Vec<Slot>, String> {
    let Some(count) = stack.pop() else {
        return Err(underflow_error(ctx, span, "times", 2, 1));
    };
    if count.quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, "times"));
    }
    if count.ty != Type::I64 {
        return Err(type_mismatch_error(ctx, span, "times", Type::I64, count.ty));
    }
    let row_preserving = eff.inputs.last() == Some(&Type::I64)
        && eff.inputs.len() == eff.outputs.len() + 1
        && eff.inputs[..eff.outputs.len()] == eff.outputs[..];
    if !row_preserving {
        return Err(times_body_row_effect_error(ctx, span));
    }
    let row_len = eff.outputs.len();
    if stack.len() < row_len {
        return Err(underflow_error(ctx, span, "times", row_len, stack.len()));
    }
    let base = stack.len() - row_len;
    for (i, want) in eff.outputs.iter().enumerate() {
        let found = stack[base + i];
        match match_slot(found, *want) {
            SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
            _ => return Err(type_mismatch_error(ctx, span, "times", *want, found.ty)),
        }
    }
    Ok(stack)
}

/// R18: gather the quotation-taking `WordBody::Terms` words, mono and poly
/// alike (`is_combinator` does not filter on `word.poly`), keyed by name, so a
/// call to one is intercepted and its body spliced (the inliner) rather than
/// lowered to a call to a word that mints no `IrFunc` (R20). `inline_combinator`
/// branches on `word.poly` internally to pick the mono or poly splice path.
fn collect_combinators(words: &[WordDef]) -> HashMap<String, Combinator<'_>> {
    let mut map = HashMap::new();
    for word in words {
        if !is_combinator(word) {
            continue;
        }
        if let WordBody::Terms { terms } = &word.body {
            map.insert(word.name.clone(), Combinator { word, terms });
        }
    }
    map
}

/// R2 (Slice 6c): the checker's inline view for one retained combinator, the
/// per-`WordDef` analogue of `collect_combinators`, so the REPL can project its
/// session store into the `HashMap<String, Combinator>` the inline path reads
/// without reaching into `Combinator`'s private fields. `None` for a
/// clause-bodied word (never a combinator: `is_combinator` requires
/// `WordBody::Terms`).
pub(crate) fn combinator_of(word: &WordDef) -> Option<Combinator<'_>> {
    match &word.body {
        WordBody::Terms { terms } => Some(Combinator { word, terms }),
        WordBody::Clauses(_) => None,
    }
}

/// R18/R20: a combinator is a **monomorphic** `WordBody::Terms` word with a
/// `Type::Quotation` input. The checker inlines a call to one (splicing its
/// body) and lowering mints no `IrFunc` for it, so `check` and `ir::lower`
/// must agree on the predicate exactly; it lives here as the single source.
/// Slice 6a phase 2: a **polymorphic** quotation-taking word (`each`/`map`/
/// `fold`) is a combinator too. It never monomorphizes to a standalone
/// `IrFunc` (R20); its body is spliced concretely at each call site, where the
/// element/length variables become the caller's concrete types, so the same
/// splice mechanism serves both the mono and poly cases (the poly signature
/// only drives the standalone def-site check, R17). The quotation parameter
/// sits in `sig.inputs` as either a variable-bearing `PolyType::Quotation` or,
/// when its effect is fully concrete, a `Concrete(Type::Quotation)`.
pub(crate) fn is_combinator(word: &WordDef) -> bool {
    matches!(word.body, WordBody::Terms { .. }) && word_declares_quotation_parameter(word)
}

/// R23 (D7): whether a word's declared effect names a quotation parameter,
/// regardless of body kind (a clause body is rejected separately by
/// `clause_bodied_quotation_word_error`, and a session never reaches a clause
/// body via `eval_def`/`eval_poly_def` at all -- this is the coarser gate the
/// REPL uses, since it cannot retain *any* quotation-taking word's body past
/// the defining line, term-body or not).
pub(crate) fn word_declares_quotation_parameter(word: &WordDef) -> bool {
    match &word.poly {
        None => word
            .effect
            .inputs
            .iter()
            .any(|s| matches!(s.ty, Type::Quotation(_))),
        Some(sig) => sig.inputs.iter().any(poly_input_is_quotation),
    }
}

/// A polymorphic input slot that declares a quotation parameter: either a
/// variable-bearing effect (`[ 'T -- ]`) or a fully-concrete one that folded
/// to `Concrete(Type::Quotation)`.
fn poly_input_is_quotation(p: &PolyType) -> bool {
    matches!(
        p,
        PolyType::Quotation(..) | PolyType::Concrete(Type::Quotation(_))
    )
}

/// R22 (D5)/R4 (D5 relaxed): reject a cycle in the quotation-taking-word call
/// subgraph. Edge `A -> B` iff combinator `A`'s body names combinator `B`
/// (any position; a call to a quotation-taking word necessarily passes it a
/// quotation). Since the inliner splices `B`'s body into `A`'s, a cycle would
/// inline forever, so unlike `check_tail_call_cycles` a self-edge is normally
/// the error.
///
/// R4 relaxes this for one shape only: a **self-tail** combinator, whose every
/// self-occurrence is in tail position, gets no self-edge, because the loop
/// transform lowers that self-call to a back-edge (a finite loop) rather than
/// re-splicing forever. A self-name in *any* non-tail position (`all_calls`
/// count exceeds `tail_position_calls` count) keeps its self-edge and stays a
/// cycle error, and every cycle of length >= 2 (a mutual cycle) is untouched.
/// Reuses `check_tail_call_cycles`'s 3-colour DFS shape (recon 8).
pub(crate) fn check_combinator_cycles(
    combinators: &HashMap<String, Combinator>,
) -> Result<(), String> {
    let members: Vec<&Combinator> = combinators.values().collect();
    let idx: HashMap<&str, usize> = members
        .iter()
        .enumerate()
        .map(|(i, c)| (c.word.name.as_str(), i))
        .collect();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); members.len()];
    for (i, c) in members.iter().enumerate() {
        let self_name = c.word.name.as_str();
        let self_all = all_calls(&c.word.body)
            .iter()
            .filter(|&&n| n == self_name)
            .count();
        let self_tail = tail_position_calls(&c.word.body)
            .iter()
            .filter(|&&n| n == self_name)
            .count();
        // R4: a tail-only self-edge (every self-occurrence in tail position,
        // and at least one) is permitted -- the loop transform makes it finite.
        let tail_only_self = self_all > 0 && self_all == self_tail;
        for callee in all_calls(&c.word.body) {
            if let Some(&j) = idx.get(callee) {
                if i == j && tail_only_self {
                    continue;
                }
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
    }
    let mut color = vec![0u8; members.len()];
    let mut path: Vec<usize> = Vec::new();
    for start in 0..members.len() {
        if color[start] == 0 {
            if let Some(cycle) = find_combinator_cycle(start, &adj, &mut color, &mut path) {
                return Err(combinator_cycle_error(&members, &cycle));
            }
        }
    }
    Ok(())
}

/// 3-colour DFS returning the members of the first cycle reached. Unlike
/// `find_tail_cycle`, a self-edge (`v == u`) is a cycle, not skipped.
fn find_combinator_cycle(
    u: usize,
    adj: &[Vec<usize>],
    color: &mut [u8],
    path: &mut Vec<usize>,
) -> Option<Vec<usize>> {
    color[u] = 1;
    path.push(u);
    for &v in &adj[u] {
        if color[v] == 1 {
            let start = path.iter().position(|&x| x == v).unwrap();
            return Some(path[start..].to_vec());
        }
        if color[v] == 0 {
            if let Some(cycle) = find_combinator_cycle(v, adj, color, path) {
                return Some(cycle);
            }
        }
    }
    path.pop();
    color[u] = 2;
    None
}

/// R22: a located cycle rejection naming the members in order and closing the
/// loop back to the first (`` `rec` -> `rec` `` for the self-recursive case).
fn combinator_cycle_error(members: &[&Combinator], cycle: &[usize]) -> String {
    let mut chain: Vec<&str> = cycle
        .iter()
        .map(|&i| crate::resolve::demangle_word(members[i].word.name.as_str()))
        .collect();
    chain.push(chain[0]);
    let rendered = chain
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(" -> ");
    let span = word_span(members[cycle[0]].word);
    format!(
        "error: a quotation-taking word cannot be recursive (the inliner would splice it forever): {} (line {}, col {})",
        rendered, span.line, span.col
    )
}

/// R18: inline a call to a monomorphic quotation-taking word. Validate each
/// declared input against the caller's live slot (a quotation parameter takes
/// a `Known` literal, checked directionally with the D3 capture check, R11/R12;
/// every other parameter is matched as usual), then splice the callee body
/// against the live stack (bracketed like a `call`, `tail = false`), so the
/// callee's own `call`/`times` fuse against the caller's literals. R22
/// guarantees termination.
#[allow(clippy::too_many_arguments)]
fn inline_combinator(
    comb: &Combinator,
    span: Span,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
) -> Result<Vec<Slot>, String> {
    let name = comb.word.name.as_str();
    // A polymorphic combinator (`each`/`map`/`fold`, or any `'T`-carrying
    // quotation-taking word) keeps its signature in `word.poly`, not
    // `word.effect` (which is empty), so the monomorphic argument loop below
    // would run zero checks and skip R11/R12 entirely (item 3). Route it
    // through the poly-argument check, which resolves the parameter's declared
    // effect against the live stack and runs the *same* directional + D3
    // check, so the two paths agree.
    if let Some(sig) = comb.word.poly.as_ref() {
        check_poly_combinator_args(
            sig, span, &stack, name, ctx, env, arrays, cells, refs, prov, scope, poly,
        )?;
    } else {
        let inputs: Vec<Type> = comb.word.effect.inputs.iter().map(|s| s.ty).collect();
        let n = inputs.len();
        if stack.len() < n {
            return Err(underflow_error(ctx, span, name, n, stack.len()));
        }
        let base = stack.len() - n;
        for (i, want) in inputs.iter().enumerate() {
            let found = stack[base + i];
            if let Type::Quotation(eff) = want {
                if let Some(QuotRef::Known(id)) = found.quot {
                    check_literal_against_declared_effect(
                        id, eff, name, span, ctx, env, arrays, cells, refs, prov, scope, poly,
                    )?;
                } else if matches!(found.ty, Type::Quotation(_)) {
                    // R21: forwarding an abstract quotation parameter. `found`
                    // is itself a declared quotation parameter of the enclosing
                    // combinator (a `Type::Quotation` slot with no `Known`
                    // literal -- the only way such a slot arises), reached only
                    // while checking that enclosing combinator standalone. At a
                    // real call site the substitution has already bound it to
                    // the caller's literal, so it carries a `Known` marker and
                    // splices there; here, at the def site, accept it when its
                    // declared effect matches the callee parameter, so `outer`
                    // may pass its own `f` to `inner`. The spliced callee
                    // body's own `f call`/`f times` then check the forwarded
                    // parameter against its declared effect (R8/R9).
                    if found.ty != *want {
                        return Err(quotation_argument_required_error(
                            ctx, span, name, *want, found.ty,
                        ));
                    }
                } else {
                    return Err(quotation_argument_required_error(
                        ctx, span, name, *want, found.ty,
                    ));
                }
            } else if found.quot.is_some() {
                return Err(reject_quotation_argument(ctx, span, name));
            } else {
                match match_slot(found, *want) {
                    SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
                    SlotMatch::NeedsSizeConversion => {
                        return Err(size_conversion_needed_error(ctx, span, name, *want));
                    }
                    SlotMatch::NeedsStrToCstrConversion => {
                        return Err(str_needs_cstr_conversion_error(ctx, span, name));
                    }
                    SlotMatch::Mismatch => {
                        return Err(type_mismatch_error(ctx, span, name, *want, found.ty));
                    }
                }
            }
        }
    }
    // R6: a self-tail combinator opens a splice-time loop. Its body is spliced
    // with `tail = true` so its own tail-position self-call is recognized as
    // the back-edge (above). 6d/R6: the nested-loop rejection is retired --
    // lowering's hoist-target split keeps a nested loop constant-stack -- so
    // opening this loop inside another is now legal and `splice_tail` is just
    // whether this is a self-tail combinator.
    let self_tail = crate::check::is_combinator(comb.word) && has_self_tail_call(comb.word);
    let splice_tail = self_tail;
    let input_count = match comb.word.poly.as_ref() {
        Some(sig) => sig.inputs.len(),
        None => comb.word.effect.inputs.len(),
    };
    // R18/R21: splice the callee body, alpha-renamed so its `| ... |` locals
    // cannot collide with a caller local or, under transitive inlining, an
    // outer combinator's locals already in scope. Lowering renames identically
    // (`ir`), so a passed-down literal's captured name stays lexical.
    let uid = prov.inline_uid;
    prov.inline_uid += 1;
    let renamed = crate::ast::alpha_rename_locals(comb.terms, uid);
    let depth = scope.depth();
    let saved_marker = if self_tail {
        let saved = prov.self_tail_combinator.take();
        prov.self_tail_combinator = Some(SelfTailMarker {
            name: name.to_string(),
            input_count,
        });
        Some(saved)
    } else {
        None
    };
    let result = check_terms(
        &renamed,
        stack,
        ctx,
        env,
        arrays,
        cells,
        refs,
        prov,
        scope,
        splice_tail,
        poly,
    );
    if let Some(saved) = saved_marker {
        prov.self_tail_combinator = saved;
    }
    stack = result?;
    leave_block(
        ctx,
        scope,
        depth,
        BlockEnd::Arm {
            token: "inline",
            span,
        },
    )?;
    Ok(stack)
}

/// R11/R12 (poly, item 3): the polymorphic twin of `inline_combinator`'s
/// monomorphic argument loop. A poly combinator's declared inputs live in
/// `sig.inputs`, not `word.effect`, so without this the directional (R11) and
/// D3 capture (R12) checks never ran on the poly argument path -- a caller
/// literal borrowing an enclosing place was silently accepted, a mono/poly
/// divergence in the premise D3 rests on. Resolve the parameter's declared
/// effect against the live stack (`unify_poly_input` binds any variable a
/// non-quotation input carries, e.g. `'T` in `['T ...] [ 'T -- &i64 ]`), then
/// ground the quotation effect and run the *same* `check_literal_against_
/// declared_effect` the monomorphic path uses, so the two agree.
#[allow(clippy::too_many_arguments)]
fn check_poly_combinator_args(
    sig: &PolySig,
    span: Span,
    stack: &[Slot],
    name: &str,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    let n = sig.inputs.len();
    if stack.len() < n {
        return Err(underflow_error(ctx, span, name, n, stack.len()));
    }
    let base = stack.len() - n;
    // Pass 1: unify the non-quotation inputs to resolve theta first, so a
    // variable a quotation effect mentions (`'T` in `[ 'T -- &i64 ]`) is
    // already bound when the effect is grounded in pass 2, whatever the
    // parameter order.
    let mut subst = Subst::default();
    for (i, pin) in sig.inputs.iter().enumerate() {
        if poly_input_is_quotation(pin) {
            continue;
        }
        let found = stack[base + i];
        if found.quot.is_some() {
            return Err(reject_quotation_argument(ctx, span, name));
        }
        unify_poly_input(sig, pin, found.ty, name, span, ctx, arrays, &mut subst)?;
    }
    // Pass 2: ground each quotation parameter and run the directional + D3
    // check on its caller literal.
    for (i, pin) in sig.inputs.iter().enumerate() {
        if !poly_input_is_quotation(pin) {
            continue;
        }
        let found = stack[base + i];
        let concrete = apply_subst(sig, pin, &subst, name, span, ctx, arrays)?;
        let Type::Quotation(eff) = concrete else {
            unreachable!("a quotation input grounds to Type::Quotation (apply_subst)")
        };
        if let Some(QuotRef::Known(id)) = found.quot {
            check_literal_against_declared_effect(
                id, eff, name, span, ctx, env, arrays, cells, refs, prov, scope, poly,
            )?;
        } else if matches!(found.ty, Type::Quotation(_)) {
            // R21 (poly): a forwarded abstract quotation parameter, accepted
            // when its declared effect matches (the spliced body's own
            // `call`/`times` re-checks it, R8/R9).
            if found.ty != concrete {
                return Err(quotation_argument_required_error(
                    ctx, span, name, concrete, found.ty,
                ));
            }
        } else {
            return Err(quotation_argument_required_error(
                ctx, span, name, concrete, found.ty,
            ));
        }
    }
    Ok(())
}

/// R11/R12: check a quotation *literal* against a declared quotation parameter
/// directionally (slice 4 D3): seed a fresh sub-stack with the declared input
/// row, run the literal's body against it, and require the exit row to equal
/// the declared output row (no standalone effect is inferred). Enforce the D3
/// capture restriction here (R12): a read that consumes a non-`Copy` enclosing
/// local, or a borrow of an enclosing place left on the row, is rejected; a
/// `Copy` local read by value is allowed.
#[allow(clippy::too_many_arguments)]
fn check_literal_against_declared_effect(
    id: QuotId,
    eff: &QuotEffect,
    word: &str,
    span: Span,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    let body = prov.quotations[id.0].body.clone();
    let outer_locals: HashSet<String> = scope.bound.iter().map(|b| b.name.clone()).collect();
    let moves_before = scope.moves.states.clone();
    let fresh: Vec<Slot> = eff.inputs.iter().map(|t| Slot::computed(*t)).collect();
    let depth = scope.depth();
    let result = check_terms(
        &body, fresh, ctx, env, arrays, cells, refs, prov, scope, false, poly,
    )?;
    // R12: a linear enclosing local the literal consumed (move-state changed
    // from `Live`).
    if let Some(local) =
        moves_before
            .iter()
            .find_map(|(n, before)| match (before, scope.moves.states.get(n)) {
                (MoveState::Live, Some(MoveState::Moved(_) | MoveState::MaybeMoved(_))) => {
                    Some(n.clone())
                }
                _ => None,
            })
    {
        return Err(quotation_captures_local_error(ctx, span, word, &local));
    }
    // R12: a borrow of an enclosing place left live on the literal's exit row.
    for slot in &result {
        if let Some(did) = slot.deriv {
            if let Some(place) = &prov.deriv(did).owned_root {
                if outer_locals.contains(place) {
                    return Err(quotation_borrows_place_error(ctx, span, word, place));
                }
            }
        }
    }
    leave_block(
        ctx,
        scope,
        depth,
        BlockEnd::Arm {
            token: "quotation",
            span,
        },
    )?;
    // R11: the literal's exit row must equal the declared output row.
    let matches_out = result.len() == eff.outputs.len()
        && result.iter().zip(&eff.outputs).all(|(f, w)| {
            matches!(
                match_slot(*f, *w),
                SlotMatch::Exact | SlotMatch::LiteralSizeType
            )
        });
    if !matches_out {
        let declared = crate::ast::quotation_type(eff.inputs.clone(), eff.outputs.clone());
        let actual =
            crate::ast::quotation_type(eff.inputs.clone(), result.iter().map(|s| s.ty).collect());
        return Err(literal_effect_mismatch_error(
            ctx, span, word, declared, actual,
        ));
    }
    Ok(())
}

/// R6 (Q1): does the quotation `body` read any name in `enclosing` that the
/// body does not itself bind? The cheap boolean the D3 materialization line
/// needs (no captures / captures), strictly less work than 7b's capture *set*.
/// Mirrors `alpha_rename_locals`'s walk (ast.rs): a `Call` strips a leading
/// `&!`/`&` exactly as `rename_call`, and a nested `TermKind::Quotation` / `if`
/// arm is walked carrying the body-bound names *by value*, so a read of an
/// outer name from inside a nested quotation still counts (D4's
/// capture-into-another-quotation case). Pure over the term tree: it inspects
/// no `Slot`/`Deriv` state, so it is testable in isolation.
fn body_captures_enclosing(body: &[Term], enclosing: &HashSet<String>) -> bool {
    fn walk(terms: &[Term], enclosing: &HashSet<String>, bound: &mut Vec<String>) -> bool {
        for term in terms {
            match &term.kind {
                TermKind::Bind(names) => {
                    for n in names {
                        bound.push(n.clone());
                    }
                }
                TermKind::Call(name) => {
                    let stripped = name
                        .strip_prefix("&!")
                        .or_else(|| name.strip_prefix('&'))
                        .unwrap_or(name);
                    if enclosing.contains(stripped) && !bound.iter().any(|b| b == stripped) {
                        return true;
                    }
                }
                TermKind::Quotation(inner) => {
                    let mut inner_bound = bound.clone();
                    if walk(inner, enclosing, &mut inner_bound) {
                        return true;
                    }
                }
                TermKind::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    let mut tb = bound.clone();
                    let mut eb = bound.clone();
                    if walk(then_branch, enclosing, &mut tb) {
                        return true;
                    }
                    if walk(else_branch, enclosing, &mut eb) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    walk(body, enclosing, &mut Vec::new())
}

/// R12/D4: a capturing quotation reaching a materialization boundary. 7a
/// materializes only non-capturing literals; a capturing one is a located
/// rejection naming 7b, reusing the escaping-quotation vocabulary shape.
/// `boundary` is one of the three wordings this project's boundaries produce:
/// `be stored` (a constructor/setter argument, or a `!`/`+!` store through a
/// reference -- the latter covers both a struct field and an array element
/// via reference alike, since a reference carries no record of what it was
/// borrowed from), `be returned`, `be left on a branch`.
fn capturing_quotation_error(ctx: &Ctx, span: Span, boundary: &str) -> String {
    let _ = ctx;
    format!(
        "error: a capturing quotation cannot {boundary} (capturing closures are slice 7b) (line {})",
        span.line,
    )
}

/// R7/D4: a materialization boundary. Materialize a non-capturing `Known`
/// literal into a runtime quotation value, or reject a capturing one naming 7b
/// (R12). (i) run the boolean capture predicate (R6); (ii) if it captures,
/// raise R12 with `boundary`'s wording; (iii) else confirm the literal against
/// the boundary's expected `Type::Quotation(eff)` via
/// `check_literal_against_declared_effect`, and return the slot *erased*
/// (`quot: None`, a real `Type::Quotation`) -- the signal `call`/`times` read
/// to emit an indirect call rather than a splice.
#[allow(clippy::too_many_arguments)]
fn materialize_quotation_at_boundary(
    id: QuotId,
    eff: &'static QuotEffect,
    boundary: &str,
    word: &str,
    span: Span,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
) -> Result<Slot, String> {
    let enclosing: HashSet<String> = scope.bound.iter().map(|b| b.name.clone()).collect();
    let body = prov.quotations[id.0].body.clone();
    if body_captures_enclosing(&body, &enclosing) {
        return Err(capturing_quotation_error(ctx, span, boundary));
    }
    check_literal_against_declared_effect(
        id, eff, word, span, ctx, env, arrays, cells, refs, prov, scope, poly,
    )?;
    Ok(Slot::computed(Type::Quotation(eff)))
}

/// R10/R21: a quotation parameter position whose argument is not a quotation
/// the callee can consume -- a non-quotation value, or (after R21 admits the
/// abstract forward) a quotation whose *declared effect* disagrees with the
/// callee parameter. Knownness is no longer the complaint: a forwarded abstract
/// parameter is accepted, so `want` and `found` always differ here (a
/// non-quotation type, or a mismatched effect), and the message names both.
fn quotation_argument_required_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    want: Type,
    found: Type,
) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: `{word}` expects a quotation `{want}` here, found `{found}`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R11: a quotation literal whose effect disagrees with the declared
/// parameter. Names the word, the declared effect, and the literal's actual
/// effect.
fn literal_effect_mismatch_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    declared: Type,
    actual: Type,
) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: the quotation passed to `{word}` was declared `{declared}` but its body has effect `{actual}`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R12: a quotation literal that consumes a linear enclosing local (D3 forbids
/// a linear capture). Names the local and the enclosing word.
fn quotation_captures_local_error(ctx: &Ctx, span: Span, word: &str, local: &str) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: the quotation passed to `{word}` consumes the enclosing local `{local}`, which is linear; a quotation may only read a `Copy` enclosing local by value (D3){} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R12: a quotation literal that borrows an enclosing place and leaves the
/// reference on its row (D3 forbids capturing an enclosing borrow).
fn quotation_borrows_place_error(ctx: &Ctx, span: Span, word: &str, place: &str) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: the quotation passed to `{word}` borrows the enclosing place `{place}`; a quotation may not capture a borrow of an enclosing local (D3){} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R18: `times` reached without a statically-known quotation literal on top
/// (D4). Parallel to `call_needs_quotation_error`.
fn times_needs_quotation_error(ctx: &Ctx, span: Span) -> String {
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: `times` in `{}` (line {}) expects a quotation on the stack (a quotation cannot be a runtime value; a runtime quotation value is slice 7)",
            name, span.line
        ),
        Ctx::Line { .. } => format!(
            "error: `times` (line {}) expects a quotation on the stack (a quotation cannot be a runtime value; a runtime quotation value is slice 7)",
            span.line
        ),
    }
}

/// R18: the body is spliced once but runs N times, so a linear outer local it
/// consumes would be disposed of more than once. The single most important
/// `times` checker rule.
fn times_body_consumes_local_error(ctx: &Ctx, span: Span, name: &str) -> String {
    format!(
        "error: a `times` body cannot consume `{name}`{} (line {}): the body runs more than once, so the value would be disposed of more than once",
        in_word(ctx),
        span.line,
    )
}

/// R18: a reference the body derives would cross the back-edge into the next
/// iteration. A borrow is idempotent per iteration, so a well-formed body
/// leaves `live_derivs` unchanged; this fires when it does not.
fn times_body_borrow_across_loop_error(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: a `times` body cannot leave a reference live across the loop{} (line {}): the local it borrows does not survive to the next iteration",
        in_word(ctx),
        span.line,
    )
}

/// R18/D6: the body's net effect on the row is not identity -- it must consume
/// the index and return the row it received unchanged.
fn times_body_row_effect_error(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: a `times` body must leave the row unchanged{} (line {}): it takes `( ..s i64 -- ..s )`, consuming the index and returning the same row",
        in_word(ctx),
        span.line,
    )
}

fn rebound_local_error(ctx: &Ctx, span: Span, name: &str) -> String {
    let scope_end = "a name may not be re-bound while it is in scope: the earlier binding would become unreachable, and a linear value in it could then never be consumed";
    match ctx {
        Ctx::Word { name: word, .. } => format!(
            "error: `{name}` is already bound in `{word}` (line {}, col {})\n  {scope_end}",
            span.line, span.col
        ),
        Ctx::Line { .. } => format!(
            "error: `{name}` is already bound (line {}, col {})\n  {scope_end}",
            span.line, span.col
        ),
    }
}

/// R2/R6: take every name bound past `depth` out of scope, and reject a linear
/// value still held when the block ended. Nothing is auto-dropped, so leaving a
/// block is one more place where forgetting a value is *caught*, never a place
/// where one is disposed for you.
fn leave_block(ctx: &Ctx, scope: &mut Scope, depth: usize, at: BlockEnd) -> Result<(), String> {
    let Some((local, ty, state)) = scope.leave(depth) else {
        return Ok(());
    };
    let every_path = matches!(state, MoveState::MaybeMoved(_));
    Err(match at {
        BlockEnd::Body(line) if every_path => linear_local_maybe_moved_error(ctx, &local, ty, line),
        BlockEnd::Body(line) => linear_local_unconsumed_error(ctx, &local, ty, line),
        BlockEnd::Arm { token, span } => {
            linear_local_out_of_scope_error(ctx, &local, ty, every_path, token, span)
        }
    })
}

/// A `usize`/`isize` position (a binary operator's other operand, a
/// word-call argument, or a declared output) fed a *computed* (non-literal)
/// `i64` (X10): unlike a bare integer literal, a computed value doesn't
/// silently coerce, since Sooth has no comptime interpreter to fold it and
/// confirm it fits; names the missing `>usize`/`>isize` conversion
/// explicitly, naming whichever size type `target` is.
fn size_conversion_needed_error(ctx: &Ctx, span: Span, op: &str, target: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` mixes `{}` with a computed `i64`: convert it explicitly with `>{}` first (a bare integer literal coerces automatically, a computed value does not)\n  note: declared {}",
            name, span.line, op, target, target, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` mixes `{target}` with a computed `i64`: convert it explicitly with `>{target}` first"
        ),
    }
}

/// An unknown type name in a conversion word (X6), e.g. `>i128`.
fn conversion_unknown_type_error(ctx: &Ctx, span: Span, name: &str) -> String {
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: unknown type `{name}` in `{wname}` (line {})",
            span.line
        ),
        Ctx::Line { .. } => format!("error: unknown type `{name}`"),
    }
}

fn branch_mismatch_error(ctx: &Ctx, span: Span, d_then: usize, d_else: usize) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `if` branches leave different stack depths (then: {}, else: {})\n  note: declared {}",
            name, span.line, d_then, d_else, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `if` branches leave different stack depths (then: {d_then}, else: {d_else})"
        ),
    }
}

fn branch_type_mismatch_error(ctx: &Ctx, span: Span, t_then: Type, t_else: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `if` branches leave different types (then: `{}`, else: `{}`)\n  note: declared {}",
            name, span.line, t_then, t_else, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `if` branches leave different types (then: `{t_then}`, else: `{t_else}`)"
        ),
    }
}

/// The borrow-suspension bookkeeping must agree at a branch join, real
/// content the type-only shape unification above does not supply. One arm
/// suspending a place the other leaves unsuspended (or suspending a
/// *different* place) is rejected rather than silently picking one arm's
/// answer, since a later hazard check would then reason about the wrong arm's
/// runtime path.
fn borrow_join_disagreement_error(
    ctx: &Ctx,
    span: Span,
    t_then: Option<&Deriv>,
    t_else: Option<&Deriv>,
) -> String {
    let describe = |d: Option<&Deriv>| match d.map(Deriv::suspension) {
        None => "no live borrow".to_string(),
        Some((Some(root), Some(place))) => {
            format!("a borrow of `{root}` reborrowed from `{place}`")
        }
        Some((Some(root), None)) => format!("a borrow of `{root}`"),
        Some((None, Some(place))) => format!("a reborrow of `{place}`"),
        Some((None, None)) => "a borrow with no local root".to_string(),
    };
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: borrow state disagrees at the `if`/`else` join in `{}` (line {})\n  the `then` arm leaves {}, the `else` arm leaves {}: both arms must agree on which place, if any, stays borrowed past the join\n  note: declared {}",
            name, span.line, describe(t_then), describe(t_else), effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: borrow state disagrees at the `if`/`else` join (line {})\n  the `then` arm leaves {}, the `else` arm leaves {}",
            span.line, describe(t_then), describe(t_else),
        ),
    }
}

/// Walk a term sequence. `scope` is the names in scope and the move-state of
/// the linear ones, mutated in place as terms bind and mention names; `tail`
/// marks the sequence as
/// occupying its word's tail position, so its final term (and, recursively,
/// both arms of a final `if`) sits on the self-tail-call back-edge. The rule
/// mirrors `tail_position_calls`/`lower_terms`; all three must stay in
/// lockstep.
#[allow(clippy::too_many_arguments)]
fn check_terms(
    terms: &[Term],
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    tail: bool,
    poly: &mut PolyCtx,
) -> Result<Vec<Slot>, String> {
    let last = terms.len().wrapping_sub(1);
    for (i, term) in terms.iter().enumerate() {
        stack = check_term(
            term,
            stack,
            ctx,
            env,
            arrays,
            cells,
            refs,
            prov,
            scope,
            tail && i == last,
            poly,
        )?;
    }
    Ok(stack)
}

#[allow(clippy::too_many_arguments)]
fn check_term(
    term: &Term,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    tail: bool,
    poly: &mut PolyCtx,
) -> Result<Vec<Slot>, String> {
    let span = term.span;
    match &term.kind {
        TermKind::IntLit(n) => {
            // A bare integer literal is the one D8 source: fresh off the
            // term, it may still silently fill a `usize` position. Its value
            // is retained for the compile-time-count array positions (M1, X4).
            stack.push(Slot {
                ty: Type::I64,
                literal: true,
                int_val: Some(*n),
                alias: None,
                deriv: None,
                quot: None,
            });
            Ok(stack)
        }
        TermKind::FloatLit(_) => {
            stack.push(Slot::computed(Type::F64));
            Ok(stack)
        }
        TermKind::BoolLit(_) => {
            stack.push(Slot::computed(Type::Bool));
            Ok(stack)
        }
        TermKind::StrLit(_) => {
            stack.push(Slot::computed(Type::Str));
            Ok(stack)
        }
        TermKind::Bind(names) => {
            // R1: pop one value per name at this point, leftmost name deepest,
            // the same shape whether this is the entry binding or a mid-body
            // one. R5: the frame floor is whatever the stack holds here, which
            // in a word body is its declared inputs and nothing beneath them.
            let mut seen = HashSet::new();
            for name in names {
                reject_variant_local(ctx, name, "local")?;
                reject_duplicate_local(ctx, name, span, &mut seen)?;
                if scope.local_type(name).is_some() {
                    return Err(rebound_local_error(ctx, span, name));
                }
            }
            if stack.len() < names.len() {
                let op = format!("| {} |", names.join(" "));
                return Err(underflow_error(ctx, span, &op, names.len(), stack.len()));
            }
            let bound = stack.split_off(stack.len() - names.len());
            for (name, slot) in names.iter().zip(bound) {
                let linear = is_linear(slot.ty, ctx.structs(), ctx.enums(), arrays);
                scope.bind(name, slot, linear, prov);
            }
            Ok(stack)
        }
        TermKind::Call(name) => {
            if let Some(binding) = scope.local(name) {
                let (ty, aliases, held, quot) =
                    (binding.ty, binding.aliases, binding.deriv, binding.quot);
                match ref_parts(ty, refs) {
                    // Naming a reference local is a reborrow, not a move.
                    // A mutable one suspends its place: a second reborrow while
                    // anything derived from the first is still live would be two
                    // live mutable references into one place.
                    Some((_, mutable)) => {
                        if mutable {
                            if let Some(id) =
                                live_deriv(&stack, scope, prov, |d| d.reborrow && d.place == *name)
                            {
                                return Err(suspended_place_error(ctx, span, name, prov.deriv(id)));
                            }
                        }
                        let deriv = prov.reborrow(name, held, mutable, span);
                        stack.push(Slot::derived(ty, Some(deriv)));
                    }
                    None => {
                        // Consuming a place while a reference derived from
                        // it is live would leave that reference aimed at storage
                        // its owner has given away. Only a linear local is
                        // consumed by being named; a Copy one is merely read.
                        if is_linear(ty, ctx.structs(), ctx.enums(), arrays) {
                            if let Some(id) = live_borrow_of(&stack, scope, prov, name) {
                                return Err(consume_of_borrowed_place_error(
                                    ctx,
                                    span,
                                    name,
                                    ty,
                                    prov.deriv(id),
                                ));
                            }
                        }
                        // Mentioning a linear local moves its value
                        // out; a second mention names the site that already
                        // consumed it.
                        if let Err(site) = scope.moves.take(name, span) {
                            return Err(use_after_move_error(ctx, span, name, ty, site));
                        }
                        // The direction symmetric with the check at the
                        // borrow: this naming would be the *second* name for
                        // storage a live `&!` already reaches, so the mutation
                        // is just as silently observable as if the naming had
                        // come first. Only an aggregate has a region, and so
                        // only an aggregate can be a second name for one.
                        if aliases.is_some() {
                            if let Some(id) = live_mutable_borrow_of(&stack, scope, prov, name) {
                                return Err(naming_aliases_borrowed_place_error(
                                    ctx,
                                    span,
                                    name,
                                    prov.deriv(id),
                                ));
                            }
                        }
                        // Naming an aggregate does not copy it, so the
                        // pushed value denotes the local's own region, located
                        // here so a later borrow can point at this naming.
                        stack.push(Slot {
                            alias: aliases.map(|set| Alias { set, span }),
                            quot,
                            ..Slot::computed(ty)
                        });
                    }
                }
                return Ok(stack);
            }
            // R6: `call`/`times` are compiler-known words intercepted before
            // every builtin family and user-word lookup (a local named `call`
            // already won above). `call` requires a statically-known
            // quotation literal on top (D4) and splices its interned body
            // against the live stack, so `[ 1 + ] call` checks as `1 +` (D3).
            if name == "call" {
                let Some(top) = stack.pop() else {
                    return Err(underflow_error(ctx, span, "call", 1, 0));
                };
                // R8: an *abstract* quotation (typed by a declared parameter,
                // `Slot.ty == Type::Quotation`, no `Known` literal) checks
                // against its declared effect directly: pop `eff.inputs`
                // deepest-first, push `eff.outputs`. This is the standalone
                // (def-site) check of a quotation-taking word (D4): `f call`
                // checks against `f`'s declared effect exactly as an ordinary
                // word call checks against its `Sig`, with no splice.
                if top.quot.is_none() {
                    if let Type::Quotation(eff) = top.ty {
                        return check_abstract_quotation_call(eff, span, stack, ctx, "call");
                    }
                    return Err(call_needs_quotation_error(ctx, span));
                }
                let Some(QuotRef::Known(id)) = top.quot else {
                    return Err(call_needs_quotation_error(ctx, span));
                };
                // Splice the body against the current locals/scope in lexical
                // extent (capture is free, recon 9), bracketed like an `if`
                // arm so a body that binds does not leak past the `call` and a
                // linear value bound inside it is caught by `leave_block`
                // (R6). `tail` is pinned `false`: lowering emits a real call
                // here, never a self-tail back-edge (R6/R13).
                let body = prov.quotations[id.0].body.clone();
                let depth = scope.depth();
                stack = check_terms(
                    &body, stack, ctx, env, arrays, cells, refs, prov, scope, false, poly,
                )?;
                leave_block(
                    ctx,
                    scope,
                    depth,
                    BlockEnd::Arm {
                        token: "call",
                        span,
                    },
                )?;
                return Ok(stack);
            }
            // R18: `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )`, the
            // constant-stack loop primitive. Intercepted alongside `call`. The
            // body is spliced against the row plus a synthesized index and must
            // return the row unchanged (D6); nested `times` is rejected here,
            // not in lowering, which has no error channel (R14 step 0).
            if name == "times" {
                let Some(top) = stack.pop() else {
                    return Err(underflow_error(ctx, span, "times", 2, 0));
                };
                // R9: an *abstract* quotation (a declared parameter, no known
                // literal) checks against its declared effect: pop the count,
                // require the effect be row-preserving with a trailing `i64`
                // index (`[ ..row i64 -- ..row ]`), and leave the row
                // unchanged. The three `times` obligations reduce to checks on
                // the declared rows (a declared effect names no local and
                // captures no borrow, so move/borrow identity hold trivially).
                if top.quot.is_none() {
                    if let Type::Quotation(eff) = top.ty {
                        return check_abstract_quotation_times(eff, span, stack, ctx);
                    }
                    return Err(times_needs_quotation_error(ctx, span));
                }
                let Some(QuotRef::Known(id)) = top.quot else {
                    return Err(times_needs_quotation_error(ctx, span));
                };
                let Some(count) = stack.pop() else {
                    return Err(underflow_error(ctx, span, "times", 2, 1));
                };
                // The count is also a type-directed read, so a quotation there
                // is the default-deny wording, not a `Cstr`-placeholder mismatch.
                if count.quot.is_some() {
                    return Err(reject_quotation_operand(ctx, span, "times"));
                }
                if count.ty != Type::I64 {
                    return Err(type_mismatch_error(ctx, span, "times", Type::I64, count.ty));
                }
                // 6d/R6: a `times` nested in a loop -- inside a self-tail word,
                // another `times` body, or a self-tail combinator body -- is no
                // longer rejected. Lowering's hoist-target split (R1-R3) keeps
                // every such nesting constant-stack, so the old R18/R14b
                // rejection and its `loop_depth` bookkeeping are gone.
                // R18: the row is the remaining stack; a quotation anywhere in
                // it would reach `begin_loop`'s phi over a phantom (R14). Guard
                // the whole row, not just the consumed top.
                if stack.iter().any(|s| s.quot.is_some()) {
                    return Err(reject_quotation_operand(ctx, span, "times"));
                }
                // R18: the body is spliced once but runs N times, so it must be
                // identity on the move/borrow state (clone-and-compare), or a
                // linear local it consumes would be disposed N times. Snapshot
                // before the splice; `leave_block` drops the body's own
                // bindings, so what remains changed is an *outer* local.
                let moves_before = scope.moves.states.clone();
                let derivs_before: HashSet<DerivId> = live_derivs(&stack, scope).collect();
                let row = stack.clone();
                // Splice the body against the row plus a synthesized index (the
                // body's top input), bracketed like `call` (R6), `tail = false`.
                // 6d/R6: a `times` nested in the body is now legal, so no
                // `loop_depth` is raised across the splice.
                stack.push(Slot::computed(Type::I64));
                let body = prov.quotations[id.0].body.clone();
                let depth = scope.depth();
                let result = check_terms(
                    &body, stack, ctx, env, arrays, cells, refs, prov, scope, false, poly,
                )?;
                leave_block(
                    ctx,
                    scope,
                    depth,
                    BlockEnd::Arm {
                        token: "times",
                        span,
                    },
                )?;
                // R18: identity on the move state. A body's own bindings are
                // already gone (`leave_block`), so a local left `Moved`/
                // `MaybeMoved` where it was `Live` is an outer linear local the
                // body consumed; name the first such one.
                if let Some(local) = moves_before.iter().find_map(|(n, before)| {
                    match (before, scope.moves.states.get(n)) {
                        (MoveState::Live, Some(MoveState::Moved(_) | MoveState::MaybeMoved(_))) => {
                            Some(n.clone())
                        }
                        _ => None,
                    }
                }) {
                    return Err(times_body_consumes_local_error(ctx, span, &local));
                }
                // R18: identity on the borrow state. A borrow is idempotent per
                // iteration, so a well-formed body leaves `live_derivs`
                // unchanged; a difference means a reference would cross the
                // back-edge into the next iteration.
                let derivs_after: HashSet<DerivId> = live_derivs(&result, scope).collect();
                if derivs_after != derivs_before {
                    return Err(times_body_borrow_across_loop_error(ctx, span));
                }
                // D6: the body's net effect on the row must equal the row.
                let same_shape = row.len() == result.len()
                    && result.iter().zip(&row).all(|(found, want)| {
                        matches!(
                            match_slot(*found, want.ty),
                            SlotMatch::Exact | SlotMatch::LiteralSizeType
                        )
                    });
                if !same_shape {
                    return Err(times_body_row_effect_error(ctx, span));
                }
                // R18: the whole-row guard runs on the *entry* row, but a body
                // that consumes a real value and constructs a quotation into
                // its place leaves a phantom in the output row that `match_slot`
                // accepts as `Exact` against the `Cstr` placeholder. That
                // phantom would be carried into the loop's back-edge phis, so
                // reject it here with the same whole-row wording.
                if result.iter().any(|s| s.quot.is_some()) {
                    return Err(reject_quotation_operand(ctx, span, "times"));
                }
                stack = result;
                return Ok(stack);
            }
            if let Some(stack) = check_reference_word(
                name, span, &mut stack, ctx, scope, arrays, cells, refs, prov,
            )? {
                return Ok(stack);
            }
            // R8 (D4): `!`/`+!` into a `&!Type::Quotation` referent is a
            // materialization boundary (an array element or a struct field via
            // reference). Materialize a non-capturing `Known` literal in place
            // before `check_access_word` (whose bare-quotation store guard would
            // else reject it), reject a capturing one naming 7b (R12). The
            // referent's declared effect is the boundary's expected effect.
            if matches!(name.as_str(), "!" | "+!") && stack.len() >= 2 {
                let vi = stack.len() - 1;
                if let Some(QuotRef::Known(id)) = stack[vi].quot {
                    if let Some((Type::Quotation(eff), _)) = ref_parts(stack[vi - 1].ty, refs) {
                        let qspan = prov.quotations[id.0].span;
                        stack[vi] = materialize_quotation_at_boundary(
                            id,
                            eff,
                            "be stored",
                            name,
                            qspan,
                            ctx,
                            env,
                            arrays,
                            cells,
                            refs,
                            prov,
                            scope,
                            poly,
                        )?;
                    }
                }
            }
            if let Some(stack) = check_access_word(name, span, &mut stack, ctx, arrays, refs)? {
                return Ok(stack);
            }
            if let Some(stack) = check_shuffle(name, span, &mut stack, ctx, arrays, prov)? {
                return Ok(stack);
            }
            if let Some(stack) = check_operator(name, span, &mut stack, ctx)? {
                return Ok(stack);
            }
            if let Some(stack) = check_str_word(name, span, &mut stack, ctx)? {
                return Ok(stack);
            }
            if let Some(stack) = check_array_word(name, span, &mut stack, ctx, arrays)? {
                return Ok(stack);
            }
            if let Some(stack) = check_owned_cell_word(name, span, &mut stack, ctx, arrays, cells)?
            {
                return Ok(stack);
            }
            if let Some(stack) = check_struct_peek_word(name, span, &mut stack, ctx, arrays, prov)?
            {
                return Ok(stack);
            }
            if let Some(stack) = check_struct_get_word(name, span, &mut stack, ctx, prov)? {
                return Ok(stack);
            }
            // R6-R9: a tail-position call, inside a self-tail combinator
            // body splice, to that same combinator is the loop back-edge, not
            // a re-splice (which would recurse forever). Intercepted before
            // the combinator dispatch below. It discharges the two
            // move/borrow obligations at the self-call (the stack-row identity
            // obligation is left to the ordinary stack-effect and `if`-join
            // discipline, R7) and produces the combinator's carried state --
            // its non-quotation inputs, which for a self-tail combinator are
            // exactly its declared outputs -- then terminates this branch. A
            // non-tail self-call never reaches here: R4 rejected it at
            // `check_combinator_cycles` before any splice.
            let back_edge = tail
                && prov
                    .self_tail_combinator
                    .as_ref()
                    .is_some_and(|m| m.name == *name);
            if back_edge {
                let n = prov
                    .self_tail_combinator
                    .as_ref()
                    .expect("back-edge marker set")
                    .input_count;
                if stack.len() < n {
                    return Err(underflow_error(ctx, span, name, n, stack.len()));
                }
                let base = stack.len() - n;
                // R8: no linear value live across the edge (below the args, or
                // an unconsumed frame local).
                check_linear_across_back_edge(ctx, span, name, &stack[..base], scope, arrays)?;
                // R9: no reference into a frame local carried by the args.
                check_reference_across_back_edge(ctx, span, name, &stack[base..], prov)?;
                // The carried state is the non-quotation inputs (a concrete
                // literal quotation carries `quot`, a def-site abstract one is
                // `Type::Quotation`); both are dropped, exactly as the loop
                // carries no quotation phantom in its phis (R10).
                let outs: Vec<Slot> = stack[base..]
                    .iter()
                    .filter(|s| s.quot.is_none() && !matches!(s.ty, Type::Quotation(_)))
                    .map(|s| Slot::computed(s.ty))
                    .collect();
                stack.truncate(base);
                stack.extend(outs);
                return Ok(stack);
            }
            // R18: a call to a monomorphic quotation-taking word is inlined
            // (term-splice) rather than looked up in `env` and lowered to a
            // call: it mints no `IrFunc` (R20). Copy the `Combinator` out of
            // the borrowed map first (it is two pointers) so `poly` can be
            // reborrowed mutably for the splice.
            if let Some(comb) = poly.combinators.get(name).copied() {
                return inline_combinator(
                    &comb, span, stack, ctx, env, arrays, cells, refs, prov, scope, poly,
                );
            }
            // R5/R14: a call to a polymorphic word is intercepted before the
            // concrete `env` lookup and unified against the concrete stack;
            // its `Sig` is per-instantiation, not name-keyed.
            if poly.env.contains_key(name) {
                return check_poly_call(name, span, &mut stack, ctx, arrays, poly);
            }
            let sig = env
                .get(name)
                .ok_or_else(|| unknown_word_error(ctx, span, name))?;
            let n = sig.inputs.len();
            if stack.len() < n {
                return Err(underflow_error(ctx, span, name, n, stack.len()));
            }
            let base = stack.len() - n;
            for (i, want) in sig.inputs.iter().enumerate() {
                let found = stack[base + i];
                // R8 (D4): a declared `Type::Quotation` parameter is a
                // materialization boundary. This is the site a struct
                // *constructor* call (`[ 1 + ] Holder`) and a generated setter
                // reach; a non-capturing `Known` literal is materialized
                // (validated here, lowered to a `(code, env)` value), a
                // capturing one rejected naming 7b (R12). Gated strictly on
                // `want`'s type, so it covers a constructor, a setter, and an
                // ordinary user word declaring a quotation parameter alike; an
                // `extern` never reaches here (its declared effect cannot name
                // a `Type::Quotation`, rejected at declaration).
                if let Type::Quotation(eff) = *want {
                    if let Some(QuotRef::Known(id)) = found.quot {
                        stack[base + i] = materialize_quotation_at_boundary(
                            id,
                            eff,
                            "be stored",
                            name,
                            span,
                            ctx,
                            env,
                            arrays,
                            cells,
                            refs,
                            prov,
                            scope,
                            poly,
                        )?;
                        continue;
                    }
                    // An already-erased runtime quotation value falls through
                    // to the ordinary `match_slot` (Exact) below.
                }
                // R9: a quotation argument rejects before ordinary unification,
                // so the message names the word rather than mismatching the
                // `Cstr` placeholder. Also covers generated struct
                // constructors/setters and `extern` args (all `env` words).
                if found.quot.is_some() {
                    return Err(reject_quotation_argument(ctx, span, name));
                }
                match match_slot(found, *want) {
                    SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
                    SlotMatch::NeedsSizeConversion => {
                        return Err(size_conversion_needed_error(ctx, span, name, *want));
                    }
                    SlotMatch::NeedsStrToCstrConversion => {
                        return Err(str_needs_cstr_conversion_error(ctx, span, name));
                    }
                    SlotMatch::Mismatch => {
                        return Err(type_mismatch_error(ctx, span, name, *want, found.ty));
                    }
                }
            }
            if tail && ctx.mangled_name() == Some(name.as_str()) {
                check_linear_across_back_edge(ctx, span, name, &stack[..base], scope, arrays)?;
                check_reference_across_back_edge(ctx, span, name, &stack[base..], prov)?;
            }
            stack.truncate(base);
            stack.extend(sig.outputs.iter().map(|ty| Slot::computed(*ty)));
            Ok(stack)
        }
        TermKind::If {
            then_branch,
            else_branch,
            else_span,
            end_span,
        } => {
            let cond = stack
                .pop()
                .ok_or_else(|| underflow_error(ctx, span, "if", 1, 0))?;
            // R11: guard before the `Bool` mismatch, or the generic message
            // names the `Cstr` placeholder instead of the `if` condition.
            if cond.quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, "if"));
            }
            if cond.ty != Type::Bool {
                return Err(type_mismatch_error(ctx, span, "if", Type::Bool, cond.ty));
            }
            // R14: each arm advances its own copy of the move-state; the join
            // reconciles them into `MaybeMoved` wherever they disagree. R2:
            // each arm is also a block, so a name it binds is gone by the join
            // and the two arms' name sets agree there again.
            let depth = scope.depth();
            let mut then_scope = scope.clone();
            let mut else_scope = scope.clone();
            let then_stack = check_terms(
                then_branch,
                stack.clone(),
                ctx,
                env,
                arrays,
                cells,
                refs,
                prov,
                &mut then_scope,
                tail,
                poly,
            )?;
            let (then_token, then_at) = match else_span {
                Some(at) => ("else", *at),
                None => ("end", *end_span),
            };
            leave_block(
                ctx,
                &mut then_scope,
                depth,
                BlockEnd::Arm {
                    token: then_token,
                    span: then_at,
                },
            )?;
            let else_stack = check_terms(
                else_branch,
                stack,
                ctx,
                env,
                arrays,
                cells,
                refs,
                prov,
                &mut else_scope,
                tail,
                poly,
            )?;
            leave_block(
                ctx,
                &mut else_scope,
                depth,
                BlockEnd::Arm {
                    token: "end",
                    span: *end_span,
                },
            )?;
            scope.moves = Moves::join(then_scope.moves, else_scope.moves);
            if then_stack.len() != else_stack.len() {
                return Err(branch_mismatch_error(
                    ctx,
                    span,
                    then_stack.len(),
                    else_stack.len(),
                ));
            }
            let mut merged = Vec::with_capacity(then_stack.len());
            for (i, (t_then, t_else)) in then_stack.iter().zip(&else_stack).enumerate() {
                // R7/R11: a branch merge cannot carry a quotation whose
                // identity is ambiguous *unless* the enclosing context declares
                // its type, in which case the join materializes each arm into a
                // runtime `(code, env)` value (D4). Two arms carrying the
                // *same* literal stay a forwarded marker (`lower_if`'s `t == e`
                // fast path emits no `Phi`, splice preserved). The `Cstr`
                // placeholder makes an arm's real `Cstr` compare equal to a
                // quotation, so the ordinary `ty` mismatch below never catches
                // the one-quotation shape; this guard has both phrasings.
                let (quot, erased_ty) = match (t_then.quot, t_else.quot) {
                    (None, None) => (None, None),
                    (Some(QuotRef::Known(a)), Some(QuotRef::Known(b))) if a == b => {
                        (Some(QuotRef::Known(a)), None)
                    }
                    (Some(QuotRef::Known(a)), Some(QuotRef::Known(b))) => {
                        // R11 ordering pin: the capture check runs before the
                        // id/expected-type resolution, so a capturing arm always
                        // raises R12 rather than falling through to
                        // `different_quotations_at_join_error`.
                        let enclosing: HashSet<String> =
                            scope.bound.iter().map(|bnd| bnd.name.clone()).collect();
                        for id in [a, b] {
                            let body = prov.quotations[id.0].body.clone();
                            if body_captures_enclosing(&body, &enclosing) {
                                return Err(capturing_quotation_error(
                                    ctx,
                                    prov.quotations[id.0].span,
                                    "be left on a branch",
                                ));
                            }
                        }
                        // The expected quotation type threaded from the
                        // enclosing declared context: at a word-body tail the
                        // merged slot maps to the declared output at index `i`.
                        // Without one the join cannot give the erased value a
                        // type, so it stays a located error.
                        let expected = if tail {
                            ctx.declared_outputs()
                                .and_then(|outs| outs.get(i))
                                .map(|slot| slot.ty)
                        } else {
                            None
                        };
                        match expected {
                            Some(Type::Quotation(eff)) => {
                                let word = ctx.word_name().unwrap_or("the branch");
                                let a_span = prov.quotations[a.0].span;
                                let b_span = prov.quotations[b.0].span;
                                check_literal_against_declared_effect(
                                    a, eff, word, a_span, ctx, env, arrays, cells, refs, prov,
                                    scope, poly,
                                )?;
                                check_literal_against_declared_effect(
                                    b, eff, word, b_span, ctx, env, arrays, cells, refs, prov,
                                    scope, poly,
                                )?;
                                // Erased: a runtime `(code, env)` value with a
                                // real `Type::Quotation`, no `Known` marker.
                                (None, Some(Type::Quotation(eff)))
                            }
                            _ => return Err(different_quotations_at_join_error(ctx, span)),
                        }
                    }
                    _ => return Err(quotation_versus_value_at_join_error(ctx, span)),
                };
                if erased_ty.is_none() && t_then.ty != t_else.ty {
                    return Err(branch_type_mismatch_error(ctx, span, t_then.ty, t_else.ty));
                }
                // The type-only join above already rejects two arms whose
                // stacks disagree in shape; it says nothing about *which place*
                // a live reference's suspension is attributed to. Two arms of
                // identical shape can each suspend a different place (one
                // derives from local `x`, the other from `y`), which the merge
                // must reject rather than silently pick one arm's answer — a
                // later hazard check would then reason about the wrong arm's
                // runtime path. A place is either arm's owned root or the
                // reference local a mutable reborrow suspends: two arms
                // reborrowing *different* reference parameters have no owned
                // root at all and still disagree.
                let deriv = match (t_then.deriv, t_else.deriv) {
                    (None, None) => None,
                    (Some(a), Some(b))
                        if prov.deriv(a).suspension() == prov.deriv(b).suspension() =>
                    {
                        Some(a)
                    }
                    _ => {
                        return Err(borrow_join_disagreement_error(
                            ctx,
                            span,
                            t_then.deriv.map(|id| prov.deriv(id)),
                            t_else.deriv.map(|id| prov.deriv(id)),
                        ));
                    }
                };
                // A merged slot is a coercible literal only if *both* arms
                // leave a literal there: a value computed on either runtime
                // path is computed after the merge, so it can't silently fill
                // a `usize`/`isize` position without an explicit conversion
                // (D8/X10).
                // Keep every region either arm could have left, since the merge
                // cannot know which one ran: dropping one would let a later
                // borrow of a name bound to the merge mutate storage a live name
                // still denotes on the path that was dropped.
                let alias = match (t_then.alias, t_else.alias) {
                    (None, None) => None,
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (Some(a), Some(b)) => Some(Alias {
                        set: prov.alias_union(a.set, b.set),
                        span: a.span,
                    }),
                };
                merged.push(Slot {
                    // R11: a materialized join slot carries the declared
                    // quotation type in place of the arms' `Cstr` placeholder.
                    ty: erased_ty.unwrap_or(t_then.ty),
                    literal: t_then.literal && t_else.literal,
                    // A value merged from two branches is never a single
                    // known literal, so it can't feed a compile-time count.
                    int_val: None,
                    alias,
                    deriv,
                    // R7: only a marker both arms agree on survives the join.
                    quot,
                });
            }
            Ok(merged)
        }
        // R5: a quotation literal interns its body into the side table and
        // pushes a compile-time-only marker (D1/D2). The body is *not* checked
        // here (D3): a bare body's input row is unknown until its consumption
        // site (`call`/`times`). The placeholder `ty` is `Cstr`, a
        // registry-free scalar no user op accepts once R11's default-deny is
        // in place (R4).
        TermKind::Quotation(body) => {
            let id = QuotId(prov.quotations.len());
            prov.quotations.push(QuotBody {
                body: body.clone(),
                span,
            });
            stack.push(Slot {
                quot: Some(QuotRef::Known(id)),
                ..Slot::computed(Type::Cstr)
            });
            Ok(stack)
        }
    }
}

/// Apply an arithmetic/comparison/conversion operator if `name` is one,
/// returning `Some(stack)`; `None` if the name is none of those (the caller
/// then looks it up in the env). `+ - *` are homogeneous over the numeric
/// types (int or float, `bool` is never numeric): both operands must be the
/// *same* type, producing that type; no implicit promotion (R6). `/` is
/// float-only: both operands must be the same float type (R7). `mod` stays
/// integer-only: both operands must be the same integer type (R8). `= < >`
/// generalise the same way as `+ - *` but always produce `bool` (R9). A
/// conversion word is `>` followed by a known numeric type name
/// (`>i8`..`>u64`, `>f32`, `>f64`): pop one numeric value, push the named
/// target (R10). `and`/`or`/`xor` are homogeneous over the integer types and
/// `bool` (float is rejected), same shape as `mod`; on two `bool`s they *are*
/// logical and/or/xor, since a stack language evaluates both operands eagerly
/// so bitwise-on-0/1 and logical coincide. `not` is unary: integer or `bool`
/// in, same type out (int stays bitwise complement, `bool` is logical
/// negation; the difference is only in how `lower_call` codegens it).
/// `shl`/`shr` take an integer value and always an `i64` shift count,
/// producing the value's type. `<= >= <>` generalise the same way as `= < >`:
/// numeric-only (never `bool`), same type, producing `bool`. `.` is
/// type-directed over any printable scalar (every integer width, either
/// float width, or `bool`): pops one, produces nothing; the concrete type
/// picks the print codegen (signed/unsigned decimal, `%g` float, or
/// `true`/`false`) at the call site, same dispatch shape as the rest of this
/// function.
fn check_operator(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
) -> Result<Option<Vec<Slot>>, String> {
    // R11: every operator this function handles reads the top slot, so a
    // quotation on top is always an operand of it. Guard once, gated on the
    // name being one we handle (else fall through so a later dispatcher can
    // claim it), before the type-directed reads that would otherwise spell the
    // `Cstr` placeholder into a mismatch.
    let is_operator = matches!(
        name,
        "+" | "-"
            | "*"
            | "/"
            | "mod"
            | "and"
            | "or"
            | "xor"
            | "not"
            | "shl"
            | "shr"
            | "="
            | "<"
            | ">"
            | "<="
            | ">="
            | "<>"
            | "max"
            | "max-total"
            | "."
    ) || name.strip_prefix('>').is_some_and(|r| !r.is_empty());
    // The unary members (`not`, print, the `>T` conversions) read only the
    // top; every other operator reads a pair, so its deeper operand at
    // `stack[n - 2]` is an operand of it too. Guarding the top alone lets a
    // quotation there fall through to `operand_pair_mismatch_error`, which
    // spells the `Cstr` placeholder into the message the audit exists to keep
    // hidden.
    let is_unary =
        matches!(name, "not" | ".") || name.strip_prefix('>').is_some_and(|r| !r.is_empty());
    if is_operator && stack.last().is_some_and(|s| s.quot.is_some()) {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    if is_operator && !is_unary && stack.len() >= 2 && stack[stack.len() - 2].quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    // Unify a homogeneous binary op's operand pair, honoring D8's literal
    // coercion (`Ok`); `Err(Some(target))` is the size-type/computed-`i64`
    // X10 case, naming which size type (`usize`/`isize`) needed the explicit
    // conversion; `Err(None)` is a plain mismatch the caller reports with its
    // own op-specific diagnostic.
    let unify = |a: Slot, b: Slot| -> Result<Type, Option<Type>> {
        match unify_pair(a, b) {
            PairMatch::Ok(ty) => Ok(ty),
            PairMatch::NeedsSizeConversion(target) => Err(Some(target)),
            PairMatch::Mismatch => Err(None),
        }
    };
    match name {
        "+" | "-" | "*" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_numeric() || !b.ty.is_numeric() {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        "/" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_float() || !b.ty.is_float() || a.ty != b.ty {
                return Err(div_requires_float_error(ctx, span, a.ty, b.ty));
            }
            stack.truncate(n - 2);
            stack.push(Slot::computed(a.ty));
        }
        "mod" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_int() || !b.ty.is_int() {
                return Err(mod_requires_int_error(ctx, span, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => mod_requires_int_error(ctx, span, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        "and" | "or" | "xor" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !(a.ty.is_int() || a.ty.is_bool()) || !(b.ty.is_int() || b.ty.is_bool()) {
                return Err(bitwise_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => bitwise_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        "not" => {
            let n = stack.len();
            if n < 1 {
                return Err(need(name, 1, n));
            }
            let a = stack[n - 1];
            if !(a.ty.is_int() || a.ty.is_bool()) {
                return Err(bitwise_not_requires_int_error(ctx, span, a.ty));
            }
        }
        "shl" | "shr" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_int() {
                return Err(shift_value_requires_int_error(ctx, span, name, a.ty));
            }
            if b.ty != Type::I64 {
                return Err(shift_count_requires_i64_error(ctx, span, name, b.ty));
            }
            stack.truncate(n - 2);
            stack.push(Slot::computed(a.ty));
        }
        "=" | "<" | ">" | "<=" | ">=" | "<>" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_numeric() || !b.ty.is_numeric() {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(Type::Bool));
        }
        // R12 (S6): `max ( 'T 'T -- 'T )`, an internal `Ord` bound resolved
        // against the integer tower (`is_int`, which already includes
        // `usize`/`isize`, D7). A float pair is rejected by name (X9),
        // directing to `max-total` (R13) rather than pretending IEEE `>` is
        // total (D6); the pair must still agree on one concrete type exactly
        // like `+`/`>`.
        "max" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if a.ty.is_float() || b.ty.is_float() {
                return Err(max_over_float_error(ctx, span, a.ty, b.ty));
            }
            if !a.ty.is_int() || !b.ty.is_int() {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        // R13 (S6): `max-total ( 'F 'F -- 'F )`, `f32`/`f64` only, ordered by
        // the `total_cmp` bit-pattern rule rather than IEEE `>` (D6). An
        // integer pair is rejected by name (X10), directing to `max`.
        "max-total" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_float() || !b.ty.is_float() {
                return Err(max_total_requires_float_error(ctx, span, a.ty, b.ty));
            }
            if a.ty != b.ty {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            stack.truncate(n - 2);
            stack.push(Slot::computed(a.ty));
        }
        "." => {
            let n = stack.len();
            if n < 1 {
                return Err(need(".", 1, n));
            }
            let a = stack[n - 1];
            if !a.ty.is_numeric() && !a.ty.is_bool() && !matches!(a.ty, Type::Str | Type::Cstr) {
                return Err(print_requires_printable_error(ctx, span, a.ty));
            }
            stack.truncate(n - 1);
        }
        _ => {
            let Some(rest) = name.strip_prefix('>').filter(|r| !r.is_empty()) else {
                return Ok(None);
            };
            let target = match Type::from_name(rest) {
                Some(ty) if ty.is_numeric() => ty,
                _ => return Err(conversion_unknown_type_error(ctx, span, rest)),
            };
            let source = *stack.last().ok_or_else(|| need(name, 1, stack.len()))?;
            if !source.ty.is_numeric() {
                return Err(conversion_source_error(ctx, span, name, source.ty));
            }
            stack.pop();
            stack.push(Slot::computed(target));
        }
    }
    Ok(Some(std::mem::take(stack)))
}

/// An array word (`fill`/`len`) applied to a non-array operand: names the
/// array word and the offending operand type (X8).
fn array_word_operand_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an array operand, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` requires an array operand, found `{found}`")
        }
    }
}

/// `S|>fi` (R10) applied to a linear field: unlike `S>fi`, a peek must leave
/// the aggregate live, so it can't also transfer ownership of a linear
/// field's value; the workaround is `S>` (destructure the whole aggregate).
fn peek_of_linear_field_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: cannot `{}` a linear field in `{}` (line {})\n  the field has type `{}`, which is linear and has no `Copy` instance, so it cannot be peeked without consuming the aggregate; use `S>` to destructure instead\n  note: declared {}",
            op, name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: cannot `{op}` a linear field: the field has type `{found}`, which is linear and has no `Copy` instance"
        ),
    }
}

/// An owning-cell word (`^>`/`^|>`) applied to a non-cell operand: names the
/// word and the offending operand type, mirroring `array_word_operand_error`.
fn owned_cell_word_operand_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an owning-cell operand, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` requires an owning-cell operand, found `{found}`")
        }
    }
}

/// `^|>` on a linear payload: the cell stays live afterward, so peeking
/// would leave a second, unowned reference to a resource the cell still
/// owns. `^>` (consuming unwrap) is the workaround.
fn peek_of_linear_owned_payload_error(
    ctx: &Ctx,
    span: Span,
    cell_ty: Type,
    payload: Type,
) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: cannot `^|>` a linear payload in `{}` (line {})\n  `{}` holds a payload of type `{}`, which is linear and has no `Copy` instance, so it cannot be peeked without consuming the cell; use `^>` to unwrap instead\n  note: declared {}",
            name, span.line, cell_ty, payload, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: cannot `^|>` a linear payload: `{cell_ty}` holds a payload of type `{payload}`, which is linear and has no `Copy` instance"
        ),
    }
}

/// A constant (literal) index out of range for a `[T N]` (X4, R11): a compile
/// error naming the length `N` and the offending index.
fn array_index_out_of_range_error(ctx: &Ctx, span: Span, count: u32, index: i64) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: array index out of range in `{}` (line {})\n  index {} is out of bounds for length {}\n  note: declared {}",
            name, span.line, index, count, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: array index out of range: index {index} is out of bounds for length {count}"
        ),
    }
}

/// `fill` given a *computed* (non-literal) count (M1): the count must be a
/// compile-time literal, since there is no comptime interpreter to fold it.
fn fill_count_not_literal_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `fill` requires a literal count, found a computed `{}` (no const-expr eval)\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `fill` requires a literal count, found a computed `{found}` (no const-expr eval)"
        ),
    }
}

/// `fill` given a literal count `< 1` (or `> u32::MAX`): an array length must
/// be `>= 1` (X2, M1), named against the offending count.
fn fill_count_out_of_range_error(ctx: &Ctx, span: Span, count: i64) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: invalid array length in `{}` (line {})\n  `fill` count {} is invalid (an array length must be >= 1 and <= {})\n  note: declared {}",
            name, span.line, count, u32::MAX, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `fill` count {count} is invalid (an array length must be >= 1 and <= {})",
            u32::MAX
        ),
    }
}

/// `fill` given a linear element type: unlike `dup`/`over`, `fill` has no
/// per-slot `Copy` gate today, so it would silently replicate a linear value
/// (and array-element linearity is not tracked transitively yet, so neither
/// `drop` nor a nested struct's `dup` check would ever see the array's real
/// element count). Reject rather than accept a value the rest of the linear
/// checker can't reason about; array-of-linear support is future work.
fn fill_of_linear_element_error(ctx: &Ctx, span: Span, elem: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear array elements are not supported yet in `{}` (line {})\n  `fill` would replicate a `{}` across every slot, but `{}` is linear and has no `Copy` instance\n  note: declared {}",
            name, span.line, elem, elem, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear array elements are not supported yet: `fill` would replicate a `{elem}` across every slot, but `{elem}` is linear and has no `Copy` instance"
        ),
    }
}

/// An exact `usize` is a runtime index; a bare integer literal coerces and
/// gets a compile-time bounds check; a computed `i64` needs an explicit
/// `>usize`; anything else is a plain type mismatch.
fn check_array_index(
    index: Slot,
    count: u32,
    ctx: &Ctx,
    span: Span,
    op: &str,
) -> Result<(), String> {
    match match_slot(index, Type::Usize) {
        SlotMatch::Exact => Ok(()),
        SlotMatch::LiteralSizeType => {
            let idx = index.int_val.expect("a literal slot carries its value");
            if idx < 0 || idx >= i64::from(count) {
                return Err(array_index_out_of_range_error(ctx, span, count, idx));
            }
            Ok(())
        }
        SlotMatch::NeedsSizeConversion => {
            Err(size_conversion_needed_error(ctx, span, op, Type::Usize))
        }
        // A `str` index is a plain mismatch: the str-to-cstr case can only
        // arise where a `cstr` is wanted, and an index always wants `usize`.
        SlotMatch::NeedsStrToCstrConversion | SlotMatch::Mismatch => {
            Err(type_mismatch_error(ctx, span, op, Type::Usize, index.ty))
        }
    }
}

/// The referent of a reference type, and whether it is mutable.
fn ref_parts(ty: Type, refs: &[RefDecl]) -> Option<(Type, bool)> {
    match ty {
        Type::Ref(id, mutable, _) => Some((refs[id.index()].referent, mutable)),
        _ => None,
    }
}

/// `&x`/`&!x` applied to something that is not a local. A place is a
/// local name and nothing more, so the diagnostic names what was found there
/// and points at the binding that would make it one.
fn borrow_of_non_place_error(ctx: &Ctx, span: Span, spelled: &str, found: &str) -> String {
    format!(
        "error: `{spelled}` does not borrow a place{} (line {}, col {})\n  {found}\n  a place is a local name; bind the value with `| name |` first, then borrow that name",
        in_word(ctx),
        span.line,
        span.col
    )
}

/// ` in `word`` for a word body, empty for a bare REPL line: the suffix the
/// slice's own diagnostics use to place themselves the way every other
/// located error here does.
fn in_word(ctx: &Ctx) -> String {
    match ctx {
        Ctx::Word { name, .. } => format!(" in `{name}`"),
        Ctx::Line { .. } => String::new(),
    }
}

/// R11: a quotation used as the operand of any type-directed consumer is an
/// audited default-deny. A quotation is a compile-time-only marker with a
/// `Cstr` placeholder `ty` (R4) that ordinary matching would silently accept
/// or spell into a mismatch, so every consumer that inspects a popped slot's
/// `ty` names itself through this one guard instead. Only `call`/`times`
/// consume a quotation; the shuffles forward it and `drop` discards it.
fn reject_quotation_operand(ctx: &Ctx, span: Span, op: &str) -> String {
    format!(
        "error: `{op}`{} (line {}) cannot take a quotation as an operand; only `call` and `times` accept a quotation (a runtime quotation value is slice 7)",
        in_word(ctx),
        span.line,
    )
}

/// R8: a quotation stored into an array (`fill`'s element) or through a
/// reference (`!`/`+!`'s value, whether the referent is an array slot, a
/// struct field, or an owned cell) would have to become a runtime value,
/// which this slice cannot represent. The wording names no container because
/// two of the three store paths have none. Shared by all of them (D4).
fn reject_quotation_stored(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: a quotation cannot be stored (escaping quotations are slice 7){} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R10/R26: a quotation passed to a parameter position that is *not* a
/// declared `Type::Quotation`. A quotation argument to a declared quotation
/// parameter is now accepted and inlined (R18); this fires only for the other
/// positions (a non-quotation user parameter, a generated constructor/setter
/// slot, an `extern` argument). Only the stale "Phase 6" parenthetical is
/// reworded to point a runtime quotation value at slice 7 (R26).
fn reject_quotation_argument(ctx: &Ctx, span: Span, word: &str) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: a quotation cannot be passed to `{word}`; only `call` and `times` accept one (a runtime quotation value is slice 7){} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R7, both arms leave a quotation but not the *same* literal: a quotation's
/// body must be statically known where it is used, and a branch merge that
/// picked one arm's would need a runtime code value (D4). Fires at the join,
/// not at consumption (R12's containment rests on it).
fn different_quotations_at_join_error(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: these two branches leave different quotations at line {}{}; give the quotation a declared type (a word output or field) so it can be materialized, or make both arms the same literal (a runtime quotation value is slice 7)",
        span.line,
        in_word(ctx),
    )
}

/// R7, one arm leaves a quotation and the other a value: the `Cstr`
/// placeholder makes the two `ty`s compare equal, so the ordinary branch-type
/// mismatch never catches this; the join guard does.
fn quotation_versus_value_at_join_error(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: one branch of the `if` at line {}{} leaves a quotation and the other does not; a quotation cannot be a runtime value (a runtime quotation value is slice 7)",
        span.line,
        in_word(ctx),
    )
}

/// Only an aggregate or cell local may be borrowed. A scalar local is an
/// SSA temporary with no address, and giving it one is work no criterion
/// needs.
fn borrow_of_scalar_local_error(ctx: &Ctx, span: Span, local: &str, ty: Type) -> String {
    format!(
        "error: cannot borrow the scalar local `{local}` of type `{ty}`{} (line {}, col {})\n  a scalar has no address; borrow a field or an aggregate instead",
        in_word(ctx),
        span.line,
        span.col
    )
}

/// `&x`/`&!x` applied to a local that is *already* a reference. A borrow
/// is only ever taken of a plain aggregate local, and the remedy is to drop
/// the sigil: naming a reference local reborrows it.
fn borrow_of_reference_local_error(ctx: &Ctx, span: Span, local: &str, ty: Type) -> String {
    format!(
        "error: cannot borrow `{local}`{}: it is already the reference `{ty}` (line {}, col {})\n  write `{local}`, not `{spelled}{local}`; naming a reference local reborrows it",
        in_word(ctx),
        span.line,
        span.col,
        spelled = if matches!(ty, Type::Ref(_, true, _)) { "&!" } else { "&" },
    )
}

/// A reference-mode word applied to something that is not the reference shape
/// it projects through (`&[T N]` for `&>`, `&^T` for `&^`, `&T` for `@`).
fn reference_word_operand_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    expected: &str,
    found: Type,
) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{name}` (line {})\n  `{op}` expected {expected}, found `{found}`\n  note: declared {}",
            span.line,
            effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` expected {expected}, found `{found}`")
        }
    }
}

/// `!`/`+!` through a shared reference. Storing through a `&T` is
/// meaningless, and the mutable spelling is right there.
fn store_through_shared_reference_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    format!(
        "error: `{op}` cannot store through the shared reference `{found}`{} (line {})\n  borrow it mutably with `&!` (and project with the `&!`-spelled accessors) to write through it",
        in_word(ctx),
        span.line
    )
}

/// `@`/`!`/`+!` are restricted to a `Copy` referent. Fetching a linear
/// value through a reference would manufacture a second owner; storing over
/// one would silently leak the value being overwritten (nothing auto-drops).
fn access_of_linear_referent_error(ctx: &Ctx, span: Span, op: &str, referent: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    let why = if op == "@" {
        "fetching one would make a second owner of a value that is used exactly once"
    } else {
        "storing over one would silently leak the value being overwritten; nothing auto-drops"
    };
    format!(
        "error: `{op}` cannot access the linear referent `{referent}`{} (line {})\n  {why}",
        in_word(ctx),
        span.line
    )
}

/// Exclusivity, in whichever of its two symmetric directions was
/// violated — a new mutable borrow conflicts with any live borrow of the place,
/// a new shared one with a live mutable borrow. When the live borrow is a
/// projection, the note says outright that path disjointness is not modeled,
/// since the two references may well be aimed at different fields.
fn conflicting_borrow_error(
    ctx: &Ctx,
    span: Span,
    place: &str,
    new_mutable: bool,
    live: &Deriv,
) -> String {
    let sigil = if new_mutable { "&!" } else { "&" };
    let held = if live.mutable { "mutable" } else { "shared" };
    let note = if live.projected {
        "\n  note: path disjointness is not modeled: a reference projected into one field borrows the whole place"
    } else {
        ""
    };
    format!(
        "error: `{sigil}{place}` conflicts with a live borrow of `{place}`{} (line {}, col {})\n  the {held} borrow taken at line {}, col {} is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first{note}",
        in_word(ctx),
        span.line,
        span.col,
        live.span.line,
        live.span.col,
    )
}

/// Naming a `&!` local reborrows it, and a reborrow may not be taken
/// while anything derived from the previous one is still live — the two would be
/// two simultaneous mutable references into the same place.
fn suspended_place_error(ctx: &Ctx, span: Span, place: &str, live: &Deriv) -> String {
    format!(
        "error: cannot reborrow `{place}`{} while a reference derived from it is live (line {}, col {})\n  the derivation taken at line {}, col {} is still live\n  a mutable borrow suspends its place until every reference derived from it is consumed",
        in_word(ctx),
        span.line,
        span.col,
        live.span.line,
        live.span.col,
    )
}

/// Consuming a place — moving it into a word, or disposing of it — while a
/// reference derived from it is still live. The reference would be left aimed at
/// storage its owner has given away.
fn consume_of_borrowed_place_error(
    ctx: &Ctx,
    span: Span,
    place: &str,
    ty: Type,
    live: &Deriv,
) -> String {
    let held = if live.mutable { "mutable" } else { "shared" };
    format!(
        "error: cannot consume the borrowed local `{place}` of type `{ty}`{} (line {}, col {})\n  the {held} borrow taken at line {}, col {} is still live\n  a place stays borrowed until every reference derived from it is consumed",
        in_word(ctx),
        span.line,
        span.col,
        live.span.line,
        live.span.col,
    )
}

/// A mutable borrow of a place a second live name denotes. Naming an
/// aggregate does not copy it, so two locals — or a local and a value still on
/// the virtual stack — can denote one region; mutating through one would then be
/// silently observable through the other, which is exactly the class of silent
/// failure the language exists to reject.
fn aliased_place_borrow_error(
    ctx: &Ctx,
    span: Span,
    place: &str,
    origin: &AliasOrigin<'_>,
) -> String {
    let (alias, other, remedy) = match origin {
        AliasOrigin::Name(name) => (
            format!("`{name}`"),
            format!("`{name}`"),
            "use `dup` for an independent copy",
        ),
        AliasOrigin::Stack(pushed) => (
            format!(
                "a value on the stack (pushed at line {}, col {})",
                pushed.line, pushed.col
            ),
            "that value".to_string(),
            "`dup` that value for an independent copy, or consume it before taking the borrow",
        ),
    };
    format!(
        "error: cannot borrow `{place}` mutably{} (line {}, col {}): it is aliased by {alias}\n  both denote one region of memory, so a mutation through `{place}` would be silently visible through {other}\n  {remedy}",
        in_word(ctx),
        span.line,
        span.col,
    )
}

/// The symmetric direction: naming an aggregate while a mutable borrow of
/// its storage is live. The converse of an exclusivity rule is
/// easy to omit, and this is that omission: checking only at the borrow
/// catches `v ... &!v` and misses `&!v ... v`, which is the same hazard with the
/// two terms swapped.
fn naming_aliases_borrowed_place_error(ctx: &Ctx, span: Span, name: &str, live: &Deriv) -> String {
    format!(
        "error: cannot name `{name}`{} (line {}, col {}): a mutable borrow of it is still live (line {}, col {})\n  naming an aggregate does not copy it, so this name would denote the storage that borrow mutates\n  finish with the borrow first, or `dup` for an independent copy",
        in_word(ctx),
        span.line,
        span.col,
        live.span.line,
        live.span.col,
    )
}

/// Two construction sites the declaration-site rule cannot reach: `fill`'s
/// element and `^`'s payload accept
/// whatever type is on the stack, with no declaration anywhere for
/// `check_no_stored_references` to have caught.
fn constructed_reference_error(ctx: &Ctx, span: Span, position: &str, ty: Type) -> String {
    format!(
        "error: a reference cannot be stored{} (line {})\n  {position} has type `{ty}`\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow",
        in_word(ctx),
        span.line
    )
}

/// Every `&`-led word — the two prefix borrow operators and the
/// reference-mode accessor family. Returns `None` if `name` is not `&`-led
/// (the caller falls through to the ordinary lookup chain).
///
/// One spelling per shape *and* per mutability: the mutability is in the
/// token, never inherited from the receiver, so a reader gets reference-ness,
/// mutability and arity from the word alone. Every accessor consumes its
/// reference argument the way any word consumes its arguments.
#[allow(clippy::too_many_arguments)]
fn check_reference_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    scope: &Scope,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
) -> Result<Option<Vec<Slot>>, String> {
    if !name.starts_with('&') {
        return Ok(None);
    }
    let mutable = name.starts_with("&!");
    let rest = &name[if mutable { 2 } else { 1 }..];
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);

    match rest {
        ">" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            if stack[n - 1].quot.is_some() || stack[n - 2].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, name));
            }
            let index = stack[n - 1];
            let Some((referent, recv_mut)) = ref_parts(stack[n - 2].ty, refs) else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a reference to an array",
                    stack[n - 2].ty,
                ));
            };
            let Type::Array(id, _) = referent else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a reference to an array",
                    stack[n - 2].ty,
                ));
            };
            if recv_mut != mutable {
                let want = intern_ref_type(refs, referent, mutable);
                return Err(type_mismatch_error(ctx, span, name, want, stack[n - 2].ty));
            }
            let (count, elem) = (arrays[id.index()].count, arrays[id.index()].element);
            check_array_index(index, count, ctx, span, name)?;
            let out = intern_ref_type(refs, elem, mutable);
            let deriv = prov.project(stack[n - 2].deriv);
            stack.truncate(n - 2);
            stack.push(Slot::derived(out, deriv));
        }
        "^" => {
            let n = stack.len();
            if n < 1 {
                return Err(need(name, 1, n));
            }
            if stack[n - 1].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, name));
            }
            let Some((referent, recv_mut)) = ref_parts(stack[n - 1].ty, refs) else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a reference to an owning cell",
                    stack[n - 1].ty,
                ));
            };
            let Type::OwnedCell(cell_id, _) = referent else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a reference to an owning cell",
                    stack[n - 1].ty,
                ));
            };
            if recv_mut != mutable {
                let want = intern_ref_type(refs, referent, mutable);
                return Err(type_mismatch_error(ctx, span, name, want, stack[n - 1].ty));
            }
            let payload = cells[cell_id.index()].payload;
            let out = intern_ref_type(refs, payload, mutable);
            let deriv = prov.project(stack[n - 1].deriv);
            stack.truncate(n - 1);
            stack.push(Slot::derived(out, deriv));
        }
        _ => {
            if let Some((struct_name, field_name)) = rest.split_once('>') {
                if let Some(idx) = ctx.structs().iter().position(|d| d.name == struct_name) {
                    let decl = &ctx.structs()[idx];
                    if let Some(field_ty) = decl
                        .fields
                        .iter()
                        .find(|(f, _)| f == field_name)
                        .map(|(_, ty)| *ty)
                    {
                        let struct_ty = Type::Struct(StructId::from_index(idx), decl.name_static);
                        let want = intern_ref_type(refs, struct_ty, mutable);
                        let n = stack.len();
                        if n < 1 {
                            return Err(need(name, 1, n));
                        }
                        if stack[n - 1].quot.is_some() {
                            return Err(reject_quotation_operand(ctx, span, name));
                        }
                        if stack[n - 1].ty != want {
                            return Err(type_mismatch_error(
                                ctx,
                                span,
                                name,
                                want,
                                stack[n - 1].ty,
                            ));
                        }
                        let out = intern_ref_type(refs, field_ty, mutable);
                        let deriv = prov.project(stack[n - 1].deriv);
                        stack.truncate(n - 1);
                        stack.push(Slot::derived(out, deriv));
                        return Ok(Some(std::mem::take(stack)));
                    }
                }
            }
            // Everything else is a prefix borrow of a local, and only of a
            // local.
            if rest.is_empty() {
                return Err(borrow_of_non_place_error(
                    ctx,
                    span,
                    name,
                    "it names nothing (a bare sigil cannot borrow whatever happens to be on the stack)",
                ));
            }
            let Some(local_ty) = scope.local_type(rest) else {
                let found = if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    format!("`{rest}` is a literal, not a local")
                } else {
                    format!("`{rest}` is not a local in scope")
                };
                return Err(borrow_of_non_place_error(ctx, span, name, &found));
            };
            // R11: `&q` on a quotation local currently reaches
            // `borrow_of_scalar_local_error`, whose message lies about the
            // `Cstr` placeholder; reject with the named-op wording instead.
            if scope.local(rest).is_some_and(|b| b.quot.is_some()) {
                return Err(reject_quotation_operand(ctx, span, name));
            }
            if local_ty.is_ref() {
                return Err(borrow_of_reference_local_error(ctx, span, rest, local_ty));
            }
            if !matches!(
                local_ty,
                Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)
            ) {
                return Err(borrow_of_scalar_local_error(ctx, span, rest, local_ty));
            }
            // Borrowing is not a move, but the referent still has to be
            // there. A local consumed earlier holds nothing, and borrowing it
            // would read (and project through) storage its owner has already
            // freed.
            if let Some(site) = scope.moves.moved_site(rest) {
                return Err(use_after_move_error(ctx, span, rest, local_ty, site));
            }
            // Exclusivity, symmetric. A new mutable borrow conflicts with
            // any live borrow of the place; a new shared one conflicts with a
            // live mutable borrow. Per place, never a global counter: two live
            // `&!` rooted at different locals do not conflict.
            if let Some(id) = live_deriv(stack, scope, prov, |d| {
                d.owned_root.as_deref() == Some(rest) && (mutable || d.mutable)
            }) {
                return Err(conflicting_borrow_error(
                    ctx,
                    span,
                    rest,
                    mutable,
                    prov.deriv(id),
                ));
            }
            // A second live name for one region makes a mutation through
            // this borrow silently observable through that name. Checked here
            // *and* symmetrically at the naming: a naming that comes first is
            // caught here, one that comes later is caught there. Naming an
            // aggregate with no `&!` anywhere near it stays free either way.
            if mutable {
                if let Some(origin) = aliasing_origin(stack, scope, prov, rest) {
                    return Err(aliased_place_borrow_error(ctx, span, rest, &origin));
                }
            }
            let out = intern_ref_type(refs, local_ty, mutable);
            let deriv = prov.borrow(rest, mutable, span);
            stack.push(Slot::derived(out, Some(deriv)));
        }
    }
    Ok(Some(std::mem::take(stack)))
}

/// `@` fetches, `!` stores, `+!` adds in place. All three are restricted
/// to a `Copy` referent, which covers a Copy *aggregate* as well as a Copy
/// scalar; `@` is typed for both `&T` and `&!T` directly, so there is no
/// `&!T -> &T` demotion coercion to write.
fn check_access_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    refs: &[RefDecl],
) -> Result<Option<Vec<Slot>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "@" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("@", 1, n));
            }
            if stack[n - 1].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, "@"));
            }
            let Some((referent, _)) = ref_parts(stack[n - 1].ty, refs) else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    "@",
                    "a reference",
                    stack[n - 1].ty,
                ));
            };
            if !is_copy(referent, ctx.structs(), ctx.enums(), arrays) {
                return Err(access_of_linear_referent_error(ctx, span, "@", referent));
            }
            stack.truncate(n - 1);
            stack.push(Slot::computed(referent));
        }
        "!" | "+!" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let value = stack[n - 1];
            // R8r: guard the stored value strictly above the `match_slot`
            // below, which returns `Exact` on the `Cstr` placeholder into a
            // `&!Cstr` referent (a silent accept) rather than a mismatch. The
            // receiver operand is an ordinary R11 default-deny.
            if value.quot.is_some() {
                return Err(reject_quotation_stored(ctx, span));
            }
            if stack[n - 2].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, name));
            }
            let Some((referent, mutable)) = ref_parts(stack[n - 2].ty, refs) else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a mutable reference",
                    stack[n - 2].ty,
                ));
            };
            if !mutable {
                return Err(store_through_shared_reference_error(
                    ctx,
                    span,
                    name,
                    stack[n - 2].ty,
                ));
            }
            if !is_copy(referent, ctx.structs(), ctx.enums(), arrays) {
                return Err(access_of_linear_referent_error(ctx, span, name, referent));
            }
            if name == "+!" && !referent.is_int() {
                return Err(type_mismatch_error(ctx, span, "+!", Type::I64, referent));
            }
            match match_slot(value, referent) {
                SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
                SlotMatch::NeedsSizeConversion => {
                    return Err(size_conversion_needed_error(ctx, span, name, referent));
                }
                SlotMatch::NeedsStrToCstrConversion => {
                    return Err(str_needs_cstr_conversion_error(ctx, span, name));
                }
                SlotMatch::Mismatch => {
                    return Err(type_mismatch_error(ctx, span, name, referent, value.ty));
                }
            }
            stack.truncate(n - 2);
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

/// Apply an array word (`fill`/`len`) if `name` is one, returning
/// `Some(stack)`; `None` if the name is not an array word (the caller then
/// looks it up in the env). These are generic over the array shape, so
/// (like the shuffles and numeric operators) they dispatch on the concrete
/// operand types rather than a fixed env signature (R6, R10):
///
/// - `fill ( T -- [T N] )`: the top slot is the compile-time count `N` (a
///   literal, M1), the slot below is the element `T`; interns the `(T, N)`
///   shape (R3) and pushes it.
/// - `len ( [T N] -- usize )`: **non-consuming**, folds to the constant `N`.
///
/// Element access is a reference word (`&>`/`&!>` then `@`/`!`), not an
/// array word: it goes through `check_access_word` instead.
/// The two `str`-only words: `len ( str -- usize )` (R8) and `cstr
/// ( str -- cstr )` (R7, the one explicit `str` -> `cstr` conversion — there
/// is no reverse). Tried before `check_array_word`, whose own `len` claims
/// the name unconditionally otherwise: returning `None` here when the
/// operand isn't a `str` lets that array path still see it.
fn check_str_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
) -> Result<Option<Vec<Slot>>, String> {
    // R11: `len`/`cstr` inspect the top operand's `ty`; reject a quotation
    // here (before `len` falls through to the array path on a non-`str`).
    if matches!(name, "len" | "cstr") && stack.last().is_some_and(|s| s.quot.is_some()) {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    match name {
        "len" => {
            let Some(top) = stack.last() else {
                return Ok(None);
            };
            if top.ty != Type::Str {
                return Ok(None);
            }
            stack.pop();
            stack.push(Slot::computed(Type::Usize));
        }
        "cstr" => {
            let n = stack.len();
            if n < 1 {
                return Err(underflow_error(ctx, span, "cstr", 1, n));
            }
            let top = stack[n - 1];
            if top.ty != Type::Str {
                return Err(cstr_conversion_source_error(ctx, span, top.ty));
            }
            stack.truncate(n - 1);
            stack.push(Slot::computed(Type::Cstr));
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

fn check_array_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
) -> Result<Option<Vec<Slot>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "fill" => {
            let n = stack.len();
            if n < 2 {
                return Err(need("fill", 2, n));
            }
            let count = stack[n - 1];
            let element = stack[n - 2];
            // R8f: a quotation element would have to become a runtime array
            // value. Guarded strictly above `contains_reference` below, whose
            // registry index would panic on an aggregate placeholder (R4); the
            // `Cstr` placeholder is registry-free but the guard order is what
            // R4's reasoning pins. A quotation count is a plain operand (R11).
            if element.quot.is_some() {
                return Err(reject_quotation_stored(ctx, span));
            }
            if count.quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, "fill"));
            }
            let Some(count_val) = count.int_val else {
                return Err(fill_count_not_literal_error(ctx, span, count.ty));
            };
            if !(1..=i64::from(u32::MAX)).contains(&count_val) {
                return Err(fill_count_out_of_range_error(ctx, span, count_val));
            }
            // A construction site the declaration-site rule cannot reach: `fill` accepts
            // any `Copy` element, and `&T` is `Copy`, so the declaration-site
            // sweep never sees this shape.
            if contains_reference(element.ty, ctx.structs(), ctx.enums(), arrays) {
                return Err(constructed_reference_error(
                    ctx,
                    span,
                    "the element `fill` would store",
                    element.ty,
                ));
            }
            if !is_copy(element.ty, ctx.structs(), ctx.enums(), arrays) {
                return Err(fill_of_linear_element_error(ctx, span, element.ty));
            }
            let array_ty = intern_array_type(arrays, element.ty, count_val as u32);
            stack.truncate(n - 2);
            stack.push(Slot::computed(array_ty));
        }
        "len" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("len", 1, n));
            }
            if stack[n - 1].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, "len"));
            }
            if !matches!(stack[n - 1].ty, Type::Array(..)) {
                return Err(array_word_operand_error(ctx, span, "len", stack[n - 1].ty));
            }
            // Non-consuming: the array stays; `len` folds to the constant `N`.
            stack.push(Slot::computed(Type::Usize));
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

/// The three owning-cell access words: `^ ( T -- ^T )` constructs a cell,
/// `^> ( ^T -- T )` consumes it and yields the payload, `^|> ( ^T -- ^T T )`
/// is a non-consuming peek restricted to a `Copy` payload. Matched by exact
/// name only, so `^>x`/`^|>x` fall through to the ordinary unknown-word error.
fn check_owned_cell_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    cells: &mut Vec<OwnedCellDecl>,
) -> Result<Option<Vec<Slot>>, String> {
    // R11: `^`/`^>`/`^|>` each inspect the top operand's `ty`.
    if matches!(name, "^" | "^>" | "^|>") && stack.last().is_some_and(|s| s.quot.is_some()) {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "^" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("^", 1, n));
            }
            let payload = stack[n - 1].ty;
            // Another construction site the declaration-site rule cannot reach: `^` interns a
            // cell over any payload type with no filter of its own.
            if contains_reference(payload, ctx.structs(), ctx.enums(), arrays) {
                return Err(constructed_reference_error(
                    ctx,
                    span,
                    "the payload `^` would store",
                    payload,
                ));
            }
            let cell_ty = intern_owned_cell_type(cells, payload);
            stack.truncate(n - 1);
            stack.push(Slot::computed(cell_ty));
        }
        "^>" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("^>", 1, n));
            }
            let Type::OwnedCell(id, _) = stack[n - 1].ty else {
                return Err(owned_cell_word_operand_error(
                    ctx,
                    span,
                    "^>",
                    stack[n - 1].ty,
                ));
            };
            let payload = cells[id.index()].payload;
            stack.truncate(n - 1);
            stack.push(Slot::computed(payload));
        }
        "^|>" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("^|>", 1, n));
            }
            let cell_ty = stack[n - 1].ty;
            let Type::OwnedCell(id, _) = cell_ty else {
                return Err(owned_cell_word_operand_error(ctx, span, "^|>", cell_ty));
            };
            let payload = cells[id.index()].payload;
            if !is_copy(payload, ctx.structs(), ctx.enums(), arrays) {
                return Err(peek_of_linear_owned_payload_error(
                    ctx, span, cell_ty, payload,
                ));
            }
            // Non-consuming: the cell stays, the payload copy is pushed atop it.
            stack.push(Slot::computed(payload));
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

/// `S|>fi` (R10): a new non-consuming `( S -- S field )` peek, keyed by the
/// per-struct-per-field name (unlike `fill`, it is not generic over a
/// shape, so it is not a fixed entry in `struct_generated_sigs`
/// either: it is looked up by parsing the `Struct|>field` name against the
/// struct registry, same as the IR's `structs.words` map). `None` if `name`
/// doesn't split on `|>` or doesn't resolve to a known struct+field (the
/// caller falls through to the env lookup, so an unrelated word still gets
/// the ordinary unknown-word error). A linear field is rejected outright
/// (R10): the peek would leave a second, unowned reference to a resource the
/// aggregate still owns, with no reference machinery to make that legal.
fn check_struct_peek_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    prov: &mut Provenance,
) -> Result<Option<Vec<Slot>>, String> {
    let Some((struct_name, field_name)) = name.split_once("|>") else {
        return Ok(None);
    };
    let structs = ctx.structs();
    let Some(idx) = structs.iter().position(|d| d.name == struct_name) else {
        return Ok(None);
    };
    let decl = &structs[idx];
    let Some((_, field_ty)) = decl.fields.iter().find(|(f, _)| f == field_name) else {
        return Ok(None);
    };
    let field_ty = *field_ty;
    if !is_copy(field_ty, structs, ctx.enums(), arrays) {
        return Err(peek_of_linear_field_error(ctx, span, name, field_ty));
    }
    let struct_ty = Type::Struct(StructId::from_index(idx), decl.name_static);
    let n = stack.len();
    if n < 1 {
        return Err(underflow_error(ctx, span, name, 1, n));
    }
    let top = stack[n - 1];
    if top.quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    if top.ty != struct_ty {
        return Err(type_mismatch_error(ctx, span, name, struct_ty, top.ty));
    }
    // The peek is non-consuming and pushes the field's *interior address*,
    // so two peeks of one field of one struct are two names for one region.
    let alias = peek_region(&mut stack[n - 1], field_ty, field_name, span, prov);
    stack.push(Slot {
        alias,
        ..Slot::computed(field_ty)
    });
    Ok(Some(std::mem::take(stack)))
}

/// `S>fi` (R21's third route): the ordinary, consuming field getter, already
/// registered in `struct_generated_sigs` and otherwise left to the generic
/// env-based dispatch. That generic path pushes a plain `Slot::computed`
/// with no alias, but for an aggregate field this getter's IR lowering
/// pushes the field's *interior address* rather than copying it out (same
/// device as `S|>fi`'s peek), so the struct operand and the extracted field
/// alias one region exactly as two peeks would. `None` for a scalar field
/// (no region to alias) or an unresolved name, so every other call site is
/// untouched. Consuming, unlike the peek: the struct operand is popped, not
/// left on the stack, but the aliasing hazard is unaffected by that, since
/// the operand's own local binding (if it is named) keeps the same region
/// regardless of what happens to the stack-level copy of its slot.
fn check_struct_get_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    prov: &mut Provenance,
) -> Result<Option<Vec<Slot>>, String> {
    let Some((struct_name, field_name)) = name.split_once('>') else {
        return Ok(None);
    };
    let structs = ctx.structs();
    let Some(idx) = structs.iter().position(|d| d.name == struct_name) else {
        return Ok(None);
    };
    let decl = &structs[idx];
    let Some((_, field_ty)) = decl.fields.iter().find(|(f, _)| f == field_name) else {
        return Ok(None);
    };
    let field_ty = *field_ty;
    if !field_ty.is_aggregate() {
        return Ok(None);
    }
    let struct_ty = Type::Struct(StructId::from_index(idx), decl.name_static);
    let n = stack.len();
    if n < 1 {
        return Err(underflow_error(ctx, span, name, 1, n));
    }
    let top = stack[n - 1];
    if top.quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    if top.ty != struct_ty {
        return Err(type_mismatch_error(ctx, span, name, struct_ty, top.ty));
    }
    let alias = peek_region(&mut stack[n - 1], field_ty, field_name, span, prov);
    stack.truncate(n - 1);
    stack.push(Slot {
        alias,
        ..Slot::computed(field_ty)
    });
    Ok(Some(std::mem::take(stack)))
}

/// Apply a stack shuffle if `name` is one, returning `Some(stack)`; `None` if
/// the name is not a shuffle (the caller then looks it up in the env). Shuffles
/// move concrete slot types with no fixed signature: `dup` of a `bool` yields
/// two `bool`s, `swap` reorders whatever two types are on top, etc.
fn check_shuffle(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    prov: &mut Provenance,
) -> Result<Option<Vec<Slot>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "dup" => {
            let top = *stack.last().ok_or_else(|| need("dup", 1, stack.len()))?;
            // R4 (D3): `dup` is the explicit copy, so it is gated on `Copy`.
            // The pure reorderings below (`swap`/`rot`) move rather than copy
            // and stay legal on a linear value.
            if !is_copy(top.ty, ctx.structs(), ctx.enums(), arrays) {
                return Err(cannot_copy_error(ctx, span, "dup", top.ty));
            }
            // `dup` of an aggregate deep-copies it (`Alloc`+`Blit`), so the
            // copy denotes a region of its own — this is the whole remedy for an
            // aliased place. `over` below reuses the value instead, and so
            // deliberately keeps the region it copies.
            stack.push(Slot { alias: None, ..top });
        }
        "drop" => {
            let top = stack.pop().ok_or_else(|| need("drop", 1, 0))?;
            // R6 (slice 8b): a side observation only. `drop` still pops one
            // value of any type with no type check, exactly as before; the
            // recorded type is what lets `check`'s post-pass resolve which
            // concrete override (if any) this call site dispatches to.
            // R11 carve-out: `drop` of a compile-time-only quotation marker
            // discards it with nothing to dispose, and its `Cstr` placeholder
            // is inert in the drop-override graph; skip the push.
            if top.quot.is_none() {
                prov.dropped.push(top.ty);
            }
        }
        "swap" => {
            let n = stack.len();
            if n < 2 {
                return Err(need("swap", 2, n));
            }
            stack.swap(n - 1, n - 2);
        }
        "over" => {
            let n = stack.len();
            if n < 2 {
                return Err(need("over", 2, n));
            }
            let mut below = stack[n - 2];
            // `over` is gated exactly like `dup`.
            if !is_copy(below.ty, ctx.structs(), ctx.enums(), arrays) {
                return Err(cannot_copy_error(ctx, span, "over", below.ty));
            }
            // Unlike `dup`, `over` reuses the value rather than deep-copying it,
            // so both slots denote one address. An anonymous aggregate has no
            // region yet, and binding each slot would otherwise mint a separate
            // one, hiding the aliasing.
            if below.alias.is_none() && below.ty.is_aggregate() {
                let region = prov.fresh_region();
                let set = prov.alias_set_of(region);
                below.alias = Some(Alias { set, span });
                stack[n - 2].alias = below.alias;
            }
            stack.push(below);
        }
        "rot" => {
            let n = stack.len();
            if n < 3 {
                return Err(need("rot", 3, n));
            }
            // a b c -> b c a
            let a = stack[n - 3];
            stack[n - 3] = stack[n - 2];
            stack[n - 2] = stack[n - 1];
            stack[n - 1] = a;
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module)
    }

    /// U7 (R18): the exported-signature helper flags a word whose effect
    /// names a private type of its own module, and clears once that type is
    /// exported too (the positive half, R18's own escape hatch).
    #[test]
    fn exported_signature_rule_flags_private_type() {
        use crate::ast::{ModuleInfo, TypedSlot};
        let structs = vec![StructDecl {
            name: "Res".to_string(),
            name_static: "Res",
            fields: vec![("n".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        }];
        let mk_word = WordDef {
            name: "mk".to_string(),
            effect: StackEffect {
                inputs: Vec::new(),
                outputs: vec![TypedSlot {
                    name: None,
                    ty: Type::Struct(StructId::from_index(0), "Res"),
                }],
            },
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            module: 0,
        };
        let mut module = Module {
            words: vec![mk_word],
            structs,
            enums: Vec::new(),
            arrays: Vec::new(),
            owned_cells: Vec::new(),
            refs: Vec::new(),
            externs: Vec::new(),
            instantiations: HashMap::new(),
            modules: vec![ModuleInfo {
                imports: HashMap::new(),
                exports: vec![("mk".to_string(), Span::default())],
                selective: HashMap::new(),
            }],
        };

        let err = check_exported_signatures(&module).unwrap_err();
        assert!(err.contains("mk"), "names the word: {err}");
        assert!(err.contains("Res"), "names the private type: {err}");

        module.modules[0]
            .exports
            .push(("Res".to_string(), Span::default()));
        assert!(
            check_exported_signatures(&module).is_ok(),
            "exporting the type clears the rule"
        );
    }

    /// U8 (R20/R21): the selective-import validator rejects a name absent from
    /// its source module's export list (R20), two selective imports of one name
    /// (R21, naming both sources), and a selective name colliding with a local
    /// word (R21), while a clean import passes.
    #[test]
    fn selective_import_collision_is_rejected() {
        use crate::ast::ModuleInfo;

        fn info(exports: &[&str]) -> ModuleInfo {
            ModuleInfo {
                imports: HashMap::new(),
                exports: exports
                    .iter()
                    .map(|n| (n.to_string(), Span::default()))
                    .collect(),
                selective: HashMap::new(),
            }
        }
        fn word(name: &str, module: u32) -> WordDef {
            WordDef {
                name: name.to_string(),
                effect: StackEffect::default(),
                body: WordBody::Terms { terms: Vec::new() },
                poly: None,
                module,
            }
        }
        fn module_with(words: Vec<WordDef>, modules: Vec<ModuleInfo>) -> Module {
            Module {
                words,
                structs: Vec::new(),
                enums: Vec::new(),
                arrays: Vec::new(),
                owned_cells: Vec::new(),
                refs: Vec::new(),
                externs: Vec::new(),
                instantiations: HashMap::new(),
                modules,
            }
        }
        fn sel(name: &str, qualifier: &str, target: u32, line: u32) -> SelectiveName {
            SelectiveName {
                name: name.to_string(),
                qualifier: qualifier.to_string(),
                target,
                span: Span { line, col: 1 },
            }
        }

        // R21: modules 1 and 2 each export `p`; module 0 selectively imports it
        // from both, colliding at the second.
        let m = module_with(
            vec![word("p", 1), word("p", 2)],
            vec![info(&[]), info(&["p"]), info(&["p"])],
        );
        let entries = vec![
            vec![sel("p", "a", 1, 1), sel("p", "b", 2, 2)],
            Vec::new(),
            Vec::new(),
        ];
        let err = check_selective_imports(&m, &entries).unwrap_err();
        assert!(err.contains("collides"), "selective collision: {err}");
        assert!(
            err.contains("`a`") && err.contains("`b`"),
            "names both sources: {err}"
        );

        // R20: a name absent from its source's export list is the visibility
        // error, distinct from a collision.
        let m = module_with(vec![word("grow", 1)], vec![info(&[]), info(&[])]);
        let err =
            check_selective_imports(&m, &[vec![sel("grow", "lib", 1, 1)], Vec::new()]).unwrap_err();
        assert!(err.contains("not exported"), "R20 export gate: {err}");
        assert!(!err.contains("collides"), "not the collision error: {err}");

        // R21: a selective name colliding with the importer's own local word.
        let m = module_with(
            vec![word("p", 0), word("p", 1)],
            vec![info(&[]), info(&["p"])],
        );
        let err =
            check_selective_imports(&m, &[vec![sel("p", "lib", 1, 1)], Vec::new()]).unwrap_err();
        assert!(
            err.contains("collides") && err.contains("local"),
            "local collision: {err}"
        );

        // A clean selective import of an exported, non-colliding name passes.
        let m = module_with(vec![word("p", 1)], vec![info(&[]), info(&["p"])]);
        assert!(check_selective_imports(&m, &[vec![sel("p", "lib", 1, 1)], Vec::new()]).is_ok());
    }

    /// U12 (R13): an `[i64 8]` array shape declared in two files interns into
    /// the one shared registry the driver assembles across the closure,
    /// deduping to a single `ArrayId` rather than one per file.
    #[test]
    fn array_shape_dedupes_across_files() {
        use crate::parser::parse_bodies;
        let a = lex(": fa ( [i64 8] -- ) drop ;").unwrap();
        let b = lex(": fb ( [i64 8] -- ) drop ;").unwrap();
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let no_imports = HashMap::new();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        parse_bodies(
            &a,
            &structs,
            &enums,
            0,
            &no_imports,
            &[],
            &no_imports,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .unwrap();
        parse_bodies(
            &b,
            &structs,
            &enums,
            1,
            &no_imports,
            &[],
            &no_imports,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .unwrap();
        assert_eq!(
            arrays.len(),
            1,
            "two files' [i64 8] dedupe to one ArrayId in the shared registry"
        );
    }

    /// U3 (R12): the duplicate-type-name check partitions by owning module, so
    /// two modules each declaring `Point` is not a duplicate, while two `Point`
    /// decls in one module still is (reported by the raw `name_static`, not the
    /// resolver's mangled `name`).
    #[test]
    fn duplicate_type_check_is_per_module() {
        let mk = |module: u32| StructDecl {
            name: format!("Point__m{module}"),
            name_static: "Point",
            fields: Vec::new(),
            span: crate::ast::Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module,
        };
        // Two modules, one `Point` each: not a duplicate.
        assert!(check_duplicate_type_names(&[mk(0), mk(1)], &[]).is_ok());
        // Same module, two `Point`: a duplicate, named by the raw surface name.
        let same_module = vec![
            StructDecl {
                name: "Point".to_string(),
                name_static: "Point",
                fields: Vec::new(),
                span: crate::ast::Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
            StructDecl {
                name: "Point".to_string(),
                name_static: "Point",
                fields: Vec::new(),
                span: crate::ast::Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
        ];
        let err = check_duplicate_type_names(&same_module, &[]).unwrap_err();
        assert!(err.contains("duplicate type `Point`"), "raw name: {err}");
    }

    /// Two words of the same name in one module are rejected; the same pair
    /// split across two modules is not (mirrors `duplicate_type_check_is_per_module`).
    #[test]
    fn duplicate_word_name_is_rejected_only_within_one_module() {
        fn word_at(name: &str, module: u32, line: u32) -> WordDef {
            WordDef {
                name: name.to_string(),
                effect: StackEffect::default(),
                body: WordBody::Terms {
                    terms: vec![Term {
                        kind: TermKind::IntLit(0),
                        span: Span { line, col: 1 },
                    }],
                },
                poly: None,
                module,
            }
        }
        fn word(name: &str, module: u32) -> WordDef {
            word_at(name, module, 0)
        }

        // Two modules, one `push` each: not a duplicate.
        assert!(check_duplicate_word_names(&[word("push", 0), word("push", 1)]).is_ok());

        // Same module, two `push`: a duplicate, naming both locations.
        let err = check_duplicate_word_names(&[word_at("push", 0, 1), word_at("push", 0, 2)])
            .unwrap_err();
        assert!(
            err.contains("duplicate word `push`") && err.contains("line 2"),
            "names the repeat's location: {err}"
        );
        assert!(
            err.contains("first defined at line 1"),
            "also names the first definition's location: {err}"
        );

        // A repeat `main` in one module is caught too: nothing else validates
        // `main`'s multiplicity within a module.
        let err = check_duplicate_word_names(&[word("main", 0), word("main", 0)]).unwrap_err();
        assert!(err.contains("duplicate word `main`"), "names main: {err}");

        // Two `drop`s sharing a module are *not* rejected here: distinct-struct
        // overloading is `find_drop_overloads`'s job, keyed by struct id, not
        // this check's; re-flagging by name alone would reject Phase 3 slice
        // 8b's legitimate multi-type overloading.
        assert!(check_duplicate_word_names(&[word("drop", 0), word("drop", 0)]).is_ok());
    }

    // A one-field struct with a `drop` overload: linear for the same reason any
    // resource is, used to force the `Copy`-bound failure (X5).
    const SPY: &str = "type: Spy tag i64 ;\n: drop ( Spy -- ) | s | s Spy>tag drop ;\n";

    #[test]
    fn quotation_survives_dup_swap_and_bind() {
        // Cu1 (D2/R4): a quotation `Slot` is `Copy`, so a shuffle moves it (and
        // its `quot` marker) verbatim; a bind carries the marker into the
        // `Binding`, from which a local read reconstructs it (the read-back is
        // witnessed end-to-end by `quotation_forwarded_through_bind_still_calls`).
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        let arrays: Vec<ArrayDecl> = Vec::new();
        let mut prov = Provenance::default();
        let span = Span { line: 1, col: 1 };
        let marker = Some(QuotRef::Known(QuotId(0)));
        let quot = Slot {
            quot: marker,
            ..Slot::computed(Type::Cstr)
        };

        // Every shuffle keeps the marker on the slot it moves.
        for name in ["dup", "swap", "over", "rot"] {
            let mut stack = match name {
                "swap" | "over" => vec![Slot::computed(Type::I64), quot],
                "rot" => vec![Slot::computed(Type::I64), Slot::computed(Type::I64), quot],
                _ => vec![quot],
            };
            let out = check_shuffle(name, span, &mut stack, &ctx, &arrays, &mut prov)
                .unwrap()
                .unwrap();
            assert!(
                out.iter().any(|s| s.quot == marker),
                "`{name}` dropped the quotation marker"
            );
        }

        // A bind carries the marker into the `Binding`.
        let mut scope = Scope::default();
        scope.bind("q", quot, false, &mut prov);
        assert_eq!(scope.local("q").unwrap().quot, marker);
    }

    #[test]
    fn times_typing_obligations() {
        // R18u: the three `times` typing obligations, each its own row, since a
        // missed guard is a silent accept (the well-typed witness never trips
        // them). Move-state identity, the whole-row guard, and row-effect
        // equality.

        // A well-typed `times` accepts (the body consumes the index and returns
        // the row unchanged, touching no linear local).
        check_src(": main ( -- ) 0 10 [ + ] times . ;\n").unwrap();

        // (1) Move-state identity: consuming an outer linear local is rejected,
        // named, with the repeated-disposal reason.
        let consume = check_src(&format!(
            "{SPY}: main ( -- ) 5 Spy | s | 0 10 [ | i | s Spy>tag + ] times . ;\n"
        ))
        .expect_err("consuming a linear local should be rejected");
        assert!(
            consume.contains("a `times` body cannot consume `s`")
                && consume.contains("the body runs more than once"),
            "move-state identity should name `s`, got: {consume}"
        );

        // (2) Whole-row guard: a quotation anywhere in the row, not just the
        // consumed top, is rejected.
        let row_quot = check_src(": main ( -- ) [ + ] 3 [ drop ] times ;\n")
            .expect_err("a quotation in the row should be rejected");
        assert!(
            row_quot.contains("`times`")
                && row_quot.contains("cannot take a quotation as an operand"),
            "whole-row guard should reject a row quotation, got: {row_quot}"
        );

        // (3) Row-effect equality: a body that changes the row's depth is
        // rejected.
        let row_effect = check_src(": main ( -- ) 0 10 [ + 1 ] times . ;\n")
            .expect_err("a body that changes the row should be rejected");
        assert!(
            row_effect.contains("`times` body must leave the row unchanged"),
            "row-effect equality should reject a changed row, got: {row_effect}"
        );
    }

    #[test]
    fn merged_quotations_are_rejected_at_the_join() {
        // Cu2 (R7): two *different* quotations merged at an `if` join are
        // rejected at the join (not at consumption), because `lower_if` would
        // otherwise build a `Phi` over two phantoms. The *same* `Known` id in
        // both arms (one literal bound before the `if`, read in each) is safe:
        // `lower_if`'s `t == e` fast path emits no `Phi`, so it must not error.
        let different = check_src(": main ( -- ) true if [ 1 + ] else [ 1 - ] end drop ;\n")
            .expect_err("two different quotations at a join should be rejected");
        assert!(
            different.contains("these two branches leave different quotations"),
            "the join guard should fire, got: {different}"
        );
        check_src(": main ( -- ) [ + ] | q | true if q else q end drop ;\n")
            .expect("the same `Known` id in both arms is safe and must not error");
    }

    #[test]
    fn check_outputs_rejects_a_quotation_left_on_exit() {
        // R10: a matching output *count* means the ordinary path would emit a
        // type mismatch that leaks the `Cstr` placeholder; the dedicated
        // quotation-at-exit branch in `check_outputs` fires first and names the
        // word.
        let err = check_src(": f ( -- i64 ) [ + ] ;\n")
            .expect_err("a quotation left on a word's exit should be rejected");
        assert!(
            err.contains("`f`")
                && err.contains("leaves a quotation on the stack")
                && err.contains("declared output"),
            "check_outputs should name `f` and the output, got: {err}"
        );
    }

    #[test]
    fn infer_line_rejects_a_quotation_left_on_the_residual() {
        // R19: a REPL line has no declared outputs, so R10's route never runs;
        // the `quot` side channel would die at the line boundary while lowering
        // has already pushed a phantom the residual spill would marshal.
        let err = infer_src("1 [ + ]", &[])
            .expect_err("a quotation on a line's residual stack should be rejected");
        assert!(
            err.contains("a quotation cannot be left on the stack at the end of a line"),
            "infer_line should reject the residual quotation, got: {err}"
        );
    }

    #[test]
    fn check_poly_call_rejects_a_quotation_argument() {
        // R9p: `check_poly_call` reads only `stack[base + i].ty`, so a quotation
        // does not *fail* unification, it *succeeds* binding `'T` to the
        // placeholder and monomorphizes a real call over a phantom. The guard
        // before `unify_poly_input` is what makes the R9 rejection reachable.
        let err = check_src(
            ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n\
             : main ( -- ) [ + ] dupit drop drop ;\n",
        )
        .expect_err("a quotation passed to a polymorphic word should be rejected");
        assert!(
            err.contains("a quotation cannot be passed to `dupit`"),
            "check_poly_call should name `dupit`, got: {err}"
        );
    }

    #[test]
    fn poly_term_rejects_a_quotation_literal() {
        // R5p: a quotation literal in a polymorphic body is rejected eagerly at
        // the literal (the polymorphic path cannot yet carry the marker).
        let err = check_src(
            ": bad ( 'T: Copy -- 'T ) [ + ] drop ;\n\
             : main ( -- ) 1 bad . ;\n",
        )
        .expect_err("a quotation literal in a polymorphic body should be rejected");
        assert!(
            err.contains("a quotation in the polymorphic body of `bad`")
                && err.contains("not yet supported"),
            "poly_term should name `bad`, got: {err}"
        );
    }

    #[test]
    fn quotation_as_operand_is_rejected_at_every_audited_site() {
        // R11t: the audit is a *test artifact*, not prose. A missed guard on the
        // `Cstr` placeholder is a silent accept (R4), so every default-deny site
        // gets a row here: a new consumer added later without a guard turns one
        // row from `Err` to `Ok` and fails the test. The one `is_line` row is the
        // REPL residual, checked through `infer_line` rather than `check`.
        //
        // Each row asserts TWO substrings, and this is load-bearing. `site` is
        // the token the message names (the op, or the word for the argument
        // family); `phrase` is text only the quotation rejection produces. The
        // pre-existing generic diagnostics (`operand_pair_mismatch`,
        // `type_mismatch`, `array_word_operand`, `reference_word_operand`,
        // `fill_count_not_literal`, ...) all print the op in backticks too, so a
        // `site`-only row stays green when its guard is removed and the fallback
        // fires: it names the same op. Requiring `phrase` as well is what turns a
        // removed guard from green to red. Every operand-family row shares the
        // one `reject_quotation_operand` phrase; the store/argument/output/
        // residual families carry their own wording no generic diagnostic emits.
        //
        // FIX 2 (verified, no row): the only `check_operator` op that would
        // accept a `Cstr` operand if its guard were removed is `.` (print, whose
        // printable set includes `Str`/`Cstr`), and it already has the `.` row.
        // Every comparison (`=`/`<`/`>`/...), like every arithmetic/bitwise/
        // shift op, requires `is_numeric`/`is_int`/`is_float` and rejects a
        // `cstr` outright, so there is no silent-accept comparison path to row.
        struct Row {
            source: &'static str,
            site: &'static str,
            phrase: &'static str,
            is_line: bool,
        }
        const OPERAND: &str = "cannot take a quotation as an operand";
        // Operand-family row: `site` is the op, `phrase` is the shared wording.
        let op = |source, site| Row {
            source,
            site,
            phrase: OPERAND,
            is_line: false,
        };
        // Any other family: spell both substrings out.
        let w = |source, site, phrase| Row {
            source,
            site,
            phrase,
            is_line: false,
        };
        let rows = [
            // check_operator, both operand positions, plus print.
            op(": main ( -- ) 1 [ + ] + ;\n", "`+`"),
            op(": main ( -- ) [ + ] 1 - . ;\n", "`-`"),
            op(": main ( -- ) [ + ] . ;\n", "`.`"),
            // the `if` condition, before the `bool` mismatch.
            op(": main ( -- ) [ + ] if 1 . else 2 . end ;\n", "`if`"),
            // check_str_word (`len`/`cstr`).
            op(": main ( -- ) [ + ] len ;\n", "`len`"),
            op(": main ( -- ) [ + ] cstr ;\n", "`cstr`"),
            // check_array_word: the `fill` count operand and the stored element.
            op(": main ( -- ) 5 [ + ] fill ;\n", "`fill`"),
            w(
                ": main ( -- ) [ + ] 8 fill drop ;\n",
                "a quotation cannot be stored",
                "escaping quotations are slice 7",
            ),
            // check_array_index, reached through the `&>` reference word.
            op(
                "type: V x i64 ;\n: main ( -- ) 1 2 V | v | &v &V>x [ + ] &> drop drop ;\n",
                "`&>`",
            ),
            // check_owned_cell_word.
            op(": main ( -- ) [ + ] ^ ;\n", "`^`"),
            // check_reference_word's `&q` prefix-borrow-of-a-local form.
            op(": main ( -- ) [ + ] | q | &q drop ;\n", "`&q`"),
            // check_struct_peek_word and check_struct_get_word (an aggregate
            // field, so the getter is intercepted here, not by the env loop).
            op("type: V x i64 ;\n: main ( -- ) [ + ] V|>x ;\n", "`V|>x`"),
            op(
                "type: Inner a i64 ;\ntype: Outer b Inner ;\n: main ( -- ) [ + ] Outer>b ;\n",
                "`Outer>b`",
            ),
            // check_access_word's store paths: the value and the receiver.
            w(
                "type: Box s cstr ;\n: main ( -- ) \"hi\" cstr Box | b | &!b &!Box>s [ + ] ! b drop ;\n",
                "a quotation cannot be stored",
                "escaping quotations are slice 7",
            ),
            op(": main ( -- ) [ + ] 1 ! ;\n", "`!`"),
            // the env argument loop and check_poly_call's input loop (R9/R9p).
            w(
                ": foo ( i64 -- i64 ) ;\n: main ( -- ) [ + ] foo drop ;\n",
                "passed to `foo`",
                "only `call` and `times` accept one",
            ),
            w(
                ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n: main ( -- ) [ + ] dupit drop drop ;\n",
                "passed to `dupit`",
                "only `call` and `times` accept one",
            ),
            // check_outputs (R10) and the `times` body-output row (blocker 2).
            w(
                ": f ( -- i64 ) [ + ] ;\n",
                "declared output",
                "leaves a quotation on the stack",
            ),
            op(
                ": main ( -- ) \"x\" cstr 0 [ drop drop [ + ] ] times drop ;\n",
                "`times`",
            ),
            // the REPL residual (R19), checked through `infer_line`.
            Row {
                source: "1 [ + ]",
                site: "end of a line",
                phrase: "a quotation cannot be left on the stack",
                is_line: true,
            },
        ];
        for Row {
            source,
            site,
            phrase,
            is_line,
        } in rows
        {
            let err = match is_line {
                true => infer_src(source, &[])
                    .expect_err("an audited site must reject a quotation, not silently accept it"),
                false => check_src(source)
                    .expect_err("an audited site must reject a quotation, not silently accept it"),
            };
            assert!(
                err.contains(site),
                "audited site `{site}` was not named, got: {err}"
            );
            assert!(
                err.contains(phrase),
                "audited site `{site}` did not produce its quotation-rejection phrase `{phrase}`, got: {err}"
            );
        }
    }

    #[test]
    fn check_poly_copy_word_accepts_and_instantiates() {
        // R1/R4–R7: a `'T: Copy` word `dup`s its variable and is called at a
        // concrete `Copy` type; the body and the instantiation both check.
        check_src(": dupit ( 'T: Copy -- 'T 'T ) dup ;\n: main ( -- ) 5 dupit drop drop ;")
            .unwrap();
    }

    #[test]
    fn check_poly_word_records_one_instantiation_per_concrete_shape() {
        // R8/R14: each distinct ground θ is recorded once, keyed by call span.
        let module = checked_module(
            ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n\
             : main ( -- ) 5 dupit drop drop true dupit drop drop ;",
        );
        // Two call sites, two distinct θ (i64 and bool): two instantiations.
        let symbols: std::collections::HashSet<&str> = module
            .instantiations
            .values()
            .map(|c| c.symbol.as_str())
            .collect();
        assert_eq!(module.instantiations.len(), 2);
        assert_eq!(symbols.len(), 2);
    }

    #[test]
    fn check_poly_ord_word_accepts_comparison_body() {
        // R7: a `'T: Ord` variable may be compared; the body and a numeric
        // instantiation both check.
        check_src(": less ( 'T: Ord 'T -- bool ) > ;\n: main ( -- ) 3 4 less drop ;").unwrap();
    }

    #[test]
    fn check_poly_length_word_accepts_and_monomorphizes_len() {
        // R1/R5/R9: a length variable is opaque through `len`; the same word
        // instantiates at `[i64 4]` and `[i64 8]`.
        check_src(
            ": alen ( [i64 'N] -- [i64 'N] usize ) len ;\n\
             : main ( -- ) 5 4 fill alen . drop 5 8 fill alen . drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_poly_row_word_accepts_and_expands_outputs() {
        // R1/R5/R7: a row-variable word passes its deeper stack through
        // untouched and duplicates the two `Copy` variables; the resolved
        // instantiation has four concrete outputs, so it interns a bundle.
        let module = checked_module(
            ": dup2 ( ..s 'a: Copy 'b: Copy -- ..s 'a 'b 'a 'b ) over over ;\n\
             : main ( -- ) 1 2 dup2 . . . . ;",
        );
        assert_eq!(module.instantiations.len(), 1);
        let inst = module.instantiations.values().next().unwrap();
        assert_eq!(inst.out_arity, 4);
        assert!(inst.bundle.is_some());
    }

    #[test]
    fn check_x4_type_variable_forced_to_two_concretes_names_both() {
        // X4: one `'T` unified to both `i64` and `bool` at one call site names
        // both concrete types.
        let err = check_src(": pairwise ( 'T 'T -- ) drop drop ;\n: main ( -- ) 1 true pairwise ;")
            .unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("i64"), "unexpected message: {err}");
        assert!(err.contains("bool"), "unexpected message: {err}");
    }

    #[test]
    fn check_x5_copy_bound_on_linear_type_names_variable_type_and_reason() {
        // X5: instantiating a `'T: Copy` word with a linear type is a located
        // call-site error naming the variable, the type, and the linear reason.
        let src = format!("{SPY}: idc ( 'T: Copy -- 'T ) ;\n: main ( -- ) 0 Spy idc drop ;");
        let err = check_src(&src).unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Spy"), "unexpected message: {err}");
        assert!(err.contains("linear"), "unexpected message: {err}");
    }

    #[test]
    fn check_x6_ord_bound_on_non_ord_type_is_error() {
        // X6: instantiating a `'T: Ord` requirement with a non-`Ord` type is a
        // located error.
        let err =
            check_src(": less ( 'T: Ord 'T -- bool ) > ;\n: main ( -- ) true false less drop ;")
                .unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Ord"), "unexpected message: {err}");
    }

    #[test]
    fn check_x7_dup_of_unbounded_variable_names_missing_copy_bound() {
        // X7: `dup` of an unbounded `'T` inside a body names the variable and
        // the missing `Copy` bound.
        let err = check_src(": bad ( 'T -- 'T 'T ) dup ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Copy"), "unexpected message: {err}");
    }

    #[test]
    fn check_x8_compare_of_unbounded_variable_requires_ord() {
        // X8: `>` on an unbounded `'T` inside a body requires an `Ord` bound.
        let err = check_src(": bad ( 'T 'T -- bool ) > ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Ord"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_local_bound_and_never_read_is_unconsumed_error() {
        // A `'T` bound to a local and never read leaks: the polymorphic body
        // checker rejects it exactly as the monomorphic sibling rejects
        // `( ^i64 -- ) | x | ;`, naming the variable.
        let err = check_src(": leaky ( 'T -- ) | x | ;\n: main ( -- ) ;").unwrap_err();
        assert!(
            err.contains("linear value `x` is never consumed"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_poly_local_read_twice_is_use_after_move() {
        // Reading a non-`Copy` local a second time is use-after-move: the
        // polymorphic checker rejects it as the monomorphic sibling rejects
        // `( ^i64 -- ^i64 ^i64 ) | x | x x ;`, naming the variable.
        let err = check_src(": twice ( 'T -- 'T 'T ) | x | x x ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("local `x`"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_local_rebound_while_in_scope_is_error() {
        // R4 twin of the monomorphic rebinding rejection: a second `| x |`
        // while `x` is still in scope would orphan the first binding, leaking
        // the non-`Copy` value parked in it. Reject at compile time, naming the
        // variable, exactly as `( ^i64 ^i64 -- ^i64 ) | x | | x | x ;` is.
        let err =
            check_src(": shadow ( 'T 'T -- 'T ) | x | | x | x ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("already bound"), "unexpected message: {err}");
        assert!(err.contains('x'), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_duplicate_local_in_bind_group_is_error() {
        // A name repeated inside one bind group (`| x x |`) orphans the first
        // binding before the cross-group rebind guard can see it: the poly
        // checker rejects it as the monomorphic sibling rejects
        // `( ^i64 ^i64 -- ^i64 ) | x x | x ;`, naming the variable.
        let err = check_src(": bad ( 'T 'T -- 'T ) | x x | x ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("duplicate local"), "unexpected message: {err}");
        assert!(err.contains('x'), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_local_named_after_variant_is_error() {
        // A local named after a registered variant would make the clause-vs-
        // locals `|` disambiguation ambiguous: the poly binder rejects it as
        // the monomorphic sibling `( i64 i64 -- i64 )` of the same body does,
        // naming the collision.
        let err = check_src(
            "type: Maybe | None | Some v i64 ;\n: f ( 'T i64 -- 'T ) drop | Some | Some ;\n: main ( -- ) 1 2 f drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("collides with the variant name `Some`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_poly_body_with_if_accepts_choose() {
        // T1: a polymorphic body may branch. `choose` consumes `a` and `b` on
        // both arms but at different sites; the move-join must recognise
        // `Moved`+`Moved` as consumed-once (not a leak), or `choose` would be
        // wrongly rejected at the word end (M1).
        assert!(
            check_src(
                ": choose ( 'T 'T bool -- 'T ) | a b flag | flag if a b drop else b a drop end ;\n: main ( -- ) 1 2 true choose drop ;",
            )
            .is_ok(),
            "choose should type-check"
        );
    }

    #[test]
    fn check_poly_arm_local_unconsumed_is_error() {
        // T2: `y` is bound inside the `then` arm and never consumed in it;
        // `leave_arm` must catch the arm-local leak (M2).
        let err = check_src(
            ": arm_leak ( 'T 'T bool -- 'T ) | a b flag | flag if a b | y | else a drop b end ;\n: main ( -- ) ;",
        )
        .unwrap_err();
        assert!(err.contains('y'), "names the arm-local: {err}");
        assert!(err.contains("never consumed"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_if_moved_on_both_arms_is_accepted() {
        // T3: `a`/`b` consumed on both arms (`Moved`+`Moved` => `Moved`), so
        // nothing leaks at the word end.
        assert!(
            check_src(
                ": both ( 'T 'T bool -- ) | a b flag | flag if a drop b drop else b drop a drop end ;\n: main ( -- ) ;",
            )
            .is_ok(),
            "both should type-check"
        );
    }

    #[test]
    fn check_poly_if_moved_on_one_arm_leaks() {
        // T4: `x` consumed on the `then` arm only (`Moved`+`Live` =>
        // `MaybeMoved`), which the leak check must count as still-unconsumed
        // (M3).
        let err =
            check_src(": one ( 'T bool -- ) | x flag | flag if x drop else end ;\n: main ( -- ) ;")
                .unwrap_err();
        assert!(err.contains('x'), "names the leaked local: {err}");
        assert!(err.contains("never consumed"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_if_moved_on_neither_arm_leaks() {
        // T5: `x` untouched on both arms (`Live`+`Live` => `Live`); a value
        // parked in a local across an `if` still leaks at the word end (M4).
        let err = check_src(": none ( 'T bool -- ) | x flag | flag if else end ;\n: main ( -- ) ;")
            .unwrap_err();
        assert!(err.contains('x'), "names the leaked local: {err}");
        assert!(err.contains("never consumed"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_if_condition_not_bool_is_error() {
        // T6: the `if` condition must be `bool`; here the popped condition is
        // the type variable `'T`, so the condition guard fires before anything
        // else (an output-mismatch never mentions `if`).
        let err = check_src(": bad ( 'T 'T -- 'T ) if drop else drop end ;\n: main ( -- ) ;")
            .unwrap_err();
        assert!(err.contains("if"), "names the `if`: {err}");
        assert!(err.contains("'T"), "names the variable condition: {err}");
    }

    #[test]
    fn check_poly_if_branch_depth_mismatch_is_error() {
        // T7: the arms leave different stack depths (then: 1, else: 2). `'T`
        // carries a `Copy` bound so the repeated reads are not use-after-move,
        // leaving the depth mismatch as the sole failure this test proves.
        let err = check_src(
            ": bad ( 'T: Copy bool -- 'T ) | x flag | flag if x else x x end ;\n: main ( -- ) ;",
        )
        .unwrap_err();
        assert!(
            err.contains("different stack depths"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_poly_if_use_after_join_is_error() {
        // T8: both arms consume `x` (the join is `Moved`), so the `x drop`
        // after `end` is a second read: use-after-move, not a leak.
        let err = check_src(
            ": bad ( 'T bool -- ) | x flag | flag if x drop else x drop end x drop ;\n: main ( -- ) ;",
        )
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains('x'), "names the moved local: {err}");
    }

    #[test]
    fn check_poly_dup_of_variable_element_array_names_type_variable() {
        // R7/`poly_copy_gate` array arm: `dup` of an array whose element is an
        // unbounded `'T` recurses to the element and names the variable, not a
        // fabricated `i64`.
        let err =
            check_src(": bad ( ['T 'N] -- ['T 'N] ['T 'N] ) dup ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Copy"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_dup_of_linear_element_array_names_element_type() {
        // `poly_copy_gate` array arm: `dup` of a length-variable array whose
        // element is a concrete linear struct names that struct, never `i64`.
        let err = check_src(&format!(
            "{SPY}: bad ( [Spy 'N] -- [Spy 'N] [Spy 'N] ) dup ;\n: main ( -- ) ;"
        ))
        .unwrap_err();
        assert!(err.contains("Spy"), "unexpected message: {err}");
        assert!(err.contains("linear"), "unexpected message: {err}");
    }

    #[test]
    fn check_two_output_word_interns_its_return_bundle() {
        // R8/R10: a word with two outputs gets a bundle struct in the same
        // registry the layout pass reads, flagged as a bundle and carrying the
        // output tuple in order (deepest output first).
        let module = checked_module(": pair ( -- i64 bool ) 1 true ; : main ( -- ) ;");
        let bundles: Vec<&StructDecl> = module.structs.iter().filter(|d| d.is_bundle).collect();
        assert_eq!(bundles.len(), 1);
        assert_eq!(
            bundles[0]
                .fields
                .iter()
                .map(|(_, ty)| *ty)
                .collect::<Vec<Type>>(),
            vec![Type::I64, Type::Bool]
        );
    }

    #[test]
    fn check_one_output_word_interns_no_bundle() {
        // R2: nothing changes for a word the aggregate ABI does not apply to.
        let module = checked_module(": inc ( i64 -- i64 ) 1 + ; : main ( -- ) ;");
        assert!(module.structs.iter().all(|d| !d.is_bundle));
    }

    #[test]
    fn check_two_words_of_one_output_shape_share_one_bundle() {
        // R8: interning dedups structurally on the output tuple, so two words
        // of the same shape share a bundle and a differing shape gets its own.
        let module = checked_module(
            ": pair ( i64 -- i64 i64 ) dup ;\n\
             : twice ( i64 -- i64 i64 ) dup ;\n\
             : flags ( -- i64 bool ) 1 true ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(module.structs.iter().filter(|d| d.is_bundle).count(), 2);
    }

    #[test]
    fn check_gcd_is_ok() {
        let src = std::fs::read_to_string("examples/gcd.sth").unwrap();
        check_src(&src).unwrap();
    }

    #[test]
    fn check_factorial_is_ok() {
        let src = std::fs::read_to_string("examples/factorial.sth").unwrap();
        check_src(&src).unwrap();
    }

    #[test]
    fn check_lerp_is_ok() {
        let src = std::fs::read_to_string("examples/lerp.sth").unwrap();
        check_src(&src).unwrap();
    }

    #[test]
    fn str_and_cstr_are_copy_and_storable() {
        // Criterion 15/R10: `dup` is accepted on both, and a `str` is
        // storable in a struct field (never seen as containing a
        // reference, and Copy, so no linearity obligation on the field).
        let src = "type: Box s str ;\n\
: main ( -- )\n  \"hi\" dup drop drop\n  \"hi\" cstr dup drop drop\n  \"hi\" Box drop ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_extern_redeclaring_a_word_is_error() {
        // Criterion 5/R1: an `extern:` naming an already-registered word (a
        // user `:` word here) is a located error.
        let src = ": foo ( i64 -- i64 ) ;\nextern: foo ( i64 -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("foo"), "unexpected message: {err}");
        assert!(err.contains("redeclares"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_redeclaring_a_builtin_is_error() {
        // Criterion 5/R1: every builtin `check_term` dispatches by name before
        // the env lookup, plus the `>`-prefixed conversion family. None is in
        // `builtin_table`, so without the `BUILTIN_WORDS` gate the declaration
        // would be accepted, never consulted, and silently do nothing.
        for name in BUILTIN_WORDS.iter().copied().chain([">u8", ">f64"]) {
            let src = format!("extern: {name} ( i64 -- i64 ) \"s\" ;");
            let Err(err) = check_src(&src) else {
                panic!("`extern: {name}` was accepted");
            };
            assert!(
                err.contains("redeclares"),
                "unexpected message for `{name}`: {err}"
            );
        }
    }

    #[test]
    fn check_extern_shadowing_a_builtin_does_not_change_its_meaning() {
        // R1's reason for existing: before the gate, this compiled, and `dup`
        // at the call site still meant the builtin with no diagnostic at all.
        let src = "extern: dup ( i64 -- i64 ) \"mydup\" ;\n: main ( -- ) 1 dup . . ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("redeclares"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_registers_its_effect_at_call_sites() {
        // Criterion 4/R1: registration is what makes the existing arity and
        // type checks apply to a foreign call unchanged. Parsing it is not
        // enough, so assert the effect is actually consulted.
        let ok =
            "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- ) \"hi\" cstr strlen . ;";
        check_src(ok).unwrap();
        let underflow = "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- ) strlen . ;";
        let err = check_src(underflow).unwrap_err();
        assert!(err.contains("strlen"), "unexpected message: {err}");
        let wrong_type =
            "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- ) true strlen . ;";
        let err = check_src(wrong_type).unwrap_err();
        assert!(err.contains("strlen"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_redeclaring_another_extern_is_error() {
        let src = "extern: foo ( i64 -- i64 ) \"foo\" ;\nextern: foo ( i64 -- i64 ) \"bar\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("redeclares"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_overload_on_non_struct_input_is_error() {
        // Criterion 5/R1: an enum, an array, or a scalar input is rejected
        // exactly as a non-struct input would be, with a located error.
        let enum_input = "type: E | V ; : drop ( E -- ) drop ;";
        let err = check_src(enum_input).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(err.contains("type:"), "unexpected message: {err}");

        let array_input = ": drop ( [i64 4] -- ) drop ;";
        let err = check_src(array_input).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");

        let scalar_input = ": drop ( i64 -- ) drop ;";
        let err = check_src(scalar_input).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_overload_with_wrong_arity_is_error() {
        // R1: a `drop` overload declaring anything other than exactly one
        // input is a located error, distinct from the non-struct-input and
        // output rejections tested above.
        let src = "type: T x i64 ; : drop ( T T -- ) drop drop ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(
            err.contains("must declare exactly one input"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_drop_overload_with_output_is_error() {
        // Criterion 6/R1: a `drop` overload declaring an output is a located
        // error, regardless of whether it also declares an input.
        let src = "type: T x i64 ; : drop ( T -- i64 ) drop 0 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(err.contains("output"), "unexpected message: {err}");
    }

    #[test]
    fn check_duplicate_drop_overload_for_one_struct_is_error() {
        // Criterion 7/R1: two `drop` overloads for the same struct id is a
        // located error naming that struct, even though the two words'
        // bodies are otherwise unrelated. Both bodies destructure rather
        // than self-recurse: a self-recursive body would let R6's own
        // recursion check produce a message containing both "T" and "drop"
        // even if the duplicate-override rejection this test targets were
        // deleted entirely, since `find_drop_overloads` runs and returns
        // before either body is ever checked.
        let src = "type: T x i64 ; : drop ( T -- ) | a | a T>x drop ; \
                   : drop ( T -- ) | a | a T>x drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("`T` already defines its own `drop`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_drop_overloads_for_different_structs_both_land_in_the_registry() {
        // Criterion 16's check-side half: two overrides for different
        // structs coexist with distinct `StructId` keys, with no collision
        // reported (the module checks fine), and the registry carries one
        // entry per struct.
        let src = "type: A x i64 ; type: B y i64 ; \
                   : drop ( A -- ) | a | a A>x . ; : drop ( B -- ) | b | b B>y . ; \
                   : main ( -- ) 1 A drop 2 B drop ;";
        check_src(src).unwrap();

        let tokens = crate::lexer::lex(src).unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let registry = find_drop_overloads(&module.words, &module.structs).unwrap();
        assert_eq!(
            registry.len(),
            2,
            "expected one entry per struct: {registry:?}"
        );
    }

    #[test]
    fn check_drop_overloads_are_excluded_from_env() {
        // Stage-test obligation (criterion 16's check-side half): neither
        // override lands in `env` under the shared literal name `"drop"` --
        // if it did, the second override registered would silently clobber
        // the first with no diagnostic, since `check`'s env-registration
        // loop has no redeclaration check for ordinary `:` words the way
        // `check_extern_decls` has for `extern:`. Mirrors `check`'s own
        // filtered registration loop rather than calling it directly, since
        // `env` is internal to `check`.
        let src = "type: A x i64 ; type: B y i64 ; \
                   : drop ( A -- ) drop ; : drop ( B -- ) drop ; \
                   : main ( -- ) 1 A drop 2 B drop ;";
        let tokens = crate::lexer::lex(src).unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let registry = find_drop_overloads(&module.words, &module.structs).unwrap();
        let overload_indices: HashSet<usize> = registry.values().copied().collect();
        let mut env: HashMap<String, Sig> = HashMap::new();
        for (idx, word) in module.words.iter().enumerate() {
            if overload_indices.contains(&idx) {
                continue;
            }
            env.insert(word.name.clone(), sig_of(&word.effect));
        }
        assert!(
            !env.contains_key("drop"),
            "a `drop` overload leaked into env: {env:?}"
        );
    }

    #[test]
    fn check_drop_overload_with_self_recursive_struct_is_still_a_declaration_error_not_overflow() {
        // R1's ordering-hazard caveat: a self-recursive struct with a
        // malformed `drop` override naming that very struct (here, an
        // extra output) must still produce this pre-pass's own located
        // diagnostic, not overflow the stack inside `is_copy`/
        // `check_recursion` -- the pre-pass runs before `check_types`
        // (where `check_recursion` lives) and never calls `is_copy` on the
        // declared input type itself.
        let src = "type: Loop | Wrap next Loop | End ; : drop ( Loop -- i64 ) drop 0 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(err.contains("output"), "unexpected message: {err}");
    }

    /// A checked module, for the tests that read a type fact back out of the
    /// registries rather than only asserting a diagnostic.
    fn checked_module(src: &str) -> Module {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        module
    }

    /// `File`, whose only field is an `i64`, with a `drop` overload: the shape
    /// every R3/R4 test turns on, since the structural fold alone would call
    /// it `Copy`.
    const FILE_RESOURCE: &str = "type: File fd i64 ; : drop ( File -- ) | f | f File>fd . ;";

    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is (R3),
    /// not by any compiler-known bit. Always the first struct in a source
    /// string that uses it, so every other struct's `StructId` shifts up by
    /// one relative to a spy-free program.
    const SPY_DEF: &str =
        "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy>tag . ;\n";

    fn struct_ty(module: &Module, name: &str) -> Type {
        let idx = module
            .structs
            .iter()
            .position(|s| s.name == name)
            .expect("declared struct");
        Type::Struct(StructId::from_index(idx), module.structs[idx].name_static)
    }

    #[test]
    fn check_struct_with_drop_overload_is_linear() {
        // Criterion 1/R3: the override forces linearity, so a struct whose
        // every field is `Copy` is not `Copy`. Without the override the same
        // declaration folds to `Copy`, which is what makes this a real
        // decision rather than a restatement of the field fold.
        let module = checked_module(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let file = struct_ty(&module, "File");
        assert!(!is_copy(
            file,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        assert!(is_linear(
            file,
            &module.structs,
            &module.enums,
            &module.arrays
        ));

        let plain = checked_module("type: File fd i64 ; : main ( -- ) 1 File drop ;");
        assert!(is_copy(
            struct_ty(&plain, "File"),
            &plain.structs,
            &plain.enums,
            &plain.arrays
        ));
    }

    #[test]
    fn check_dup_of_drop_overload_type_names_the_cause() {
        // Criterion 2/R4: the reason-carrying cause, in both `Ctx` arms. The
        // generic linear wording ("no bits to copy") would be actively
        // misleading here: `File`'s bits are one plain `i64`, and its own
        // `: drop` declaration is the whole reason they may not be copied.
        let err = check_src(&format!(
            "{FILE_RESOURCE} : main ( -- ) 1 File dup drop drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
        assert!(
            err.contains("`File` is linear because it defines `drop`"),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains("no bits to copy"),
            "the generic linear cause was used: {err}"
        );

        // The `Ctx::Line` arm: the same fact reaches a bare REPL line, whose
        // carried `File` slot is linear for the same reason.
        let module = checked_module(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let tokens = lex("dup").unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        let err = infer_line(
            &terms,
            &[struct_ty(&module, "File")],
            &builtin_table(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &module.structs,
            &module.enums,
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(
            err.contains("`File` is linear because it defines `drop`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_unconsumed_all_copy_resource_at_word_end_is_error() {
        // Criterion 3/R3: the forgotten-disposal check inherits the forced
        // linearity, so an all-`Copy`-fields resource left bound at the end of
        // a body is an error naming it.
        let err = check_src(&format!("{FILE_RESOURCE} : main ( -- ) 1 File | f | ;")).unwrap_err();
        assert!(
            err.contains("linear value `f` is never consumed"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`File`"), "unexpected message: {err}");
    }

    #[test]
    fn check_double_drop_of_all_copy_resource_is_use_after_move_error() {
        // Criterion 4/R3: a second `drop` of the same resource is a compile
        // error rather than a runtime double-close, which is the whole point
        // of forcing linearity on a struct the field fold calls `Copy`.
        let err = check_src(&format!(
            "{FILE_RESOURCE} : main ( -- ) 1 File | f | f drop f drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("local `f`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_body_must_consume_linear_fields() {
        // Criterion 12/R5/R9: an override body is checked like any other word
        // body, so a resource holding a linear field is already forced to
        // account for it -- no scalar-only restriction, and no new check.
        let src = format!(
            "{SPY_DEF}type: Inner s Spy ; type: Res i Inner ; \
             : drop ( Res -- ) | r | r Res> drop ; \
             : main ( -- ) 1 Spy Inner Res drop ;"
        );
        check_src(&src).unwrap();

        let forgotten = format!(
            "{SPY_DEF}type: Inner s Spy ; type: Res i Inner ; \
             : drop ( Res -- ) | r | ; \
             : main ( -- ) 1 Spy Inner Res drop ;"
        );
        let err = check_src(&forgotten).unwrap_err();
        assert!(
            err.contains("linear value `r` is never consumed"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_drop_body_direct_self_recursion_is_error() {
        // Criterion 8/R6: a `drop` body that drops its own receiver is a
        // cycle of length one. The message names the chain and `File>` as the
        // remedy, since destructuring is what the user has to do instead.
        let src = "type: File fd i64 ; : drop ( File -- ) drop ; : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`File>`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_body_indirect_self_recursion_through_helper_is_error() {
        // Criterion 9/R6: the same rejection through one helper word, which is
        // why this is reachability over the whole call graph rather than a
        // self-call test. The chain names the helper it goes through.
        let src = "type: File fd i64 ; \
                   : shut ( File -- ) drop ; \
                   : drop ( File -- ) shut ; \
                   : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`shut`"), "unexpected message: {err}");
        assert!(err.contains("`File>`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_body_recursion_inside_an_if_arm_is_error() {
        // R6: the call graph is over calls in *any* position, so the walker
        // has to visit both `if` arms and every term after them --
        // `tail_position_calls` only ever reads `terms.last()`, and would see
        // neither of these.
        let src = "type: File fd i64 ; \
                   : shut ( File -- ) drop ; \
                   : drop ( File -- ) | f | true if f shut else f shut end 1 . ; \
                   : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`shut`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_of_copy_scalar_inside_drop_body_is_not_a_cycle() {
        // Criterion 10/R6: the dogfood's own shape. Its body ends in a `drop`
        // of the `Copy` `i64` its extern call returns, which a name-keyed
        // graph would read as a call to the override itself and reject.
        let src = "type: File fd i64 ; \
                   : drop ( File -- ) | f | f File>fd drop ; \
                   : main ( -- ) 1 File drop ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_drop_of_different_resource_inside_another_drop_body_is_ok() {
        // Criterion 11/R6: dispatch is per struct id, so `drop@A` disposing a
        // `B` is an edge to `drop@B` and nothing more -- no cycle, since
        // `drop@B` reaches nothing back.
        let src = "type: A x i64 ; type: B y i64 ; \
                   : drop ( A -- ) | a | a A>x B drop ; \
                   : drop ( B -- ) | b | b B>y drop ; \
                   : main ( -- ) 1 A drop ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_drop_body_recursion_through_a_containing_aggregate_is_error() {
        // Criterion 21/R6 case (b): `Box` has no override, so dropping one
        // runs generic field glue that disposes its `File` field through
        // `File`'s own override -- unbounded recursion at runtime, invisible
        // to a graph that only looked at directly dropped types.
        let src = "type: File fd i64 ; type: Box f File ; \
                   : drop ( File -- ) | f | f Box drop ; \
                   : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`File>`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_of_an_overridden_aggregate_does_not_walk_its_fields() {
        // R6: case (b) must not fire when the dropped type is *itself*
        // overridden. Dropping a `B` runs `B`'s own body, never `B`'s field
        // glue, so walking into its `A` field here would fabricate an edge
        // `drop@A -> drop@A` and reject a program that terminates: `drop@B`
        // destructures its `A` field rather than dropping it.
        let src = "type: A x i64 ; type: B a A ; \
                   : drop ( A -- ) | a | a A>x drop 1 A B drop ; \
                   : drop ( B -- ) | b | b B>a A>x drop ; \
                   : main ( -- ) 1 A drop ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_drop_body_sharing_a_helper_with_another_word_is_not_a_cycle() {
        // R6: reachability is over the whole call graph, so a helper called
        // both from an override and from elsewhere must not read as a cycle
        // just for being reachable from two places.
        let src = "type: File fd i64 ; \
                   : show ( i64 -- ) . ; \
                   : drop ( File -- ) | f | f File>fd show ; \
                   : main ( -- ) 1 File drop 2 show ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_a_word_named_drop_contributes_no_tail_call_edge() {
        // A `drop` term never resolves to a user word (`check_shuffle`
        // intercepts it first), so the tail-call graph must not treat one as a
        // call to a `drop` overload: `helper`'s trailing `drop` of an `i64`
        // would otherwise close a fabricated mutual cycle with the override
        // that tail-calls `helper`.
        let src = "type: T x i64 ; \
                   : helper ( i64 -- ) drop ; \
                   : drop ( T -- ) | t | t T>x helper ; \
                   : main ( -- ) 1 T drop ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_extern_accepts_the_full_r2_boundary_type_set() {
        // R2: the numeric tower, `bool`, `&T`/`&!T`, and `cstr` may all cross
        // an `extern:` boundary in either position.
        let src = "extern: f1 ( i64 u8 usize isize f64 f32 bool -- i64 ) \"f1\" ;\nextern: f2 ( &i64 &!i64 -- i64 ) \"f2\" ;\nextern: f3 ( cstr -- cstr ) \"f3\" ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_extern_with_str_parameter_is_error() {
        // R2/R7: a `str` is a descriptor handle (R4), not a scalar or a
        // single opaque `Ptr`, so it matches no C parameter; the rejection
        // names the total conversion to `cstr`.
        let src = "extern: f ( str -- i64 ) \"f\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("matches no C parameter") && err.contains("`cstr`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_extern_returning_str_is_error() {
        // R11: a returned `str` would be one not built from a literal, which
        // is the invariant R10's `Copy`/non-escaping status rests on.
        let src = "extern: f ( -- str ) \"f\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("cannot return a `str`") && err.contains("static data only"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_extern_with_aggregate_parameter_is_error() {
        // Criterion 11/R3: an owned aggregate (struct/enum/array) as an
        // `extern:` input is rejected at the declaration.
        let src = "type: Point x i64 y i64 ;\nextern: foo ( Point -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("owned aggregate"), "unexpected message: {err}");
        assert!(err.contains("Point"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_with_array_parameter_is_error() {
        let src = "extern: foo ( [i64 4] -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("owned aggregate"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_with_owned_pointer_parameter_is_error() {
        // R3: `^T` is an owned aggregate too, rejected in input position
        // with the generic aggregate message (the output-specific
        // "forge ownership" message is only for the output position).
        let src = "extern: foo ( ^i64 -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("owned aggregate"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_cannot_express_a_variadic_c_function() {
        // R3: `extern:`'s grammar has no syntax for a variadic parameter
        // list, so `printf` cannot be usefully declared: only a fixed
        // effect can be spelled, e.g. one `cstr` and nothing else.
        let src = "extern: printf ( cstr -- i64 ) \"printf\" ;";
        check_src(src).unwrap();
        let err =
            crate::parser::parse(&lex("extern: printf ( cstr ... -- i64 ) \"printf\" ;").unwrap())
                .unwrap_err();
        assert!(
            err.contains("unknown type `...`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_extern_multi_output_is_error() {
        // Criterion 18/R8: a two-output `extern:` describes no C prototype.
        // Unrejected it lowered to a discarded result and panicked in the
        // *next* consumer of the value that was never pushed, naming the
        // wrong term; the diagnostic sits at the declaration instead.
        let src = "extern: two ( i64 -- i64 i64 ) \"two\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("`extern: two` declares 2 outputs")
                && err.contains("no C function returns more than one value"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_extern_returning_owned_pointer_is_error() {
        // Criterion 12/R3: an `extern:` returning `^T` is rejected: it would
        // forge ownership of memory the allocator did not hand out.
        let src = "extern: foo ( i64 -- ^i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("forge ownership"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_returning_a_reference_is_error() {
        // Criterion 13/R3: reusing the existing no-declared-output-reference
        // message rather than duplicating it.
        let src = "extern: foo ( i64 -- &i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("a reference cannot be stored"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_str_where_cstr_declared_is_error() {
        // Criterion 10/R7: passing a `str` where a `cstr` is declared is a
        // type error naming the conversion, not a silent pointer pun.
        let src =
            "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- )\n  \"hi\" strlen drop ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("cstr"), "unexpected message: {err}");
        assert!(
            err.contains("convert it explicitly"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_len_on_str_types_as_usize() {
        // R8: `check_str_word` claims `len` on a `str` operand before the
        // array path ever sees it, consuming the `str` and typing the result
        // `usize` (not the array `len`'s non-consuming signature).
        check_src(": w ( -- usize ) \"hi\" len ;").unwrap();
    }

    #[test]
    fn check_stack_underflow_is_error() {
        let src = ": oops ( i64 -- i64 )\n  | a | a a + + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("oops"));
        assert!(err.contains("`+`"));
        assert!(err.contains("needs 2 values"));
        assert!(err.contains("holds 1"));
        assert!(err.contains("( i64 -- i64 )"));
    }

    #[test]
    fn check_branch_depth_mismatch_is_error() {
        let src = ": w ( bool -- i64 ) if 1 1 else 1 end ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different stack depths"));
    }

    #[test]
    fn check_branch_join_types_agree_ok() {
        // Both arms leave a single `i64`: the join unifies cleanly.
        check_src(": w ( bool -- i64 ) if 1 else 2 end ;").unwrap();
    }

    #[test]
    fn check_branch_join_type_mismatch_is_error() {
        // `then` leaves an `i64`, `else` leaves a `bool`: same depth, different type.
        let src = ": w ( bool -- i64 ) if 1 else true end ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different types"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_declared_output_mismatch_is_error() {
        let src = ": w ( -- i64 ) 1 1 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("body leaves 2 values"));
        assert!(err.contains("declares 1 outputs"));
    }

    #[test]
    fn scope_leave_reports_the_unconsumed_linear_local() {
        // `leave_block`'s diagnostic depends on this return value. Extent is
        // enforced by checking each arm on its own `scope.clone()`, not by the
        // `bound` truncation `leave` performs as a side effect, so the extent
        // rule is covered end to end by the goldens rather than here.
        let mut scope = Scope::default();
        let prov = &mut Provenance::default();
        scope.bind("a", Slot::computed(Type::I64), false, prov);
        let depth = scope.depth();
        scope.bind("b", Slot::computed(Type::I64), false, prov);
        assert!(scope.leave(depth).is_none(), "a Copy local leaves cleanly");

        // R6: a linear name leaving scope with its value still held is what the
        // block-end firing site reports. `bind`'s `linear` flag is passed
        // explicitly by the caller (not derived from the `Type` via
        // `is_copy`), so any type distinct from `a`'s suffices here.
        scope.bind("s", Slot::computed(Type::Bool), true, prov);
        let leaked = scope.leave(depth).expect("an unconsumed linear local");
        assert_eq!((leaked.0.as_str(), leaked.1), ("s", Type::Bool));
        assert_eq!(leaked.2, MoveState::Live);
    }

    #[test]
    fn check_word_duplicate_local_is_error() {
        let src = ": w ( i64 i64 -- i64 ) | a a | a ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("duplicate local"), "unexpected message: {err}");
        assert!(err.contains("`a`"), "unexpected message: {err}");
        assert!(err.contains("`w`"), "unexpected message: {err}");
    }

    #[test]
    fn check_main_linear_output_is_error() {
        let err = check_src(&format!("{SPY_DEF}: main ( -- Spy ) 7 Spy ;")).unwrap_err();
        assert!(
            err.contains("cannot declare a linear type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_main_linear_input_is_error() {
        let err = check_src(&format!("{SPY_DEF}: main ( Spy -- ) | s | s drop ;")).unwrap_err();
        assert!(
            err.contains("cannot declare a linear type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_duplicate_word_in_one_file_is_error() {
        // Two `push` words in one file used to reach codegen silently (the
        // env-population loop kept only the last), surfacing only as a bare
        // linker `symbol already defined` error at the very end of the
        // pipeline. Now it is a located compiler diagnostic.
        let err =
            check_src(": push ( -- i64 ) 1 ;\n: push ( -- i64 ) 2 ;\n: main ( -- ) push drop ;")
                .unwrap_err();
        assert!(
            err.contains("duplicate word `push`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("first defined at line 1"),
            "names the first definition's location too: {err}"
        );
    }

    #[test]
    fn check_main_copy_effect_is_ok() {
        check_src(": main ( i64 -- i64 ) 1 + ;").unwrap();
        // The misfire risk is `is_copy`'s recursive struct/enum arms, not the
        // scalar arm: a Copy struct in `main`'s effect must not be rejected.
        check_src("type: P a i64 b i64 ; : main ( P -- ) P> drop drop ;").unwrap();
    }

    #[test]
    fn check_clause_body_duplicate_local_is_error() {
        let src = "type: Shape | Circle r f64 s f64 ;
             : area ( Shape -- f64 ) | Circle | a a | a ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("duplicate local"), "unexpected message: {err}");
        assert!(err.contains("`a`"), "unexpected message: {err}");
        assert!(err.contains("`area`"), "unexpected message: {err}");
    }

    #[test]
    fn check_unknown_word_is_error() {
        let src = ": w ( i64 -- i64 ) frobnicate ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown word"));
        assert!(err.contains("frobnicate"));
    }

    #[test]
    fn check_locals_exceed_inputs_is_error() {
        let src = ": w ( i64 -- i64 ) | a b | a ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("locals bind"));
    }

    #[test]
    fn check_type_propagates_through_body_expected() {
        // `0 >` yields a bool that `if` consumes; both arms leave an i64.
        check_src(": sign ( i64 -- i64 ) 0 > if 1 else 0 end ;").unwrap();
    }

    #[test]
    fn check_if_condition_not_bool_is_error() {
        let src = ": w ( -- i64 ) 5 if 1 else 2 end ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("expected `bool`"), "unexpected message: {err}");
        assert!(err.contains("found `i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_operand_type_mismatch_is_error() {
        let src = ": w ( -- i64 ) true 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_declared_output_type_mismatch_is_error() {
        let src = ": w ( i64 -- bool ) 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("type mismatch"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shuffle_dup_bool_is_type_transparent() {
        // `dup` of a `bool` yields two `bool`s and satisfies the declaration.
        check_src(": w ( bool -- bool bool ) dup ;").unwrap();
    }

    #[test]
    fn check_arith_same_width_ok() {
        check_src(": w ( -- i32 ) 1 >i32 2 >i32 + ;").unwrap();
    }

    #[test]
    fn check_arith_mixed_width_is_error() {
        // An `i32` and an `i64` fed to `+` names both differing types, via
        // the operand-pair-mismatch diagnostic specifically (not just any error
        // that happens to mention both type names).
        let src = ": f ( -- i32 ) 1 >i32 5 + ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_cmp_mixed_sign_is_error() {
        // `u8` and `i8` fed to `<` names both differing operand types, via
        // the same operand-pair-mismatch diagnostic.
        let src = ": w ( -- bool ) 200 >u8 5 >i8 < ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`u8`"), "unexpected message: {err}");
        assert!(err.contains("`i8`"), "unexpected message: {err}");
    }

    #[test]
    fn check_arith_mixed_int_float_is_error() {
        // X1: mixed int/float arithmetic names both operand types.
        let src = ": f ( -- f64 ) 1 >i32 5.0 + ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_cmp_mixed_float_width_is_error() {
        // X2: mixed float-width comparison names both operand types.
        let src = ": w ( -- bool ) 1.0 >f32 2.0 < ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`f32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_div_same_float_type_ok() {
        check_src(": w ( -- f64 ) 1.0 2.0 / ;").unwrap();
    }

    #[test]
    fn check_div_on_ints_is_error() {
        // X3: `/` requires floats; integer operands are a sharp error.
        let src = ": w ( -- i64 ) 4 2 / ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`/`"), "unexpected message: {err}");
        assert!(err.contains("float"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_mod_same_int_type_ok() {
        check_src(": w ( -- i64 ) 5 2 mod ;").unwrap();
    }

    #[test]
    fn check_mod_on_floats_is_error() {
        // X4: `mod` requires integers; float operands are a sharp error.
        let src = ": w ( -- f64 ) 5.0 2.0 mod ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`mod`"), "unexpected message: {err}");
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_max_same_int_type_ok() {
        check_src(": w ( -- i64 ) 3 5 max ;").unwrap();
    }

    #[test]
    fn check_max_on_floats_is_error() {
        // X9: `max` is integer-only; a float pair names `max-total`.
        let src = ": w ( -- f64 ) 3.0 5.0 max ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`max`"), "unexpected message: {err}");
        assert!(err.contains("`max-total`"), "unexpected message: {err}");
    }

    #[test]
    fn check_max_total_same_float_type_ok() {
        check_src(": w ( -- f64 ) 3.0 5.0 max-total ;").unwrap();
    }

    #[test]
    fn check_max_total_on_ints_is_error() {
        // X10: `max-total` is float-only; an integer pair names `max`.
        let src = ": w ( -- i64 ) 3 5 max-total ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`max-total`"), "unexpected message: {err}");
        assert!(err.contains("`max`"), "unexpected message: {err}");
    }

    #[test]
    fn check_bitwise_and_or_xor_same_type_ok() {
        check_src(": w ( -- i32 ) 1 >i32 2 >i32 and 3 >i32 or 4 >i32 xor ;").unwrap();
    }

    #[test]
    fn check_bitwise_and_mixed_width_is_error() {
        let src = ": w ( -- i64 ) 1 >i32 2 and ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same integer or bool type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_bitwise_and_or_xor_on_bool_is_ok() {
        // Bool is now an accepted homogeneous operand class for `and`/`or`/`xor`
        // (logical-and on two 0/1 bools coincides with bitwise-and).
        check_src(": w ( -- bool ) true false and true false or drop true false xor drop ;")
            .unwrap();
    }

    #[test]
    fn check_bitwise_and_mixed_bool_int_is_error() {
        let src = ": w ( -- bool ) true 5 and ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same integer or bool type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`bool`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_bitwise_and_on_float_is_error() {
        let src = ": w ( -- f64 ) 3.0 5.0 and ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_not_same_type_ok() {
        check_src(": w ( -- u8 ) 5 >u8 not ;").unwrap();
    }

    #[test]
    fn check_not_on_float_is_error() {
        let src = ": w ( -- f64 ) 3.0 not ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`not`"), "unexpected message: {err}");
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_not_on_bool_is_ok() {
        // `not` is type-directed: on a `bool` it is logical negation, not
        // the integer bitwise complement (R9-ext).
        check_src(": w ( -- bool ) true not ;").unwrap();
    }

    #[test]
    fn check_cmp_le_ge_ne_numeric_same_type_ok() {
        check_src(": w ( -- bool bool bool ) 1 2 <= 1 2 >= 1 2 <> ;").unwrap();
    }

    #[test]
    fn check_cmp_le_ge_ne_on_bool_is_error() {
        // Comparisons stay numeric-only: `bool` is never accepted, even
        // though it now is for `and`/`or`/`xor`.
        let src = ": w ( -- bool ) true false <= ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_cmp_ne_mixed_type_is_error() {
        let src = ": w ( -- bool ) 1 >i32 2 <> ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shl_shr_i64_count_ok() {
        check_src(": w ( -- u8 ) 1 >u8 3 shl ;").unwrap();
        check_src(": w ( -- u8 ) 200 >u8 3 shr ;").unwrap();
    }

    #[test]
    fn check_shl_count_not_i64_is_error() {
        let src = ": w ( -- u8 ) 1 >u8 3 >i32 shl ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`shl`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`i32`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shr_value_not_int_is_error() {
        let src = ": w ( -- f64 ) 3.0 2 shr ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`shr`"), "unexpected message: {err}");
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_is_recognised_as_a_type_name() {
        check_src(": w ( -- usize ) 5 ;").unwrap();
    }

    #[test]
    fn check_usize_arithmetic_and_comparison_ok() {
        check_src(": w ( -- usize ) 5 3 >usize + ;").unwrap();
        check_src(": w ( -- bool ) 5 3 >usize < ;").unwrap();
    }

    #[test]
    fn check_usize_literal_coerces_into_usize_position_ok() {
        // D8: a bare integer literal fills a `usize` position on either side
        // of a homogeneous binary op, no `>usize` required.
        check_src(": w ( -- usize ) 3 >usize 5 + ;").unwrap();
        check_src(": w ( -- usize ) 5 3 >usize + ;").unwrap();
    }

    #[test]
    fn check_usize_computed_value_without_conversion_is_error() {
        // X10: `1 1 +` is a *computed* i64 (no constant folding), so mixing
        // it with a `usize` still needs an explicit `>usize`.
        let src = ": w ( -- usize ) 3 >usize 1 1 + + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("usize"), "unexpected message: {err}");
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_to_int_and_int_to_usize_conversions_ok() {
        check_src(": w ( -- i64 ) 5 >usize >i64 ;").unwrap();
        check_src(": w ( -- usize ) 5 >usize ;").unwrap();
    }

    #[test]
    fn check_usize_print_is_type_directed_ok() {
        check_src(": w ( -- ) 5 >usize . ;").unwrap();
    }

    // Array words: fill / len type-checking.

    #[test]
    fn check_fill_len_happy_path_ok() {
        // `fill` builds `[i64 4]`; `len` is non-consuming (the array stays).
        check_src(": w ( -- ) 7 4 fill len drop drop ;").unwrap();
    }

    #[test]
    fn check_fill_output_type_is_the_array_shape() {
        // `fill` interns `[i64 4]` and the declared output must match it, so
        // this word type-checks with an array-typed output slot (R2/R3/R10).
        check_src(": w ( -- [i64 4] ) 7 4 fill ;").unwrap();
    }

    #[test]
    fn check_len_is_non_consuming_leaves_array_ok() {
        check_src(": w ( [i64 4] -- [i64 4] usize ) | a | a len ;").unwrap();
    }

    #[test]
    fn check_len_on_non_array_is_error() {
        // X8: `len` on a non-array operand names the word and the operand
        // type via `array_word_operand_error`.
        let err = check_src(": w ( i64 -- usize ) len ;").unwrap_err();
        assert!(
            err.contains("`len` requires an array operand"),
            "unexpected message: {err}"
        );
        assert!(err.contains("i64"), "should name the offending type: {err}");
    }

    #[test]
    fn check_constant_index_out_of_range_is_error() {
        // X4/R11: a literal index >= N is a sharp located compile error naming
        // the length and the index. Index (9) and length (4) are deliberately
        // distinct so a swapped-arg diagnostic bug can't hide behind a
        // same-valued assertion.
        let err = check_src(": w ( [i64 4] -- ) | a | &a 9 &> drop ;").unwrap_err();
        assert!(err.contains("out of range"), "unexpected message: {err}");
        assert!(err.contains('9'), "should name the index: {err}");
        assert!(err.contains('4'), "should name the length: {err}");
    }

    #[test]
    fn check_constant_index_at_length_boundary_is_error() {
        // Index == length is the first invalid index (valid range is
        // 0..length-1); this off-by-one boundary is distinct from the
        // gross-violation case above and must be rejected too.
        let err = check_src(": w ( [i64 4] -- ) | a | &a 4 &> drop ;").unwrap_err();
        assert!(err.contains("out of range"), "unexpected message: {err}");
        assert!(err.contains("index 4"), "should name the index: {err}");
        assert!(err.contains("length 4"), "should name the length: {err}");
    }

    #[test]
    fn check_computed_index_without_conversion_is_error() {
        // X10: a computed (non-literal) `i64` index needs an explicit `>usize`.
        let err = check_src(": w ( [i64 4] i64 -- ) | a n | &a n &> drop ;").unwrap_err();
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }

    #[test]
    fn check_fill_non_literal_count_is_error() {
        // M1: the count must be a compile-time literal; a computed count errors.
        let err = check_src(": w ( i64 -- ) | n | 0 n fill drop ;").unwrap_err();
        assert!(err.contains("literal count"), "unexpected message: {err}");
    }

    #[test]
    fn check_fill_zero_count_is_error() {
        // A `fill` count < 1 is invalid (an array length must be >= 1).
        let err = check_src(": w ( -- ) 0 0 fill drop ;").unwrap_err();
        assert!(
            err.contains("length must be >= 1"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_fill_of_linear_element_is_error() {
        // `fill` has no per-slot `Copy` gate today (unlike `dup`/`over`), and
        // array-element linearity isn't tracked transitively, so a linear
        // element is rejected rather than silently replicated/leaked.
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 0 Spy 3 fill drop ;")).unwrap_err();
        assert!(
            err.contains("not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_fill_of_linear_struct_element_is_error() {
        // The same rejection applies transitively: a struct that is linear
        // because one of its fields is (R7) is just as unsupported as a bare
        // `Spy` element.
        let err = check_src(&format!(
            "{SPY_DEF}type: Holder xs Spy ;\n: w ( -- ) 0 Spy Holder 3 fill drop ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Holder`"), "unexpected message: {err}");
    }

    #[test]
    fn check_print_on_array_is_error() {
        // X6/R13: `.` on an array is a sharp located error naming `[T N]`.
        let err = check_src(": w ( -- ) 0 4 fill . ;").unwrap_err();
        assert!(err.contains("[i64 4]"), "should name the array type: {err}");
    }

    #[test]
    fn check_equality_on_array_is_error() {
        // X7/R13: `=` on arrays reaches the operand guard naming the type.
        let err = check_src(": w ( -- bool ) 0 4 fill 0 4 fill = ;").unwrap_err();
        assert!(err.contains("[i64 4]"), "should name the array type: {err}");
    }

    #[test]
    fn check_arithmetic_on_array_is_error() {
        // X7/R13: `+` on arrays reaches the operand guard naming the type
        // (the diagnostic covers `=` *and* arithmetic; both are exercised).
        let err = check_src(": w ( -- [i64 4] ) 0 4 fill 0 4 fill + ;").unwrap_err();
        assert!(err.contains("[i64 4]"), "should name the array type: {err}");
    }

    #[test]
    fn check_two_spellings_of_same_shape_are_one_type_ok() {
        // R8: structural dedup means `[i64 4]` in two positions is one type, so
        // an `[i64 4]` argument satisfies an `[i64 4]`-typed word.
        check_src(
            ": mk ( -- [i64 4] ) 0 4 fill ;\n: use ( [i64 4] -- i64 ) | a | &a 0 &> @ ;\n: w ( -- i64 ) mk use ;",
        )
        .unwrap();
    }

    #[test]
    fn check_value_recursion_through_array_element_is_error() {
        // X5/R14/M3: a struct containing itself via an array element is a
        // recursive definition (infinite size), caught by the DFS.
        let err = check_src("type: Node kids [Node 4] ;").unwrap_err();
        assert!(err.contains("recursive"), "unexpected message: {err}");
        assert!(err.contains("Node"), "should name the cycle: {err}");
    }

    #[test]
    fn check_usize_mixed_with_bool_is_error() {
        // X9: `usize` mixed with a non-coercible operand (`bool`) names both.
        let src = ": w ( -- usize ) 5 >usize true and ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`usize`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_mixed_with_float_is_error() {
        // X9: `usize` mixed with `f64` (both numeric, not coercible).
        let src = ": w ( -- bool ) 5 >usize 1.0 < ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`usize`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_declared_output_needs_conversion_is_error() {
        // X10 at a declared-output position: a computed `i64` doesn't
        // silently satisfy a declared `usize` output.
        let src = ": w ( -- usize ) 1 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("usize"), "unexpected message: {err}");
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }

    #[test]
    fn check_isize_mixed_with_usize_is_error() {
        // `usize` and `isize` are sibling size types but do not coerce
        // into each other; mixing them is a plain type mismatch naming both
        // backticked types.
        let src = ": w ( -- bool ) 5 >usize 3 >isize < ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`usize`"), "unexpected message: {err}");
        assert!(err.contains("`isize`"), "unexpected message: {err}");
    }

    #[test]
    fn check_isize_declared_output_needs_conversion_is_error() {
        // X10 at a declared-output position, mirroring
        // check_usize_declared_output_needs_conversion_is_error: a computed
        // `i64` doesn't silently satisfy a declared `isize` output, and the
        // message names the backticked `isize` form rather than `usize`.
        let src = ": w ( -- isize ) 1 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`isize`"), "unexpected message: {err}");
        assert!(err.contains(">isize"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_branch_merge_keeps_computed_arm_non_coercible_is_error() {
        // A literal in one arm and a computed value in the other must NOT
        // merge to a coercible literal: on the computed arm's runtime path a
        // computed `i64` would fill the `usize` output without `>usize` (X10).
        for src in [
            ": w ( bool -- usize ) if 5 else 1 1 + end ;",
            ": w ( bool -- usize ) if 1 1 + else 5 end ;",
        ] {
            let err = check_src(src).unwrap_err();
            assert!(err.contains("usize"), "unexpected message: {err}");
            assert!(err.contains(">usize"), "unexpected message: {err}");
        }
    }

    #[test]
    fn check_usize_branch_merge_both_literals_coerces_ok() {
        // Both arms leave a literal, so the merged slot stays a coercible
        // literal and fills the `usize` output.
        check_src(": w ( bool -- usize ) if 5 else 6 end ;").unwrap();
    }

    #[test]
    fn check_usize_call_argument_literal_coerces_ok() {
        // A bare literal fills a declared `usize` parameter without `>usize`.
        let src = ": at ( usize -- usize ) ; : w ( -- usize ) 5 at ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_usize_call_argument_computed_needs_conversion_is_error() {
        let src = ": at ( usize -- usize ) ; : w ( -- usize ) 1 1 + at ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("usize"), "unexpected message: {err}");
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }

    #[test]
    fn check_conv_int_to_float_ok() {
        check_src(": w ( -- f64 ) 5 >f64 ;").unwrap();
    }

    #[test]
    fn check_conv_float_to_int_ok() {
        check_src(": w ( -- i64 ) 5.0 >i64 ;").unwrap();
    }

    #[test]
    fn check_conv_float_target_of_bool_is_error() {
        // X5: a conversion to a float target applied to a `bool` source.
        let src = ": w ( -- f64 ) true >f64 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_conv_unknown_float_target_is_error() {
        // X6: `>f128` reads as an unknown conversion target.
        let src = ": w ( -- f64 ) 5.0 >f128 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("f128"), "unexpected message: {err}");
    }

    #[test]
    fn check_float_lit_types_as_f64() {
        check_src(": w ( -- f64 ) 3.14 ;").unwrap();
    }

    #[test]
    fn check_branch_join_float_widths_mismatch_is_error() {
        // `if` branches leaving `f32` vs `f64` disagree at the join (R12).
        let src = ": w ( bool -- f64 ) if 1.0 >f32 else 2.0 end ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different types"), "unexpected message: {err}");
        assert!(err.contains("`f32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_branch_join_float_types_agree_ok() {
        check_src(": w ( bool -- f64 ) if 1.0 else 2.0 end ;").unwrap();
    }

    #[test]
    fn check_shuffle_dup_float_is_type_transparent() {
        check_src(": w ( -- f64 f64 ) 1.0 dup ;").unwrap();
    }

    #[test]
    fn check_conv_from_any_int_ok() {
        check_src(": w ( -- u8 ) 5 >i32 >u8 ;").unwrap();
    }

    #[test]
    fn check_conv_of_bool_is_error() {
        // A conversion applied to `bool` is a type error (X5).
        let src = ": w ( -- i32 ) true >i32 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_declared_output_needs_conversion_is_error() {
        // X3: the literal is `i64`, the declared output is `u8`.
        let src = ": f ( -- u8 ) 5 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`u8`"), "unexpected message: {err}");
    }

    #[test]
    fn check_conv_unknown_target_is_error() {
        // X6: `>i128` reads as an unknown conversion target.
        // (this test predates R10's float target; kept for the integer case)
        let src = ": w ( -- i64 ) 5 >i128 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("i128"), "unexpected message: {err}");
    }

    #[test]
    fn check_shuffle_dup_u8_is_transparent() {
        check_src(": w ( -- u8 u8 ) 5 >u8 dup ;").unwrap();
    }

    #[test]
    fn check_shuffle_swap_mixed_types_is_type_transparent() {
        // `swap` reorders a mixed `bool`/`i64` pair with no fixed signature.
        check_src(": w ( bool i64 -- i64 bool ) swap ;").unwrap();
    }

    #[test]
    fn check_print_accepts_every_printable_scalar() {
        // `.` is type-directed over the whole integer tower, both float
        // widths, and `bool`, not just `i64`.
        check_src(": w ( -- ) 5 . ;").unwrap();
        check_src(": w ( -- ) 5 >u8 . ;").unwrap();
        check_src(": w ( -- ) 5 >i32 . ;").unwrap();
        check_src(": w ( -- ) -1 >u64 . ;").unwrap();
        check_src(": w ( -- ) 3.14 . ;").unwrap();
        check_src(": w ( -- ) 3.14 >f32 . ;").unwrap();
        check_src(": w ( -- ) true . ;").unwrap();
    }

    #[test]
    fn check_print_accepts_str_and_cstr() {
        // `.`'s printable-scalar guard also accepts `str`/`cstr` (R9), matched
        // by name rather than `is_numeric`/`is_bool`, since neither is numeric.
        check_src(": w ( -- ) \"hi\" . ;").unwrap();
        check_src(": w ( -- ) \"hi\" cstr . ;").unwrap();
    }

    #[test]
    fn check_print_on_empty_stack_is_underflow_error() {
        let src = ": w ( -- ) . ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`.`"), "unexpected message: {err}");
        assert!(err.contains("needs 1 values"), "unexpected message: {err}");
    }

    fn infer_src(src: &str, entry: &[Type]) -> Result<Vec<Type>, String> {
        let tokens = lex(src).unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        infer_line(
            &terms,
            entry,
            &builtin_table(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &[],
            &[],
            &HashMap::new(),
            &HashMap::new(),
        )
        .map(|(stack, _insts)| stack)
    }

    #[test]
    fn infer_line_net_effect_expected() {
        assert_eq!(infer_src("2 3 +", &[]).unwrap(), vec![Type::I64]);
    }

    #[test]
    fn infer_line_carries_entry_depth() {
        // `2 +` from a carried `i64`: the literal plus the carried slot are
        // consumed by `+`, leaving one `i64`.
        assert_eq!(infer_src("2 +", &[Type::I64]).unwrap(), vec![Type::I64]);
    }

    #[test]
    fn infer_line_carries_slot_types_expected() {
        // A comparison line leaves a `bool` on the carried stack.
        assert_eq!(infer_src("5 3 >", &[]).unwrap(), vec![Type::Bool]);
    }

    #[test]
    fn line_underflow_against_carried_stack_is_error() {
        let err = infer_src("+", &[Type::I64]).unwrap_err();
        assert!(err.contains("stack underflow"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
        assert!(err.contains("holds 1"), "unexpected message: {err}");
    }

    #[test]
    fn infer_line_unknown_word_is_error() {
        let err = infer_src("frobnicate", &[]).unwrap_err();
        assert!(err.contains("unknown word"), "unexpected message: {err}");
        assert!(err.contains("frobnicate"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_generated_words_flat_struct_ok() {
        check_src(
            "type: Vec2 x i64 y i64 ;
             : main ( -- ) 1 2 Vec2 dup Vec2>x drop Vec2>y drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_struct_generated_words_nested_struct_ok() {
        check_src(
            "type: Vec2 x i64 y i64 ;
             type: Segment from Vec2 to Vec2 ;
             : main ( -- ) 1 2 Vec2 3 4 Vec2 Segment dup Segment>from Vec2>x drop Segment> drop drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_struct_zero_field_registers_only_ctor_and_destructure() {
        check_src("type: Unit ; : main ( -- ) Unit Unit> ;").unwrap();
    }

    #[test]
    fn check_struct_setter_returns_updated_struct_ok() {
        check_src("type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 2 Vec2 3 Vec2<x ;").unwrap();
    }

    #[test]
    fn check_struct_peek_copy_field_leaves_struct_live_ok() {
        // R10: `Vec2|>x` is non-consuming, so the struct is still on the
        // stack for the second peek and the trailing `Vec2>` destructure.
        check_src("type: Vec2 x i64 y i64 ; : main ( -- ) 1 2 Vec2 Vec2|>x drop Vec2> drop drop ;")
            .unwrap();
    }

    #[test]
    fn check_struct_peek_on_linear_field_is_error() {
        // R10: a linear field can't be peeked (workaround: `S>`).
        let err = check_src(&format!(
            "{SPY_DEF}type: Holds a Spy b i64 ; : main ( -- ) 7 Spy 1 Holds Holds|>a drop drop ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("cannot `Holds|>a`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(err.contains("`S>`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_peek_on_wrong_type_is_error() {
        // A peek word applied to a value that isn't its struct: names the
        // peek word and both types, same shape as the getter/setter checks.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- i64 ) 5 Vec2|>x drop ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2|>x"), "unexpected message: {err}");
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_duplicate_type_name_is_error() {
        // X2: two `type:` declarations sharing a name name that type.
        let err = check_src("type: Vec2 x i64 ; type: Vec2 y i64 ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Vec2"), "unexpected message: {err}");
    }

    #[test]
    fn check_recursion_by_value_self_cycle_is_error() {
        // X3/M5: a directly self-referential struct (no `^` anywhere
        // on the cycle) is an error naming the full path (a bare string,
        // no span), and this test itself is proof the checker terminated
        // rather than hung.
        let err = check_src("type: Loop next Loop ;").unwrap_err();
        assert!(
            err.contains("recursive struct"),
            "unexpected message: {err}"
        );
        assert!(err.contains("Loop -> Loop"), "unexpected message: {err}");
    }

    #[test]
    fn check_recursion_by_value_mutual_cycle_is_error() {
        // X3/M5: a mutually-recursive pair of structs, no `^`
        // anywhere, names the full path A -> B -> A.
        let err = check_src("type: A b B ; type: B a A ;").unwrap_err();
        assert!(
            err.contains("recursive struct"),
            "unexpected message: {err}"
        );
        assert!(err.contains("A -> B -> A"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_direct_recursion_is_error_not_hang() {
        // X3/M5: a directly self-referential enum (a variant field of its own
        // type) is an error naming the cycle (bare, no span), and this
        // test's return is proof the DFS terminated rather than hung.
        let err = check_src("type: Loop | Wrap next Loop | End ;").unwrap_err();
        assert!(err.contains("recursive enum"), "unexpected message: {err}");
        assert!(err.contains("Loop"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_mutual_recursion_is_error_not_hang() {
        // X3/M5: a mutually-recursive pair of enums, names both in the cycle.
        let err = check_src("type: A | Ta x B ; type: B | Tb y A ;").unwrap_err();
        assert!(err.contains("recursive enum"), "unexpected message: {err}");
        assert!(err.contains('A'), "unexpected message: {err}");
        assert!(err.contains('B'), "unexpected message: {err}");
    }

    #[test]
    fn check_recursion_cell_cycle_in_struct_field_is_ok() {
        // A `^` edge through a struct field is legal, not just through
        // an enum variant payload -- the rule is about size finiteness, not
        // idiom.
        check_src("type: Node v i64 next ^Node ;").unwrap();
    }

    #[test]
    fn check_recursion_cell_cycle_in_enum_variant_is_ok() {
        // The same `^` cycle acceptance in enum variant position,
        // mirroring check_recursion_cell_cycle_in_struct_field_is_ok.
        check_src("type: List | Nil | Cons v i64 next ^List ;").unwrap();
    }

    #[test]
    fn check_recursion_array_element_cell_is_cut_then_rejected_as_linear() {
        // The `^` edge is cut inside an array element too, so this
        // definition survives the recursion rule and reaches the linear
        // array-element rule instead of "recursive array definition".
        let err = check_src("type: Node kids [^Node 4] ;").unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`^Node`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_enum_mixed_recursion_is_error_not_hang() {
        // D9/X3: a struct field of enum type closing a cycle back to the
        // struct is caught by the combined-graph DFS.
        let err = check_src("type: S f E ; type: E | V g S ;").unwrap_err();
        assert!(err.contains("recursive"), "unexpected message: {err}");
        assert!(err.contains('S'), "unexpected message: {err}");
        assert!(err.contains('E'), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_direct_element_in_struct_field_is_error() {
        // The parser cannot reject `[Spy N]` (struct fields aren't resolved
        // until the whole module is parsed), so this is the checker's job.
        let err = check_src(&format!(
            "{SPY_DEF}type: Bag xs [Spy 2] ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_direct_element_in_word_signature_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( [Spy 2] -- ) | a | a drop ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_indirect_via_linear_struct_field_is_error() {
        // `Arr`'s element (`Holds`) is not itself `Spy`, but contains one
        // transitively; `is_copy` already sees through that, so the sweep
        // over `module.arrays` must too.
        let err = check_src(&format!(
            "{SPY_DEF}type: Holds s Spy ; type: Arr a [Holds 2] ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Holds`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_indirect_via_linear_struct_in_signature_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}type: Holds s Spy ; : w ( [Holds 2] -- ) | a | a drop ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Holds`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_copy_element_is_ok() {
        check_src("type: V xs [i64 4] ; : main ( -- ) 0 . ;").unwrap();
    }

    #[test]
    fn array_of_owned_is_error() {
        let err = check_src(": w ( [^i64 4] -- ) drop ; : main ( -- ) 0 . ;").unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`^i64`"), "unexpected message: {err}");
    }

    #[test]
    fn owned_of_linear_array_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( ^[Spy 2] -- ) drop ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn nested_array_of_owned_is_error() {
        let err = check_src(": w ( ^[^i64 4] -- ) drop ; : main ( -- ) 0 . ;").unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`^i64`"), "unexpected message: {err}");
    }

    #[test]
    fn array_of_struct_holding_owned_is_error() {
        // Keeps `emit_drop`'s linear-array `unreachable!` guard valid now that
        // cells are a second linear type: an array whose element only holds a
        // cell transitively must be rejected here too, or lowering would reach
        // that arm with an array needing drop glue.
        let err = check_src("type: Holds c ^i64 ; type: Arr a [Holds 2] ; : main ( -- ) 0 . ;")
            .unwrap_err();
        assert!(err.contains("linear array elements are not supported yet"));
        assert!(err.contains("`Holds`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_and_enum_duplicate_name_across_registries_is_error() {
        // X2: a name used by one struct and one enum names that type.
        let err = check_src("type: Dup x i64 ; type: Dup | V ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Dup"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_nested_aggregate_fields_ok() {
        // D9: a variant may carry a struct, and a struct may carry an enum,
        // acyclically — no recursion error.
        check_src(
            "type: Vec2 x f64 y f64 ;
             type: Shape | Dot p Vec2 | Empty ;
             type: Tagged k Shape n i64 ;",
        )
        .unwrap();
    }

    #[test]
    fn check_struct_constructor_arity_mismatch_is_error() {
        // X4: too few values fed to the constructor, naming the struct.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 Vec2 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_constructor_field_type_mismatch_is_error() {
        // X4: a `bool` where an `i64` field is expected, naming struct+field type.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 true Vec2 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_accessor_on_wrong_type_is_error() {
        // X5: `Vec2>x` applied to a bare `i64` names the accessor and both types.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- i64 ) 5 Vec2>x ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2>x"), "unexpected message: {err}");
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_accessor_on_other_struct_is_error() {
        // X5: a `Vec2` accessor applied to a `Segment` names both struct types.
        let src = "type: Vec2 x i64 y i64 ; type: Segment from Vec2 to Vec2 ;
            : main ( -- i64 ) 1 2 Vec2 3 4 Vec2 Segment Vec2>x ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2>x"), "unexpected message: {err}");
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
        assert!(err.contains("`Segment`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_print_is_error() {
        // X6: `.` on a struct reaches `print_requires_printable`, naming it.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- ) 1 2 Vec2 . ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("printable"), "unexpected message: {err}");
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_equality_operator_is_error() {
        // X7: `=` on two structs is scalar-only, naming the struct type.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- bool ) 1 2 Vec2 1 2 Vec2 = ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_arithmetic_operator_is_error() {
        // X7: `+` on two structs is scalar-only, naming the struct type.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 2 Vec2 1 2 Vec2 + ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_unifies_through_if_else_join_ok() {
        // R10: a struct type flows through an `if`/`else` join like any Type.
        check_src(
            "type: Vec2 x i64 y i64 ;
             : pick ( bool -- Vec2 ) if 1 2 Vec2 else 3 4 Vec2 end ;",
        )
        .unwrap();
    }

    #[test]
    fn check_struct_moves_through_shuffles_ok() {
        // R10: dup/drop/swap/over move a struct value with no special case.
        check_src(
            "type: Vec2 x i64 y i64 ;
             : main ( -- Vec2 ) 1 2 Vec2 3 4 Vec2 swap drop dup drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_enum_zero_field_variant_constructor_ok() {
        check_src("type: Cmd | Halt ; : main ( -- Cmd ) Halt ;").unwrap();
    }

    #[test]
    fn check_enum_multi_field_variant_constructor_ok() {
        check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ; : main ( -- Shape ) 2.0 Circle ;",
        )
        .unwrap();
    }

    #[test]
    fn check_enum_used_in_word_effect_ok() {
        check_src("type: Shape | Circle r f64 ; : id ( Shape -- Shape ) ;").unwrap();
    }

    #[test]
    fn check_enum_single_variant_newtype_ok() {
        // M3: a single-variant enum is allowed.
        check_src("type: Id | Wrap v i64 ; : main ( -- Id ) 5 Wrap ;").unwrap();
    }

    #[test]
    fn check_enum_duplicate_type_name_across_two_enums_is_error() {
        // X2: two enum `type:` declarations sharing a name.
        let err =
            check_src("type: Shape | Circle r f64 ; type: Shape | Square s f64 ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_duplicate_type_name_against_struct_is_error() {
        // X2: a struct and an enum sharing a name, across the combined
        // struct+enum registry (D10).
        let err = check_src("type: Vec2 x i64 y i64 ; type: Vec2 | Only v i64 ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Vec2"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_constructor_arity_mismatch_is_error() {
        // X9: too few values fed to a variant constructor, naming the enum.
        let src = "type: Shape | Rect w f64 h f64 ; : main ( -- Shape ) 1.0 Rect ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Shape"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_constructor_field_type_mismatch_is_error() {
        // X9: a `bool` where an `f64` field is expected, naming both types.
        let src = "type: Shape | Circle r f64 ; : main ( -- Shape ) true Circle ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`f64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_unifies_through_if_else_join_ok() {
        // R10: an enum type flows through an `if`/`else` join like any Type.
        check_src(
            "type: Shape | Circle r f64 | Square s f64 ;
             : pick ( bool -- Shape ) if 1.0 Circle else 2.0 Square end ;",
        )
        .unwrap();
    }

    #[test]
    fn check_enum_moves_through_shuffles_ok() {
        // R10: dup/drop/swap/over move an enum value with no special case.
        check_src(
            "type: Shape | Circle r f64 | Square s f64 ;
             : main ( -- Shape ) 1.0 Circle 2.0 Square swap drop dup drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_enum_struct_and_enum_coexist_ok() {
        // D10: a distinct registry per kind; structs and enums both resolve
        // and both generate correctly-typed words in the same module.
        check_src(
            "type: Vec2 x i64 y i64 ;
             type: Shape | Circle r f64 ;
             : main ( -- Vec2 Shape ) 1 2 Vec2 3.0 Circle ;",
        )
        .unwrap();
    }

    #[test]
    fn check_clause_word_multi_and_zero_field_ok() {
        // R11: a clause per variant, each leaving the single declared output;
        // a clause-body `| w h |` binds the payload, a zero-field clause with
        // a value flowing underneath the scrutinee type-checks.
        check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;
             type: MaybeInt | None | Some v i64 ;
             : area ( Shape -- f64 ) | Circle dup * 3.14159 * | Rect | w h | w h * ;
             : unwrap-or ( i64 MaybeInt -- i64 ) | None | Some swap drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_clause_word_non_exhaustive_names_missing_variant() {
        // X4: a clause word missing a variant names the missing one.
        let err = check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;
             : area ( Shape -- f64 ) | Circle dup * ;",
        )
        .unwrap_err();
        assert!(err.contains("non-exhaustive"), "unexpected message: {err}");
        assert!(err.contains("Rect"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_clause_word_duplicate_clause_names_variant() {
        // X5: two clauses for the same variant names it.
        let err = check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;
             : area ( Shape -- f64 ) | Circle dup * | Circle dup * | Rect | w h | w h * ;",
        )
        .unwrap_err();
        assert!(
            err.contains("duplicate clause"),
            "unexpected message: {err}"
        );
        assert!(err.contains("Circle"), "unexpected message: {err}");
    }

    #[test]
    fn check_clause_word_unknown_variant_names_it_and_enum() {
        // X6: a clause naming a non-variant of the scrutinee enum.
        let err = check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;
             type: Other | Blob b i64 ;
             : area ( Shape -- f64 ) | Circle dup * | Rect | w h | w h * | Blob 0.0 ;",
        )
        .unwrap_err();
        assert!(err.contains("unknown variant"), "unexpected message: {err}");
        assert!(err.contains("Blob"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_clause_word_on_non_enum_top_input_is_error() {
        // X7: a clause body whose top input is a scalar (not an enum).
        let err = check_src(
            "type: Circle | C r f64 ;
             : bad ( i64 -- i64 ) | C 0 ;",
        )
        .unwrap_err();
        assert!(err.contains("not an enum"), "unexpected message: {err}");
        assert!(err.contains("bad"), "unexpected message: {err}");
    }

    #[test]
    fn check_clause_body_violating_declared_output_is_error() {
        // X8/M6: a clause whose body leaves a type other than the single
        // declared output effect.
        let err = check_src(
            "type: MaybeInt | None | Some v i64 ;
             : bad ( MaybeInt -- i64 ) | None true | Some ;",
        )
        .unwrap_err();
        assert!(err.contains("type mismatch"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_parameter_named_after_variant_is_error() {
        // X12 (D8 backstop): a binding name equal to a registered variant
        // name is rejected. A parameter name is the reachable case — a `|`
        // local named after a variant is instead read as a clause by D8, so
        // the parameter slot is where the collision actually surfaces.
        let err = check_src(
            "type: Shape | Circle r f64 ;
             : bad ( Circle : i64 -- i64 ) drop 0 ;",
        )
        .unwrap_err();
        assert!(err.contains("collides"), "unexpected message: {err}");
        assert!(err.contains("Circle"), "unexpected message: {err}");
    }

    #[test]
    fn check_term_word_with_entry_locals_still_ok() {
        // Regression: a plain term word with `| ... |` entry locals is
        // unaffected by the clause-body path (no enum in scope).
        check_src(": sq ( i64 -- i64 ) | n | n n * ;").unwrap();
    }

    #[test]
    fn check_enum_print_is_error() {
        // X10/M2: `.` on an enum reaches the printable guard, naming the enum.
        let err = check_src("type: Shape | Circle r f64 ; : w ( Shape -- ) . ;").unwrap_err();
        assert!(err.contains("printable"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_equality_operator_is_error() {
        // X10/M2: `=` on two enums reaches the operand-pair guard.
        let err =
            check_src("type: Shape | Circle r f64 ; : w ( Shape Shape -- bool ) = ;").unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_arithmetic_operator_is_error() {
        // X10/M2: arithmetic on an enum reaches the operand-pair guard.
        let err =
            check_src("type: Shape | Circle r f64 ; : w ( Shape Shape -- Shape ) + ;").unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    fn first_word(src: &str) -> WordDef {
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        module.words.into_iter().next().unwrap()
    }

    #[test]
    fn tail_position_final_self_call_is_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec ;");
        assert_eq!(tail_position_calls(&w.body), vec!["rec"]);
        assert!(has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_trailing_arithmetic_is_not_tail() {
        // `rec *`: the final term is `*`, so the self-call is not in tail
        // position (classic non-tail recursion).
        let w = first_word(": rec ( i64 -- i64 ) rec * ;");
        assert_eq!(tail_position_calls(&w.body), vec!["*"]);
        assert!(!has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_trailing_swap_is_not_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec swap ;");
        assert_eq!(tail_position_calls(&w.body), vec!["swap"]);
        assert!(!has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_trailing_drop_is_not_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec drop ;");
        assert_eq!(tail_position_calls(&w.body), vec!["drop"]);
        assert!(!has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_both_terminal_if_arms_are_tail() {
        // A terminal `if` hands tail position to the last term of both arms.
        let w = first_word(": rec ( i64 -- i64 ) dup 0 > if rec else rec end ;");
        assert_eq!(tail_position_calls(&w.body), vec!["rec", "rec"]);
        assert!(has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_non_terminal_if_self_call_is_not_tail() {
        // The `if` is followed by more terms, so it is non-terminal and its
        // arms are not in tail position.
        let w = first_word(": rec ( i64 -- i64 ) dup 0 > if rec else 0 end drop 5 ;");
        assert!(!has_self_tail_call(&w));
        assert!(!tail_position_calls(&w.body).contains(&"rec"));
    }

    #[test]
    fn tail_position_clause_body_final_self_call_is_tail() {
        let w = first_word("type: E | A | B ; : w ( E -- E ) | A w | B w ;");
        assert_eq!(tail_position_calls(&w.body), vec!["w", "w"]);
        assert!(has_self_tail_call(&w));
    }

    #[test]
    fn check_mutual_tail_recursion_is_error() {
        // X1: A tail-calls B, B tail-calls A -> located error naming the cycle.
        let err = check_src(": a ( i64 -- i64 ) b ; : b ( i64 -- i64 ) a ;").unwrap_err();
        assert!(
            err.contains("mutual tail recursion"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`a`"), "unexpected message: {err}");
        assert!(err.contains("`b`"), "unexpected message: {err}");
    }

    #[test]
    fn check_non_tail_mutual_recursion_is_ok() {
        // Both words call each other only in non-tail position (`x 1 +`), so no
        // tail-call edge exists and X1 must not fire (R4 no-false-positive).
        check_src(
            ": a ( i64 -- i64 ) dup 0 > if b 1 + else drop 0 end ; \
             : b ( i64 -- i64 ) dup 0 > if a 1 + else drop 0 end ;",
        )
        .unwrap();
    }

    #[test]
    fn check_self_tail_recursion_is_allowed() {
        // A self-loop (`gcd -> gcd`) is tier-1 and must not be flagged as a
        // mutual cycle.
        check_src(&std::fs::read_to_string("examples/gcd.sth").unwrap()).unwrap();
    }

    // Phase 3 Slice 1: the linear core on bare linear values.

    #[test]
    fn is_copy_every_scalar_is_copy_and_a_drop_overloaded_struct_is_not() {
        for name in ["i8", "u64", "f32", "f64", "bool", "usize"] {
            assert!(
                is_copy(Type::from_name(name).unwrap(), &[], &[], &[]),
                "{name} is Copy"
            );
        }
        // R3 (slice 8b): a struct with a user `drop` overload is linear
        // whatever its fields say -- built directly here since this test
        // exercises `is_copy`'s own signature, not a checked module.
        let structs = vec![StructDecl {
            name: "Res".to_string(),
            name_static: "Res",
            fields: vec![("tag".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: true,
            is_bundle: false,
            module: 0,
        }];
        let res = Type::Struct(StructId::from_index(0), "Res");
        assert!(!is_copy(res, &structs, &[], &[]));
    }

    #[test]
    fn is_copy_owned_cell_is_never_copy_regardless_of_payload() {
        // R4: always linear, no payload lookup, even over a Copy payload.
        let mut cells = Vec::new();
        let ty = crate::ast::intern_owned_cell_type(&mut cells, Type::I64);
        assert!(!is_copy(ty, &[], &[], &[]));
    }

    #[test]
    fn check_owned_cell_underflow_is_error_for_all_three_words() {
        // `^`, `^>`, `^|>` each underflow the same way as any other word.
        for (op, src) in [
            ("^", ": w ( -- ^i64 ) ^ ;"),
            ("^>", ": w ( -- i64 ) ^> ;"),
            ("^|>", ": w ( -- i64 ) ^|> ;"),
        ] {
            let err = check_src(src).unwrap_err();
            assert!(
                err.contains(&format!("`{op}`")),
                "{op}: unexpected message: {err}"
            );
            assert!(
                err.contains("needs 1 values"),
                "{op}: unexpected message: {err}"
            );
            assert!(err.contains("holds 0"), "{op}: unexpected message: {err}");
        }
    }

    #[test]
    fn check_unwrap_of_non_cell_is_error() {
        // `^>` on a plain `i64` names the word and the offending type.
        let err = check_src(": w ( -- i64 ) 5 ^> ;").unwrap_err();
        assert!(err.contains("`^>`"), "unexpected message: {err}");
        assert!(
            err.contains("requires an owning-cell operand"),
            "unexpected message: {err}"
        );
        assert!(err.contains("found `i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_peek_of_non_cell_is_error() {
        // `^|>` on a plain `bool` names the word and the offending type.
        let err = check_src(": w ( -- bool bool ) true ^|> ;").unwrap_err();
        assert!(err.contains("`^|>`"), "unexpected message: {err}");
        assert!(
            err.contains("requires an owning-cell operand"),
            "unexpected message: {err}"
        );
        assert!(err.contains("found `bool`"), "unexpected message: {err}");
    }

    #[test]
    fn is_copy_struct_is_linear_iff_a_field_is_transitively() {
        // R7/R8 (Phase 2): a struct with no linear field is Copy; one with a
        // linear field (direct or nested) is linear, transitively. `^i64`
        // (an owning cell, always linear regardless of payload) stands in
        // for a direct linear leaf field, since this test exercises
        // `is_copy`'s own fold directly rather than through a checked module.
        let mut owned_cells = Vec::new();
        let cell_ty = intern_owned_cell_type(&mut owned_cells, Type::I64);
        let structs = vec![
            StructDecl {
                name: "Plain".to_string(),
                name_static: "Plain",
                fields: vec![("x".to_string(), Type::I64), ("y".to_string(), Type::I64)],
                span: Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
            StructDecl {
                name: "Holds".to_string(),
                name_static: "Holds",
                fields: vec![("a".to_string(), cell_ty), ("b".to_string(), Type::I64)],
                span: Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
            StructDecl {
                name: "Wraps".to_string(),
                name_static: "Wraps",
                fields: vec![(
                    "h".to_string(),
                    Type::Struct(StructId::from_index(1), "Holds"),
                )],
                span: Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
        ];
        let plain = Type::Struct(StructId::from_index(0), "Plain");
        let holds = Type::Struct(StructId::from_index(1), "Holds");
        let wraps = Type::Struct(StructId::from_index(2), "Wraps");
        assert!(is_copy(plain, &structs, &[], &[]));
        assert!(!is_copy(holds, &structs, &[], &[]));
        assert!(!is_copy(wraps, &structs, &[], &[]));
    }

    #[test]
    fn is_copy_enum_is_linear_iff_a_variant_field_is_transitively() {
        // R7/R12 (Phase 4): an enum with no linear variant field is Copy; one
        // with a linear field (direct in one variant, or nested through a
        // struct in another) is linear, transitively. `Plain` has no linear
        // variant, `Item` carries a linear field (an owning cell) directly in
        // `Full`, `Boxed` carries one nested inside `Holds`. Built by hand
        // rather than parsed, for the same reason as the struct fold above.
        let mut owned_cells = Vec::new();
        let cell_ty = intern_owned_cell_type(&mut owned_cells, Type::I64);
        let structs = vec![StructDecl {
            name: "Holds".to_string(),
            name_static: "Holds",
            fields: vec![("a".to_string(), cell_ty), ("b".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        }];
        let variant = |name: &'static str, fields: Vec<(String, Type)>| VariantDecl {
            name: name.to_string(),
            name_static: name,
            fields,
            span: Span::default(),
        };
        let enums = vec![
            EnumDecl {
                name: "Plain".to_string(),
                name_static: "Plain",
                variants: vec![variant("A", vec![]), variant("B", vec![])],
                span: Span::default(),
                module: 0,
            },
            EnumDecl {
                name: "Item".to_string(),
                name_static: "Item",
                variants: vec![
                    variant("Empty", vec![]),
                    variant("Full", vec![("v".to_string(), cell_ty)]),
                ],
                span: Span::default(),
                module: 0,
            },
            EnumDecl {
                name: "Boxed".to_string(),
                name_static: "Boxed",
                variants: vec![
                    variant(
                        "Some",
                        vec![(
                            "h".to_string(),
                            Type::Struct(StructId::from_index(0), "Holds"),
                        )],
                    ),
                    variant("None", vec![]),
                ],
                span: Span::default(),
                module: 0,
            },
        ];
        let plain = Type::Enum(EnumId::from_index(0), "Plain");
        let item = Type::Enum(EnumId::from_index(1), "Item");
        let boxed = Type::Enum(EnumId::from_index(2), "Boxed");
        assert!(is_copy(plain, &structs, &enums, &[]));
        assert!(!is_copy(item, &structs, &enums, &[]));
        assert!(!is_copy(boxed, &structs, &enums, &[]));
    }

    #[test]
    fn check_struct_constructor_takes_a_matching_i64_field_ok() {
        check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy drop ;")).unwrap();
    }

    #[test]
    fn check_struct_constructor_on_a_float_field_is_error() {
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7.5 Spy drop ;")).unwrap_err();
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_dup_of_linear_value_is_error() {
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy dup drop drop ;")).unwrap_err();
        assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(err.contains("linear"), "unexpected message: {err}");
    }

    #[test]
    fn check_over_of_linear_value_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( -- ) 7 Spy 1 over drop drop drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("cannot `over`"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shuffles_that_only_reorder_linear_values_are_ok() {
        // `swap`/`rot` move rather than copy, so the `dup`/`over` gate must not
        // over-reach to them.
        check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy 8 Spy swap drop drop ;")).unwrap();
        check_src(&format!(
            "{SPY_DEF}: w ( -- ) 1 Spy 2 Spy 3 Spy rot drop drop drop ;"
        ))
        .unwrap();
    }

    #[test]
    fn check_print_on_linear_value_is_error() {
        // R16: `.` is a printable-scalar path, and a linear value is not one
        // (the backend's `unreachable!` guard depends on this).
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy . ;")).unwrap_err();
        assert!(err.contains("printable"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_use_after_move_of_linear_local_names_the_move_site() {
        // `SPY_DEF` is two lines, so `w`'s own line 3 (the first `s drop`)
        // lands on line 5 of the full source.
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy -- )\n  | s |\n  s drop\n  s drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(
            err.contains("moved at line 5, col 3"),
            "the diagnostic should name the move site: {err}"
        );
    }

    #[test]
    fn check_second_mention_of_a_copy_local_is_ordinary_reuse() {
        // The move-state tracks linear locals only: a Copy local stays usable.
        check_src(": w ( i64 -- i64 ) | n | n n + ;").unwrap();
    }

    #[test]
    fn check_unconsumed_linear_local_is_error() {
        let err = check_src(&format!("{SPY_DEF}: w ( Spy -- )\n  | s |\n  1 . ;")).unwrap_err();
        assert!(err.contains("never consumed"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(
            err.contains("`s`"),
            "the error should name the local: {err}"
        );
    }

    #[test]
    fn check_surplus_linear_value_is_a_linear_flavoured_error() {
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy ;")).unwrap_err();
        assert!(
            err.contains("linear value left on the stack"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_surplus_copy_value_keeps_the_arity_error() {
        // No misfire: the linear branch must not swallow the Copy surplus case.
        let err = check_src(": w ( -- ) 1 ;").unwrap_err();
        assert!(
            err.contains("body leaves 1 values"),
            "unexpected message: {err}"
        );
        assert!(!err.contains("linear"), "unexpected message: {err}");
    }

    #[test]
    fn check_linear_local_consumed_in_both_arms_is_ok() {
        // R14: `Moved` in both arms joins to `Moved`, not `MaybeMoved`, even
        // though the two move sites differ.
        check_src(&format!(
            "{SPY_DEF}: w ( Spy bool -- )\n  | s c |\n  c if s drop else s drop end ;"
        ))
        .unwrap();
    }

    #[test]
    fn check_linear_local_moved_in_one_arm_then_used_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy bool -- )\n  | s c |\n  c if s drop else 1 . end\n  s drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_linear_local_moved_in_one_arm_and_dropped_nowhere_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy bool -- )\n  | s c |\n  c if s drop else 1 . end ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("not consumed on every path"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_linear_value_across_self_tail_call_is_error() {
        // R15: the fresh Spy pushed in the recursive arm leaves `s` live
        // across the back-edge, which the loop lowering cannot dispose yet.
        // `SPY_DEF` is two lines, so `spin`'s own line 3 lands on line 5.
        let err = check_src(&format!(
            "{SPY_DEF}: spin ( Spy i64 -- i64 )\n  | s n |\n  n 0 = if s drop 0 else 9 Spy n 1 - spin end ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(err.contains("line 5"), "the error should be located: {err}");
    }

    #[test]
    fn check_linear_value_forwarded_into_the_self_tail_call_is_ok() {
        // Moved *into* the recursive call's arguments, the Spy is forwarded,
        // not stranded, so the R15 guard must not fire.
        check_src(&format!(
            "{SPY_DEF}: spin ( Spy i64 -- i64 )\n  | s n |\n  n 0 = if s drop 0 else s n 1 - spin end ;"
        ))
        .unwrap();
    }

    #[test]
    fn check_copy_self_tail_call_is_unaffected_by_the_linear_guard() {
        check_src(&std::fs::read_to_string("examples/countdown.sth").unwrap()).unwrap();
    }

    #[test]
    fn infer_line_consumes_a_carried_linear_slot_ok() {
        // The REPL path: a residual linear slot can be dropped by a later
        // line (no scope-end rule applies to a bare line). `^i64` (an owning
        // cell, always linear) stands in for a linear entry slot, since this
        // test exercises `infer_line` directly with no struct/enum registry.
        let mut owned_cells = Vec::new();
        let cell_ty = intern_owned_cell_type(&mut owned_cells, Type::I64);
        let out = infer_src("drop", &[cell_ty]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn shared_reference_is_copy_and_mutable_reference_is_neither() {
        // The soundness question here: getting either wrong silently misclassifies
        // a reference as duplicable-and-droppable, or as owing a linear drop it
        // must never receive.
        let mut refs = Vec::new();
        let shared = intern_ref_type(&mut refs, Type::I64, false);
        let mutable = intern_ref_type(&mut refs, Type::I64, true);
        assert_ne!(shared, mutable);
        assert_eq!(shared.name(), "&i64");
        assert_eq!(mutable.name(), "&!i64");

        assert!(is_copy(shared, &[], &[], &[]));
        assert!(!is_copy(mutable, &[], &[], &[]));
        // Neither is linear: a reference owns nothing, so neither enters move
        // tracking nor owes a disposal.
        assert!(!is_linear(shared, &[], &[], &[]));
        assert!(!is_linear(mutable, &[], &[], &[]));
    }

    #[test]
    fn intern_ref_type_dedups_per_referent_and_mutability() {
        let mut refs = Vec::new();
        let a = intern_ref_type(&mut refs, Type::I64, true);
        let b = intern_ref_type(&mut refs, Type::I64, true);
        let c = intern_ref_type(&mut refs, Type::Bool, true);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn provenance_interns_one_region_per_parent_and_segment() {
        // The peek route rests on this: two non-consuming projections of one
        // field of one parent must be recognised as one region, or the aliasing
        // they create is invisible.
        let mut prov = Provenance::default();
        let s = prov.fresh_region();
        let other = prov.fresh_region();
        assert_ne!(s, other);
        assert_eq!(prov.field_region(s, "a"), prov.field_region(s, "a"));
        assert_ne!(prov.field_region(s, "a"), prov.field_region(s, "b"));
        assert_ne!(prov.field_region(s, "a"), prov.field_region(other, "a"));
    }

    #[test]
    fn provenance_regions_overlap_along_the_field_chain() {
        // The alias check reads this: a field region is still an alias of
        // its parent (and transitively, its parent's parent), while two
        // fields of unrelated parents share no ancestry at all.
        let mut prov = Provenance::default();
        let s = prov.fresh_region();
        let other = prov.fresh_region();
        let a = prov.field_region(s, "a");
        let ab = prov.field_region(a, "b");
        assert!(
            prov.regions_overlap(s, s),
            "a region always overlaps itself"
        );
        assert!(prov.regions_overlap(s, a), "a field overlaps its parent");
        assert!(prov.regions_overlap(a, s), "overlap is symmetric");
        assert!(
            prov.regions_overlap(s, ab),
            "overlap reaches through a grandparent"
        );
        assert!(
            !prov.regions_overlap(other, a),
            "unrelated parents share no ancestry"
        );
    }

    #[test]
    fn provenance_bind_consumes_the_reborrow_and_keeps_the_owned_root() {
        // The asymmetry that makes `push-byte` legal while the underlying check still
        // fires: binding a projected reference into a local releases the place
        // it was reborrowed from, but not the owned local it ultimately borrows.
        let mut prov = Provenance::default();
        let span = Span { line: 1, col: 1 };
        let fresh = prov.borrow("v", true, span);
        let held = prov.bind(Some(fresh)).expect("a bound derivation");
        let reborrow = prov.reborrow("r", Some(held), true, span);
        let projected = prov.project(Some(reborrow)).expect("a projection");
        assert!(prov.deriv(projected).reborrow, "still suspends `r`");
        assert!(prov.deriv(projected).projected, "R7's note is apt here");
        assert_eq!(prov.deriv(projected).owned_root.as_deref(), Some("v"));

        let rebound = prov.bind(Some(projected)).expect("a bound derivation");
        assert!(!prov.deriv(rebound).reborrow, "`r` is suspended no longer");
        assert_eq!(
            prov.deriv(rebound).owned_root.as_deref(),
            Some("v"),
            "`v` is still borrowed by the local"
        );
    }

    #[test]
    fn provenance_suspension_key_covers_a_reborrow_with_no_owned_root() {
        // The join key: a reborrow of a reference *parameter* has no owned
        // root, so keying the join on `owned_root` alone would make two arms
        // reborrowing two different parameters look identical.
        let mut prov = Provenance::default();
        let span = Span { line: 1, col: 1 };
        let p = prov.reborrow("p", None, true, span);
        let q = prov.reborrow("q", None, true, span);
        assert_eq!(prov.deriv(p).owned_root, prov.deriv(q).owned_root);
        assert_ne!(prov.deriv(p).suspension(), prov.deriv(q).suspension());

        // A shared reborrow suspends nothing: `&T` is Copy, so two arms
        // reborrowing different shared parameters still agree.
        let p = prov.reborrow("p", None, false, span);
        let q = prov.reborrow("q", None, false, span);
        assert_eq!(prov.deriv(p).suspension(), prov.deriv(q).suspension());
    }

    #[test]
    fn contains_reference_sees_through_a_struct_field() {
        // The predicate is transitive: a struct that merely *reaches* a
        // reference is rejected wherever a bare one would be.
        let tokens = lex("type: Plain x i64 ;\n").unwrap();
        let module = parse(&tokens).unwrap();
        let mut refs = Vec::new();
        let plain = Type::Struct(StructId::from_index(0), "Plain");
        assert!(!contains_reference(
            plain,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        let mut structs = module.structs;
        structs[0].fields.push((
            "r".to_string(),
            intern_ref_type(&mut refs, Type::I64, false),
        ));
        assert!(contains_reference(
            plain,
            &structs,
            &module.enums,
            &module.arrays
        ));
    }

    #[test]
    fn quotation_effect_unifies_and_binds_variable() {
        // Criterion 2 (R6): a declared `[ 'T -- ]` unified against a concrete
        // `[ i64 -- ]` binds `'T = i64`; an arity mismatch is a located type
        // mismatch, never a silent bind. Exercises `unify_poly_input`'s
        // `PolyType::Quotation` arm directly (the concrete poly path is Phase
        // 2), so deleting the pointwise-row unify makes this fail.
        use crate::ast::quotation_type;
        let sig = PolySig {
            row_in: None,
            inputs: vec![PolyType::Quotation(vec![PolyType::Var(0)], Vec::new())],
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: Vec::new(),
            row_var_names: Vec::new(),
        };
        let structs: [StructDecl; 0] = [];
        let enums: [EnumDecl; 0] = [];
        let arrays: [ArrayDecl; 0] = [];
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        let mut subst = Subst::default();
        unify_poly_input(
            &sig,
            &sig.inputs[0],
            quotation_type(vec![Type::I64], Vec::new()),
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &mut subst,
        )
        .expect("`[ 'T -- ]` should unify against `[ i64 -- ]`");
        assert_eq!(subst.ty_of(0), Some(Type::I64), "`'T` should bind to `i64`");

        let mut subst2 = Subst::default();
        let err = unify_poly_input(
            &sig,
            &sig.inputs[0],
            quotation_type(vec![Type::I64, Type::I64], Vec::new()),
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &mut subst2,
        )
        .expect_err("an arity mismatch must be a located type mismatch");
        assert!(
            err.contains("`f`"),
            "the arity mismatch should name the word, got: {err}"
        );
        assert!(
            subst2.ty_of(0).is_none(),
            "an arity mismatch must not silently bind `'T`"
        );
    }

    #[test]
    fn quotation_parameter_is_copy_no_move_obligation() {
        // Criterion 6b: a quotation parameter is `Copy` (it registers no move
        // obligation), so a body that *binds* its quotation param and never
        // consumes it still checks -- forgetting is only an error for a linear
        // value. The body must bind and drop-on-the-floor (`| f |`, not an
        // explicit `drop`), or the `drop` discharges the obligation and the
        // test cannot detect the property it names: making quotation types
        // linear would leave a `drop`-bodied version green.
        check_src(": ignore ( i64 [ i64 -- i64 ] -- i64 ) | f | ;\n")
            .expect("an unused quotation parameter is not a linear-forgetting error");
    }

    #[test]
    fn quotation_taking_word_mints_no_symbol() {
        // U20: a monomorphic quotation-taking word is a combinator, so it is
        // inlined and mints no `IrFunc`; `is_combinator` (the single predicate
        // `check` and `ir::lower` share) recognizes it and excludes an
        // ordinary word. Deleting the `Type::Quotation` clause makes `apply`
        // stop being a combinator and mint a symbol (a link error, since its
        // body is a bare `call` over a phantom).
        let src = ": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
                   : plain ( i64 -- i64 ) 1 + ;\n";
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        let apply = module.words.iter().find(|w| w.name == "apply").unwrap();
        let plain = module.words.iter().find(|w| w.name == "plain").unwrap();
        assert!(is_combinator(apply), "`apply` is a combinator (no symbol)");
        assert!(!is_combinator(plain), "`plain` is an ordinary word");
    }
}
