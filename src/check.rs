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
    intern_array_type, intern_owned_cell_type, intern_ref_type, ArrayDecl, Clause, EnumDecl,
    EnumId, Module, OwnedCellDecl, RefDecl, Span, StackEffect, StructDecl, StructId, Term,
    TermKind, Type, VariantDecl, WordBody, WordDef, SPY_NAME,
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
    /// R21: which region this aggregate value denotes, and where this name for
    /// it was pushed.
    alias: Option<Alias>,
    /// R6: the outstanding derivation a reference-typed value holds.
    deriv: Option<DerivId>,
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
        }
    }

    /// The same value, reached through a reference derived from `deriv`: a
    /// projection's result (R3), which keeps its parent's provenance so the
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
/// Every *structural* builtin is handled directly in `check_term`
/// (`check_shuffle`/`check_operator`): the stack shuffles, the numeric-tower
/// operators, and `.` (type-directed over any printable scalar, not a fixed
/// `( i64 -- )`) all dispatch on the concrete operand type rather than a fixed
/// signature, so they are absent here. The drop-spy constructor `__spy ( i64
/// -- __spy )` (R6) is the one builtin with a fixed effect, so it is the one
/// entry.
pub fn builtin_table() -> HashMap<String, Sig> {
    HashMap::from([(
        SPY_NAME.to_string(),
        Sig {
            inputs: vec![Type::I64],
            outputs: vec![Type::Spy],
        },
    )])
}

/// R2/R7: whether `ty` is `Copy` (freely duplicated and discarded) rather than
/// linear (used exactly once, disposed by `drop`). The drop-spy is linear;
/// a struct or enum is linear iff any field/variant-payload field is
/// (transitively), so a struct-of-struct-of-spy or an enum carrying one is
/// linear too. `structs`/`enums` resolve a `Type::Struct`/`Type::Enum`'s
/// fields; neither can recurse into itself (`check_recursion` rejects that
/// first), so this always terminates.
pub fn is_copy(ty: Type, structs: &[StructDecl], enums: &[EnumDecl], arrays: &[ArrayDecl]) -> bool {
    match ty {
        Type::Spy => false,
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
        // is not (R5's exclusivity, which `dup`'s Copy gate already enforces).
        Type::Ref(_, mutable, _) => !mutable,
        _ => true,
    }
}

/// Whether `ty` carries an exactly-once obligation: used exactly once,
/// disposed by `drop`, tracked by `Moves`. This is *not* the negation of
/// `is_copy`: `&!T` is neither `Copy` nor linear (R5/R8), so it is duplicated
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

/// R8: whether `ty` **transitively contains** a reference — is one itself, or
/// reaches one through a struct field, an enum variant payload, or an array
/// element. The predicate every escape rejection is stated over, so a
/// reference cannot slip into storage one level down from a declaration site.
///
/// A `^T` payload is deliberately *not* followed: a cell may close a type
/// cycle (`^List` inside `List`), so following one would not terminate.
/// `check_no_stored_references` sweeps the interned cell registry directly
/// instead, which reaches every payload shape a program can name without
/// recursing.
pub fn contains_reference(
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

/// R21: which region of memory an aggregate value denotes. Two slots carrying
/// the same id are two names for one address, which is what makes a mutation
/// through one silently observable through the other. `None` means "denotes a
/// region nothing else names": every value is born that way, and an aggregate
/// is given an id lazily, the first time something could alias it (a binding,
/// or a non-consuming projection out of it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegionId(u32);

/// R21: one live name for a region, and where that name was pushed. The span is
/// what lets the alias check report a *stack-resident* alias, which has no name
/// of its own to cite: an aggregate spends most of its life on the virtual
/// stack in this language, so the ability to locate one there is the difference
/// between R21 catching the hazard and only catching the spelling of it where
/// both ends happen to be bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Alias {
    region: RegionId,
    span: Span,
}

/// R6: one outstanding derivation from a place, interned in `Provenance` so a
/// `Slot` can carry it by id and stay `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DerivId(u32);

/// R6: what one live reference traces back to. Created by a fresh borrow
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
    /// what R3's suspend rule is keyed on. Cleared when the derivation is bound
    /// into a local: the binding consumes the reborrow, which is why
    /// `push-byte` may name `b` three times.
    reborrow: bool,
    mutable: bool,
    /// Whether any projection step stands between the place and this
    /// reference: R7's path-disjointness note is only apt when one does.
    projected: bool,
    /// Where the borrow was taken, so a conflict can name both ends.
    span: Span,
}

impl Deriv {
    /// R10: the places this derivation keeps suspended, which is what a branch
    /// join has to agree on. Both halves are consulted by a hazard check
    /// (`owned_root` by the consume/borrow-conflict scans, the reborrowed
    /// reference local by R3's suspend rule), so both belong in the key: a join
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
/// to (R6) and which region each aggregate value denotes (R21). Threaded `&mut`
/// through the walk rather than kept in `Scope`, which an `if` arm clones: ids
/// stay unique across the arms, and a record outlives the arm that made it.
#[derive(Debug, Default)]
struct Provenance {
    derivs: Vec<Deriv>,
    regions: u32,
    /// The interned region of one non-consuming projection out of a parent
    /// region, so two peeks of the same field yield one id.
    fields: HashMap<(u32, String), RegionId>,
    /// Each field region's immediate parent (R7/R21): a name for a struct's
    /// field is still a name for part of the whole struct, so the alias check
    /// has to test region *overlap* along this chain, not bare equality.
    parents: HashMap<u32, RegionId>,
}

impl Provenance {
    fn fresh_region(&mut self) -> RegionId {
        let id = RegionId(self.regions);
        self.regions += 1;
        id
    }

    /// R21: the region an interior value of `parent` denotes, interned per path
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

    /// R21: whether `a` and `b` denote overlapping storage — the same region,
    /// or one an ancestor of the other along the field-projection chain.
    /// Mirrors R7's conservative field-borrow rule on the naming side: a name
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

    /// R2: a fresh borrow of an owned aggregate place.
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

    /// R5: naming a reference local reborrows it — a new chain rooted at that
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

    /// R3: one projection step — the same place, one step further from it.
    fn project(&mut self, parent: Option<DerivId>) -> Option<DerivId> {
        let deriv = Deriv {
            projected: true,
            ..self.deriv(parent?).clone()
        };
        Some(self.add(deriv))
    }

    /// R3: binding a reference into a local consumes the reborrow it came from,
    /// so the place it was reborrowed from is suspended no longer. The owned
    /// root survives: the local still keeps its referent borrowed (R6).
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
/// region an aggregate binding denotes (R21) and which derivation a reference
/// binding holds (R6).
#[derive(Debug, Clone)]
struct Binding {
    name: String,
    ty: Type,
    region: Option<RegionId>,
    deriv: Option<DerivId>,
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
    /// so forgetting it is caught at the end of its block (R6). An aggregate
    /// with no region of its own gets one here: a binding is the first point at
    /// which a second name could denote the same address (R21).
    fn bind(&mut self, name: &str, slot: Slot, linear: bool, prov: &mut Provenance) {
        if linear {
            self.moves.states.insert(name.to_string(), MoveState::Live);
        }
        let region = match (slot.alias, slot.ty.is_aggregate()) {
            (Some(alias), _) => Some(alias.region),
            (None, true) => Some(prov.fresh_region()),
            (None, false) => None,
        };
        self.bound.push(Binding {
            name: name.to_string(),
            ty: slot.ty,
            region,
            deriv: prov.bind(slot.deriv),
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

/// R6: every derivation still live — held by a slot on the virtual stack, or by
/// a reference-typed local still in scope. A reference is live from the term
/// that creates it until the term that consumes its slot; a reference *local*
/// is live for the whole block (R8).
fn live_derivs<'a>(stack: &'a [Slot], scope: &'a Scope) -> impl Iterator<Item = DerivId> + 'a {
    stack
        .iter()
        .filter_map(|slot| slot.deriv)
        .chain(scope.bound.iter().filter_map(|b| b.deriv))
}

/// R6: the first live derivation satisfying `pred`. The scan is over
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

/// R6: a live borrow rooted at the owned place `place`, whatever its
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

/// R21's naming side: a live *mutable* borrow rooted at `place`, which any new
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

/// R21: the region a non-consuming projection out of `parent` denotes, for an
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
    let base = parent
        .alias
        .get_or_insert_with(|| Alias {
            region: prov.fresh_region(),
            span,
        })
        .region;
    Some(Alias {
        region: prov.field_region(base, segment),
        span,
    })
}

/// R21: where a second live name for a region is, when the diagnostic has to
/// point at it. A bound local reports its name, which is what the user has to
/// change; a value still on the virtual stack has no name, so it reports the
/// site that pushed it instead.
enum AliasOrigin<'a> {
    Name(&'a str),
    Stack(Span),
}

/// R21: another live name denoting a region overlapping the local `place`'s —
/// the same region, or one nested inside the other's field chain (R7: a name for
/// a field is still a name for part of the whole place). The scan covers the
/// virtual stack as well as the locals map, exactly as R6's does: a
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
    let region = scope.local(place)?.region?;
    let overlaps = |r: RegionId| prov.regions_overlap(region, r);
    let mut names: Vec<&str> = scope
        .bound
        .iter()
        .filter(|b| {
            b.name != place
                && b.region.is_some_and(&overlaps)
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
        .find(|alias| overlaps(alias.region))
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
        name: &'a str,
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
        name: &word.name,
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
}

/// Takes `&mut Module` because an array word (`fill`) interns its result
/// shape `[T N]` into `module.arrays` during checking (R3, R10): the same
/// registry `ir::lower` then reads, so the checker and the layout builder
/// share one `ArrayId` numbering. `check` runs before `lower`, so the
/// interned shapes are present when codegen consults them.
pub fn check(module: &mut Module) -> Result<(), String> {
    check_types(
        &module.structs,
        &module.enums,
        &module.arrays,
        &module.owned_cells,
    )?;

    let mut env = builtin_table();
    for (name, sig) in struct_generated_sigs(&module.structs) {
        env.insert(name, sig);
    }
    for (name, sig) in enum_generated_sigs(&module.enums) {
        env.insert(name, sig);
    }
    for word in &module.words {
        env.insert(word.name.clone(), sig_of(&word.effect));
    }

    // Reject mutual tail-recursion cycles (D3, X1) on the whole-module
    // tail-call graph, after signature registration and before body checking.
    check_tail_call_cycles(&module.words)?;

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
    } = module;
    for word in words.iter() {
        check_word(word, enums, &env, arrays, owned_cells, refs, structs)?;
    }
    Ok(())
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

/// R8's declaration-site half: a struct field, an enum variant payload field,
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

/// R8: the one wording every escape rejection shares. `position` names the
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
/// direct `[__spy N]` and an indirect `[LinearStruct N]` alike. Runs after
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

/// A duplicate `type:` name is a sharp located error naming the type.
fn check_duplicate_struct_names(structs: &[StructDecl]) -> Result<(), String> {
    let mut seen: HashMap<&str, ()> = HashMap::new();
    for decl in structs {
        if seen.insert(decl.name.as_str(), ()).is_some() {
            return Err(format!(
                "error: duplicate type `{}` (line {}, col {})",
                decl.name, decl.span.line, decl.span.col
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
    let mut seen: HashMap<&str, ()> = structs
        .iter()
        .map(|decl| (decl.name.as_str(), ()))
        .collect();
    for decl in enums {
        if seen.insert(decl.name.as_str(), ()).is_some() {
            return Err(format!(
                "error: duplicate type `{}` (line {}, col {})",
                decl.name, decl.span.line, decl.span.col
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
        // by-value cycle — and R8 keeps one out of every field position
        // anyway.
        Type::Ref(..) => None,
        Type::Int(_) | Type::Float(_) | Type::Bool | Type::Usize | Type::Isize | Type::Spy => None,
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
pub fn check_def(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
) -> Result<(), String> {
    let mut env = env.clone();
    env.insert(word.name.clone(), sig_of(&word.effect));
    check_word(word, enums, &env, arrays, cells, refs, structs)
}

/// Infer the net effect of a bare line: simulate the typed stack from
/// `entry_stack` (the carried slot types) and return the resulting typed stack.
/// A type mismatch or underflow against the carried stack is a reported error.
#[allow(clippy::too_many_arguments)]
pub fn infer_line(
    terms: &[Term],
    entry_stack: &[Type],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
) -> Result<Vec<Type>, String> {
    let initial: Vec<Slot> = entry_stack.iter().map(|ty| Slot::computed(*ty)).collect();
    // A line is one block: names it binds die with it, so its end is a scope
    // end like any other. It is not a word body, so nothing in it is in tail
    // position.
    let ctx = Ctx::Line { structs, enums };
    let mut scope = Scope::default();
    let mut prov = Provenance::default();
    let final_stack = check_terms(
        terms, initial, &ctx, env, arrays, cells, refs, &mut prov, &mut scope, false,
    )?;
    let line = terms.last().map(|t| t.span.line).unwrap_or(0);
    leave_block(&ctx, &mut scope, 0, BlockEnd::Body(line))?;
    // R8's sixth position: the session's inter-line stack outlives this line's
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
    Ok(final_stack.into_iter().map(|s| s.ty).collect())
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
            word.name, line, final_stack.len(), declared.len(), effect_str(&word.effect),
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
pub fn has_self_tail_call(word: &WordDef) -> bool {
    tail_position_calls(&word.body)
        .iter()
        .any(|&callee| callee == word.name)
}

/// A word's location, derived from the first term (or clause) of its body,
/// for locating a whole-word diagnostic like X1.
fn word_span(word: &WordDef) -> Span {
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
fn check_tail_call_cycles(words: &[WordDef]) -> Result<(), String> {
    let name_to_idx: HashMap<&str, usize> = words
        .iter()
        .enumerate()
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
    let mut chain: Vec<&str> = cycle.iter().map(|&i| words[i].name.as_str()).collect();
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

#[allow(clippy::too_many_arguments)]
fn check_word(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
) -> Result<(), String> {
    // A parameter name equal to a registered variant name is rejected (X12)
    // regardless of body form.
    let ctx = word_ctx(word, structs, enums);
    for slot in &word.effect.inputs {
        if let Some(name) = &slot.name {
            reject_variant_local(&ctx, name, "parameter")?;
        }
    }
    check_reference_free_signature(word, structs, enums, arrays)?;
    match &word.body {
        WordBody::Terms { terms } => {
            check_terms_word(word, enums, terms, env, arrays, cells, refs, structs)
        }
        WordBody::Clauses(clauses) => {
            check_clause_word(word, enums, clauses, env, arrays, cells, refs, structs)
        }
    }
}

/// R8's effect-signature half: no declared **output** may transitively
/// contain a reference (returning one would outlive the frame that owns the
/// referent), and an **input** may only be a reference at the top level — a
/// type that merely *contains* one nested inside an array or a cell is
/// rejected there too, so the carve-out stays closed if a future aggregate
/// constructor arrives.
fn check_reference_free_signature(
    word: &WordDef,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    for slot in &word.effect.outputs {
        if contains_reference(slot.ty, structs, enums, arrays) {
            return Err(format!(
                "error: a reference cannot be stored: `{}` declares the output `{}`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead",
                word.name, slot.ty
            ));
        }
    }
    for slot in &word.effect.inputs {
        if !slot.ty.is_ref() && contains_reference(slot.ty, structs, enums, arrays) {
            return Err(format!(
                "error: a reference cannot be stored: `{}` declares the input `{}`, which contains a reference\n  an input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate",
                word.name, slot.ty
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
                word.name,
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
    let final_stack = check_terms(
        terms, initial, &ctx, env, arrays, cells, refs, &mut prov, &mut scope, true,
    )?;

    let declared: Vec<Type> = word.effect.outputs.iter().map(|s| s.ty).collect();
    let line = terms.last().map(|t| t.span.line).unwrap_or(0);
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
) -> Result<(), String> {
    // R16: the top input may be a plain enum (value mode) or a reference to
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
                    word.name,
                    effect_str(&word.effect),
                ));
            }
        },
        _ => {
            return Err(format!(
                "error: clause-style body on `{}` whose top input is not an enum\n  note: declared {}",
                word.name,
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
                word.name,
                clause.span.line,
                clause_variant_ambiguity_note(&clause.variant),
            ));
        };
        if seen.insert(clause.variant.as_str(), ()).is_some() {
            return Err(format!(
                "error: duplicate clause for variant `{}` of enum `{}` in `{}` (line {}){}",
                clause.variant,
                enum_name,
                word.name,
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
) -> Result<(), String> {
    let ctx = word_ctx(word, structs, enums);
    let mut seen_locals = HashSet::new();
    for name in &clause.locals {
        reject_variant_local(&ctx, name, "local")?;
        reject_duplicate_local(&ctx, name, clause.span, &mut seen_locals)?;
    }

    // The clause consumes the scrutinee and pushes the variant's fields
    // (first field deepest) atop any inputs below it. R16: in reference mode
    // every field arrives as a reference inheriting the scrutinee's
    // mutability, projecting through it exactly as a struct-field projection
    // would (R3) — the payload is never owned, so it is never moved or freed.
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
    )?;
    let line = clause
        .body
        .last()
        .map(|t| t.span.line)
        .unwrap_or(clause.span.line);
    check_outputs(word, &final_stack, declared, line, structs, enums, arrays)?;
    leave_block(&ctx, &mut scope, 0, BlockEnd::Body(line))
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
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `{}` needs {} values, but the stack holds {}\n  note: declared {}",
            name, span.line, op, needs, holds, effect_str(effect),
        ),
        Ctx::Line { .. } => format!("error: stack underflow: needs {needs} values, but the stack holds {holds}"),
    }
}

fn type_mismatch_error(ctx: &Ctx, span: Span, op: &str, expected: Type, found: Type) -> String {
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

/// `and`/`or`/`xor` applied to a non-integer/non-bool or mixed-type pair:
/// bitwise ops are homogeneous over the integer types and `bool`, same shape
/// as `mod_requires_int_error`.
fn bitwise_pair_mismatch_error(ctx: &Ctx, span: Span, op: &str, a: Type, b: Type) -> String {
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

/// R4 (D3): `dup`/`over` applied to a non-`Copy` value, in the DESIGN.md form.
/// A linear value has no bits to copy: the only ways to get a second one are to
/// thread this one through or to acquire another explicitly.
fn cannot_copy_linear_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: cannot `{}` a value of type `{}` in `{}` (line {})\n  `{}` is linear: it owns a resource and has no `Copy` instance, so there are no bits to copy; thread the value through instead\n  note: declared {}",
            op, found, name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: cannot `{op}` a value of type `{found}`: `{found}` is linear and has no `Copy` instance"
        ),
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
        word.name,
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

/// R9: a reference argument to a self-tail-call whose provenance traces to an
/// owned local of *this* frame — a `place` naming an actual
/// `Deriv::owned_root` — crosses a loop iteration boundary. Locals rebind at
/// the loop header (`header_phis`, src/ir.rs:1491), so the storage that local
/// named this iteration is not the storage the same name denotes next
/// iteration, and a reference into it would alias a reused slot. A reference
/// *parameter*, or one derived from it by projection, has no owned root
/// (`owned_root` is `None`, R9's accept-case) and may cross freely — its
/// referent lives in an ancestor frame that outlives every iteration, which is
/// what keeps `walk ( &!List -- ) ... walk ;` legal.
fn reference_across_back_edge_error(ctx: &Ctx, span: Span, callee: &str, place: &str) -> String {
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

/// R9: reject a reference argument to the recursive call whose derivation's
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

/// R10: the borrow-suspension bookkeeping must agree at a branch join, real
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
                let (ty, region, held) = (binding.ty, binding.region, binding.deriv);
                match ref_parts(ty, refs) {
                    // R5: naming a reference local is a reborrow, not a move.
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
                        // R6: consuming a place while a reference derived from
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
                        // R3 (D2): mentioning a linear local moves its value
                        // out; a second mention names the site that already
                        // consumed it.
                        if let Err(site) = scope.moves.take(name, span) {
                            return Err(use_after_move_error(ctx, span, name, ty, site));
                        }
                        // R21, the direction symmetric with the check at the
                        // borrow: this naming would be the *second* name for
                        // storage a live `&!` already reaches, so the mutation
                        // is just as silently observable as if the naming had
                        // come first. Only an aggregate has a region, and so
                        // only an aggregate can be a second name for one.
                        if region.is_some() {
                            if let Some(id) = live_mutable_borrow_of(&stack, scope, prov, name) {
                                return Err(naming_aliases_borrowed_place_error(
                                    ctx,
                                    span,
                                    name,
                                    prov.deriv(id),
                                ));
                            }
                        }
                        // R21: naming an aggregate does not copy it, so the
                        // pushed value denotes the local's own region, located
                        // here so a later borrow can point at this naming.
                        stack.push(Slot {
                            alias: region.map(|region| Alias { region, span }),
                            ..Slot::computed(ty)
                        });
                    }
                }
                return Ok(stack);
            }
            if let Some(stack) = check_reference_word(
                name, span, &mut stack, ctx, scope, arrays, cells, refs, prov,
            )? {
                return Ok(stack);
            }
            if let Some(stack) = check_access_word(name, span, &mut stack, ctx, arrays, refs)? {
                return Ok(stack);
            }
            if let Some(stack) = check_shuffle(name, span, &mut stack, ctx, arrays)? {
                return Ok(stack);
            }
            if let Some(stack) = check_operator(name, span, &mut stack, ctx)? {
                return Ok(stack);
            }
            if let Some(stack) = check_array_word(name, span, &mut stack, ctx, arrays, prov)? {
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
                match match_slot(found, *want) {
                    SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
                    SlotMatch::NeedsSizeConversion => {
                        return Err(size_conversion_needed_error(ctx, span, name, *want));
                    }
                    SlotMatch::Mismatch => {
                        return Err(type_mismatch_error(ctx, span, name, *want, found.ty));
                    }
                }
            }
            if tail && ctx.word_name() == Some(name.as_str()) {
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
            for (t_then, t_else) in then_stack.iter().zip(&else_stack) {
                if t_then.ty != t_else.ty {
                    return Err(branch_type_mismatch_error(ctx, span, t_then.ty, t_else.ty));
                }
                // R10: the type-only join above already rejects two arms whose
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
                merged.push(Slot {
                    ty: t_then.ty,
                    literal: t_then.literal && t_else.literal,
                    // A value merged from two branches is never a single
                    // known literal, so it can't feed a compile-time count.
                    int_val: None,
                    // R21: an arm can be a bare `Call` of a local that never
                    // gets rebound (`if v else v end`), which leaves both arms
                    // denoting the *same* region — collapsing that to `None`
                    // regardless of agreement would let a name bound to the
                    // merge alias its source silently.
                    alias: match (t_then.alias, t_else.alias) {
                        (Some(a), Some(b)) if a.region == b.region => Some(a),
                        _ => None,
                    },
                    deriv,
                });
            }
            Ok(merged)
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
        "." => {
            let n = stack.len();
            if n < 1 {
                return Err(need(".", 1, n));
            }
            let a = stack[n - 1];
            if !a.ty.is_numeric() && !a.ty.is_bool() {
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

/// An array word (`fill`/`get`/`set`/`len`) applied to a non-array operand:
/// names the array word and the offending operand type (X8).
fn array_word_operand_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
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
        SlotMatch::Mismatch => Err(type_mismatch_error(ctx, span, op, Type::Usize, index.ty)),
    }
}

/// The referent of a reference type, and whether it is mutable.
fn ref_parts(ty: Type, refs: &[RefDecl]) -> Option<(Type, bool)> {
    match ty {
        Type::Ref(id, mutable, _) => Some((refs[id.index()].referent, mutable)),
        _ => None,
    }
}

/// R2: `&x`/`&!x` applied to something that is not a local. A place is a
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

/// R11: only an aggregate or cell local may be borrowed. A scalar local is an
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

/// R2: `&x`/`&!x` applied to a local that is *already* a reference. A borrow
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

/// R4: `!`/`+!` through a shared reference. Storing through a `&T` is
/// meaningless, and the mutable spelling is right there.
fn store_through_shared_reference_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    format!(
        "error: `{op}` cannot store through the shared reference `{found}`{} (line {})\n  borrow it mutably with `&!` (and project with the `&!`-spelled accessors) to write through it",
        in_word(ctx),
        span.line
    )
}

/// R4: `@`/`!`/`+!` are restricted to a `Copy` referent. Fetching a linear
/// value through a reference would manufacture a second owner; storing over
/// one would silently leak the value being overwritten (nothing auto-drops).
fn access_of_linear_referent_error(ctx: &Ctx, span: Span, op: &str, referent: Type) -> String {
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

/// R5: exclusivity, in whichever of its two symmetric directions was
/// violated — a new mutable borrow conflicts with any live borrow of the place,
/// a new shared one with a live mutable borrow. R7: when the live borrow is a
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

/// R3/R5: naming a `&!` local reborrows it, and a reborrow may not be taken
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

/// R6: consuming a place — moving it into a word, or disposing of it — while a
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

/// R21: a mutable borrow of a place a second live name denotes. Naming an
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

/// R21, the symmetric direction: naming an aggregate while a mutable borrow of
/// its storage is live. R5 warns that the converse of an exclusivity rule is
/// easy to omit, and this is that omission for R21: checking only at the borrow
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

/// R8's two construction sites: `fill`'s element and `^`'s payload accept
/// whatever type is on the stack, with no declaration anywhere for
/// `check_no_stored_references` to have caught.
fn constructed_reference_error(ctx: &Ctx, span: Span, position: &str, ty: Type) -> String {
    format!(
        "error: a reference cannot be stored{} (line {})\n  {position} has type `{ty}`\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow",
        in_word(ctx),
        span.line
    )
}

/// R2/R3: every `&`-led word — the two prefix borrow operators and the
/// reference-mode accessor family. Returns `None` if `name` is not `&`-led
/// (the caller falls through to the ordinary lookup chain).
///
/// One spelling per shape *and* per mutability (R3): the mutability is in the
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
            // R2: everything else is a prefix borrow of a local, and only of a
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
            if local_ty.is_ref() {
                return Err(borrow_of_reference_local_error(ctx, span, rest, local_ty));
            }
            if !matches!(
                local_ty,
                Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)
            ) {
                return Err(borrow_of_scalar_local_error(ctx, span, rest, local_ty));
            }
            // R2/R8: borrowing is not a move, but the referent still has to be
            // there. A local consumed earlier holds nothing, and borrowing it
            // would read (and project through) storage its owner has already
            // freed.
            if let Some(site) = scope.moves.moved_site(rest) {
                return Err(use_after_move_error(ctx, span, rest, local_ty, site));
            }
            // R5: exclusivity, symmetric. A new mutable borrow conflicts with
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
            // R21: a second live name for one region makes a mutation through
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

/// R4: `@` fetches, `!` stores, `+!` adds in place. All three are restricted
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

/// Apply an array word (`fill`/`get`/`set`/`len`) if `name` is one, returning
/// `Some(stack)`; `None` if the name is not an array word (the caller then
/// looks it up in the env). These are generic over the array shape, so
/// (like the shuffles and numeric operators) they dispatch on the concrete
/// operand types rather than a fixed env signature (R6, R10):
///
/// - `fill ( T -- [T N] )`: the top slot is the compile-time count `N` (a
///   literal, M1), the slot below is the element `T`; interns the `(T, N)`
///   shape (R3) and pushes it.
/// - `get ( [T N] usize -- T )`: **non-consuming** (R12/M4) — the array stays
///   on the stack; a constant index is bounds-checked (X4).
/// - `set ( [T N] usize T -- [T N] )`: a functional write; the value must
///   match the element type.
/// - `len ( [T N] -- usize )`: **non-consuming**, folds to the constant `N`.
fn check_array_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
    prov: &mut Provenance,
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
            let Some(count_val) = count.int_val else {
                return Err(fill_count_not_literal_error(ctx, span, count.ty));
            };
            if !(1..=i64::from(u32::MAX)).contains(&count_val) {
                return Err(fill_count_out_of_range_error(ctx, span, count_val));
            }
            // R8's third position, at the construction site: `fill` accepts
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
            if !matches!(stack[n - 1].ty, Type::Array(..)) {
                return Err(array_word_operand_error(ctx, span, "len", stack[n - 1].ty));
            }
            // Non-consuming: the array stays; `len` folds to the constant `N`.
            stack.push(Slot::computed(Type::Usize));
        }
        "get" => {
            let n = stack.len();
            if n < 2 {
                return Err(need("get", 2, n));
            }
            let index = stack[n - 1];
            let Type::Array(id, _) = stack[n - 2].ty else {
                return Err(array_word_operand_error(ctx, span, "get", stack[n - 2].ty));
            };
            let count = arrays[id.index()].count;
            let elem = arrays[id.index()].element;
            check_array_index(index, count, ctx, span, "get")?;
            // Non-consuming (R12): drop the index, leave the array, push T.
            // R21: for an aggregate element the pushed value *is* the element,
            // so two `get`s of one array denote one region. Which element is
            // not modelled (R7 does not model path disjointness either), so
            // every element of one array shares one region here.
            let alias = peek_region(&mut stack[n - 2], elem, "[]", span, prov);
            stack.truncate(n - 1);
            stack.push(Slot {
                alias,
                ..Slot::computed(elem)
            });
        }
        "set" => {
            let n = stack.len();
            if n < 3 {
                return Err(need("set", 3, n));
            }
            let value = stack[n - 1];
            let index = stack[n - 2];
            let Type::Array(id, _) = stack[n - 3].ty else {
                return Err(array_word_operand_error(ctx, span, "set", stack[n - 3].ty));
            };
            let array_ty = stack[n - 3].ty;
            let count = arrays[id.index()].count;
            let elem = arrays[id.index()].element;
            check_array_index(index, count, ctx, span, "set")?;
            match match_slot(value, elem) {
                SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
                SlotMatch::NeedsSizeConversion => {
                    return Err(size_conversion_needed_error(ctx, span, "set", elem));
                }
                SlotMatch::Mismatch => {
                    return Err(type_mismatch_error(ctx, span, "set", elem, value.ty));
                }
            }
            stack.truncate(n - 3);
            stack.push(Slot::computed(array_ty));
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
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "^" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("^", 1, n));
            }
            let payload = stack[n - 1].ty;
            // R8's fourth position, at the construction site: `^` interns a
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
/// per-struct-per-field name (unlike `fill`/`get`/`set`, it is not generic
/// over a shape, so it is not a fixed entry in `struct_generated_sigs`
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
    if top.ty != struct_ty {
        return Err(type_mismatch_error(ctx, span, name, struct_ty, top.ty));
    }
    // R21: the peek is non-consuming and pushes the field's *interior address*,
    // so two peeks of one field of one struct are two names for one region.
    let alias = peek_region(&mut stack[n - 1], field_ty, field_name, span, prov);
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
) -> Result<Option<Vec<Slot>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "dup" => {
            let top = *stack.last().ok_or_else(|| need("dup", 1, stack.len()))?;
            // R4 (D3): `dup` is the explicit copy, so it is gated on `Copy`.
            // The pure reorderings below (`swap`/`rot`) move rather than copy
            // and stay legal on a linear value.
            if !is_copy(top.ty, ctx.structs(), ctx.enums(), arrays) {
                return Err(cannot_copy_linear_error(ctx, span, "dup", top.ty));
            }
            // R21: `dup` of an aggregate deep-copies it (`Alloc`+`Blit`), so the
            // copy denotes a region of its own — this is the whole remedy for an
            // aliased place. `over` below reuses the value instead, and so
            // deliberately keeps the region it copies.
            stack.push(Slot { alias: None, ..top });
        }
        "drop" => {
            if stack.is_empty() {
                return Err(need("drop", 1, 0));
            }
            stack.pop();
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
            let below = stack[n - 2];
            // R4: `over` copies the second slot, so it is gated exactly like
            // `dup`.
            if !is_copy(below.ty, ctx.structs(), ctx.enums(), arrays) {
                return Err(cannot_copy_linear_error(ctx, span, "over", below.ty));
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
        // block-end firing site reports.
        scope.bind("s", Slot::computed(Type::Spy), true, prov);
        let leaked = scope.leave(depth).expect("an unconsumed linear local");
        assert_eq!((leaked.0.as_str(), leaked.1), ("s", Type::Spy));
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
        let err = check_src(": main ( -- __spy ) 7 __spy ;").unwrap_err();
        assert!(
            err.contains("cannot declare a linear type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_main_linear_input_is_error() {
        let err = check_src(": main ( __spy -- ) | s | s drop ;").unwrap_err();
        assert!(
            err.contains("cannot declare a linear type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
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

    // Array words (R10-R14): fill / get / set / len type-checking.

    #[test]
    fn check_fill_get_set_len_happy_path_ok() {
        // `fill` builds `[i64 4]`; `get`/`len` are non-consuming (the array
        // stays), `set` yields a fresh array; one `drop` clears the residual.
        check_src(": w ( -- ) 7 4 fill 0 get drop len drop 0 9 set drop ;").unwrap();
    }

    #[test]
    fn check_fill_output_type_is_the_array_shape() {
        // `fill` interns `[i64 4]` and the declared output must match it, so
        // this word type-checks with an array-typed output slot (R2/R3/R10).
        check_src(": w ( -- [i64 4] ) 7 4 fill ;").unwrap();
    }

    #[test]
    fn check_get_is_non_consuming_leaves_array_ok() {
        // R12/M4: `get` leaves the array live, so a word returning both the
        // array and the read element type-checks without a `dup`.
        check_src(": w ( [i64 4] usize -- [i64 4] i64 ) | a i | a i get ;").unwrap();
    }

    #[test]
    fn check_len_is_non_consuming_leaves_array_ok() {
        check_src(": w ( [i64 4] -- [i64 4] usize ) | a | a len ;").unwrap();
    }

    #[test]
    fn check_get_runtime_usize_index_ok() {
        // A computed `usize` index is admissible (the runtime path; its bounds
        // trap lands in Phase 4).
        check_src(": w ( [i64 4] -- [i64 4] i64 ) | a | a 1 >usize get ;").unwrap();
    }

    #[test]
    fn check_constant_index_out_of_range_is_error() {
        // X4/R11: a literal index >= N is a sharp located compile error naming
        // the length and the index.
        let err = check_src(": w ( -- ) 0 4 fill 9 get drop drop ;").unwrap_err();
        assert!(err.contains("out of range"), "unexpected message: {err}");
        assert!(err.contains("9"), "should name the index: {err}");
        assert!(err.contains("4"), "should name the length: {err}");
    }

    #[test]
    fn check_computed_index_without_conversion_is_error() {
        // X10: a computed (non-literal) `i64` index needs an explicit `>usize`.
        let err = check_src(": w ( i64 -- ) | n | 0 4 fill n get drop drop ;").unwrap_err();
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
        let err = check_src(": w ( -- ) 0 __spy 3 fill drop ;").unwrap_err();
        assert!(
            err.contains("not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_fill_of_linear_struct_element_is_error() {
        // The same rejection applies transitively: a struct that is linear
        // because one of its fields is (R7) is just as unsupported as a bare
        // `__spy` element.
        let err = check_src("type: Holder xs __spy ;\n: w ( -- ) 0 __spy Holder 3 fill drop ;")
            .unwrap_err();
        assert!(
            err.contains("not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Holder`"), "unexpected message: {err}");
    }

    #[test]
    fn check_get_on_non_array_is_error() {
        // X8: `get` on a non-array operand names the array word and the type.
        let err = check_src(": w ( -- i64 ) 5 1 get ;").unwrap_err();
        assert!(err.contains("`get`"), "unexpected message: {err}");
        assert!(err.contains("array"), "unexpected message: {err}");
    }

    #[test]
    fn check_set_wrong_element_type_is_error() {
        // X8: `set` with a value not matching the element type errors, naming
        // both the expected element type and the offending found type.
        let err = check_src(": w ( -- ) 0 4 fill 0 true set drop ;").unwrap_err();
        assert!(err.contains("type mismatch"), "unexpected message: {err}");
        assert!(
            err.contains("expected `i64`"),
            "should name the element type: {err}"
        );
        assert!(
            err.contains("found `bool`"),
            "should name the offending type: {err}"
        );
    }

    #[test]
    fn check_get_wrong_arity_is_error() {
        // X8: too few operands to `get` is a located underflow error naming
        // the array word.
        let err = check_src(": w ( -- i64 ) 5 get ;").unwrap_err();
        assert!(err.contains("`get`"), "should name the word: {err}");
        assert!(
            err.contains("needs 2 values, but the stack holds 1"),
            "should name the arity mismatch: {err}"
        );
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
            ": mk ( -- [i64 4] ) 0 4 fill ;\n: use ( [i64 4] -- i64 ) 0 get swap drop ;\n: w ( -- i64 ) mk use ;",
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
        )
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
        let err = check_src(
            "type: Holds a __spy b i64 ; : main ( -- ) 7 __spy 1 Holds Holds|>a drop drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("cannot `Holds|>a`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
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
        // The parser cannot reject `[__spy N]` (struct fields aren't resolved
        // until the whole module is parsed), so this is the checker's job.
        let err = check_src("type: Bag xs [__spy 2] ; : main ( -- ) 0 . ;").unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_direct_element_in_word_signature_is_error() {
        let err = check_src(": w ( [__spy 2] -- ) | a | a drop ; : main ( -- ) 0 . ;").unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_indirect_via_linear_struct_field_is_error() {
        // `Arr`'s element (`Holds`) is not itself `__spy`, but contains one
        // transitively; `is_copy` already sees through that, so the sweep
        // over `module.arrays` must too.
        let err = check_src("type: Holds s __spy ; type: Arr a [Holds 2] ; : main ( -- ) 0 . ;")
            .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Holds`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_indirect_via_linear_struct_in_signature_is_error() {
        let err = check_src(
            "type: Holds s __spy ; : w ( [Holds 2] -- ) | a | a drop ; : main ( -- ) 0 . ;",
        )
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
        let err = check_src(": w ( ^[__spy 2] -- ) drop ; : main ( -- ) 0 . ;").unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
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

    // Phase 3 Slice 1: the linear core on bare `__spy` values.

    #[test]
    fn is_copy_every_type_but_the_spy() {
        for name in ["i8", "u64", "f32", "f64", "bool", "usize"] {
            assert!(
                is_copy(Type::from_name(name).unwrap(), &[], &[], &[]),
                "{name} is Copy"
            );
        }
        assert!(!is_copy(Type::Spy, &[], &[], &[]));
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
        // linear field (direct or nested) is linear, transitively.
        let tokens = lex("type: Plain x i64 y i64 ;\n\
type: Holds a __spy b i64 ;\n\
type: Wraps h Holds ;\n")
        .unwrap();
        let module = parse(&tokens).unwrap();
        let plain = Type::Struct(StructId::from_index(0), "Plain");
        let holds = Type::Struct(StructId::from_index(1), "Holds");
        let wraps = Type::Struct(StructId::from_index(2), "Wraps");
        assert!(is_copy(
            plain,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        assert!(!is_copy(
            holds,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        assert!(!is_copy(
            wraps,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
    }

    #[test]
    fn is_copy_enum_is_linear_iff_a_variant_field_is_transitively() {
        // R7/R12 (Phase 4): an enum with no linear variant field is Copy; one
        // with a linear field (direct in one variant, or nested through a
        // struct in another) is linear, transitively. `Plain` has no linear
        // variant, `Item` carries a spy directly in `Full`, `Boxed` carries
        // one nested inside `Holds`.
        let tokens = lex("type: Plain | A | B ;\n\
type: Item | Empty | Full v __spy ;\n\
type: Holds a __spy b i64 ;\n\
type: Boxed | Some h Holds | None ;\n")
        .unwrap();
        let module = parse(&tokens).unwrap();
        let plain = Type::Enum(EnumId::from_index(0), "Plain");
        let item = Type::Enum(EnumId::from_index(1), "Item");
        let boxed = Type::Enum(EnumId::from_index(2), "Boxed");
        assert!(is_copy(
            plain,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        assert!(!is_copy(
            item,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        assert!(!is_copy(
            boxed,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
    }

    #[test]
    fn check_spy_constructor_takes_an_i64_tag_ok() {
        check_src(": w ( -- ) 7 __spy drop ;").unwrap();
    }

    #[test]
    fn check_spy_constructor_on_a_float_tag_is_error() {
        let err = check_src(": w ( -- ) 7.5 __spy drop ;").unwrap_err();
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_dup_of_linear_value_is_error() {
        let err = check_src(": w ( -- ) 7 __spy dup drop drop ;").unwrap_err();
        assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
        assert!(err.contains("linear"), "unexpected message: {err}");
    }

    #[test]
    fn check_over_of_linear_value_is_error() {
        let err = check_src(": w ( -- ) 7 __spy 1 over drop drop drop ;").unwrap_err();
        assert!(err.contains("cannot `over`"), "unexpected message: {err}");
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shuffles_that_only_reorder_linear_values_are_ok() {
        // `swap`/`rot` move rather than copy, so the `dup`/`over` gate must not
        // over-reach to them.
        check_src(": w ( -- ) 7 __spy 8 __spy swap drop drop ;").unwrap();
        check_src(": w ( -- ) 1 __spy 2 __spy 3 __spy rot drop drop drop ;").unwrap();
    }

    #[test]
    fn check_print_on_linear_value_is_error() {
        // R16: `.` is a printable-scalar path, and a linear value is not one
        // (the backend's `unreachable!` guard depends on this).
        let err = check_src(": w ( -- ) 7 __spy . ;").unwrap_err();
        assert!(err.contains("printable"), "unexpected message: {err}");
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_use_after_move_of_linear_local_names_the_move_site() {
        let err = check_src(": w ( __spy -- )\n  | s |\n  s drop\n  s drop ;").unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
        assert!(
            err.contains("moved at line 3, col 3"),
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
        let err = check_src(": w ( __spy -- )\n  | s |\n  1 . ;").unwrap_err();
        assert!(err.contains("never consumed"), "unexpected message: {err}");
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
        assert!(
            err.contains("`s`"),
            "the error should name the local: {err}"
        );
    }

    #[test]
    fn check_surplus_linear_value_is_a_linear_flavoured_error() {
        let err = check_src(": w ( -- ) 7 __spy ;").unwrap_err();
        assert!(
            err.contains("linear value left on the stack"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
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
        check_src(": w ( __spy bool -- )\n  | s c |\n  c if s drop else s drop end ;").unwrap();
    }

    #[test]
    fn check_linear_local_moved_in_one_arm_then_used_is_error() {
        let err =
            check_src(": w ( __spy bool -- )\n  | s c |\n  c if s drop else 1 . end\n  s drop ;")
                .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_linear_local_moved_in_one_arm_and_dropped_nowhere_is_error() {
        let err = check_src(": w ( __spy bool -- )\n  | s c |\n  c if s drop else 1 . end ;")
            .unwrap_err();
        assert!(
            err.contains("not consumed on every path"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_linear_value_across_self_tail_call_is_error() {
        // R15: the fresh spy pushed in the recursive arm leaves `s` live across
        // the back-edge, which the loop lowering cannot dispose yet.
        let err = check_src(
            ": spin ( __spy i64 -- i64 )\n  | s n |\n  n 0 = if s drop 0 else 9 __spy n 1 - spin end ;",
        )
        .unwrap_err();
        assert!(
            err.contains("not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`__spy`"), "unexpected message: {err}");
        assert!(err.contains("line 3"), "the error should be located: {err}");
    }

    #[test]
    fn check_linear_value_forwarded_into_the_self_tail_call_is_ok() {
        // Moved *into* the recursive call's arguments, the spy is forwarded, not
        // stranded, so the R15 guard must not fire.
        check_src(
            ": spin ( __spy i64 -- i64 )\n  | s n |\n  n 0 = if s drop 0 else s n 1 - spin end ;",
        )
        .unwrap();
    }

    #[test]
    fn check_copy_self_tail_call_is_unaffected_by_the_linear_guard() {
        check_src(&std::fs::read_to_string("examples/countdown.sth").unwrap()).unwrap();
    }

    #[test]
    fn infer_line_consumes_a_carried_linear_slot_ok() {
        // The REPL path: a residual linear slot can be dropped by a later line
        // (no scope-end rule applies to a bare line).
        let out = infer_src("drop", &[Type::Spy]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn shared_reference_is_copy_and_mutable_reference_is_neither() {
        // R12's soundness answers: getting either wrong silently misclassifies
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
        // R21's peek route rests on this: two non-consuming projections of one
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
        // R21's alias check reads this: a field region is still an alias of
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
        // The asymmetry that makes `push-byte` legal while criterion 8 still
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
        // R10's join key. A reborrow of a reference *parameter* has no owned
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
        // R8's predicate is transitive: a struct that merely *reaches* a
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
}
