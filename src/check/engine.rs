//! The borrow/scope/liveness engine: the shared substrate every body-level
//! checker threads through the walk. Owns the region/alias/derivation arenas
//! (`RegionId`, `Alias`, `Deriv`, `Provenance`), the move-state and scope
//! bookkeeping (`Moves`, `Scope`/`Binding`), the last-use liveness analysis
//! (`Liveness` + its query helpers), and the walk context (`Ctx`/`word_ctx`,
//! `BlockEnd`). Engine-independent clusters do not touch these types; every
//! `engine`-dependent cluster imports them via `super::*`.

use std::cell::RefCell;

use crate::ast::GenericTypes;

use super::*;

/// Which region of memory an aggregate value denotes. Two slots carrying
/// the same id are two names for one address, which is what makes a mutation
/// through one silently observable through the other. `None` means "denotes a
/// region nothing else names": every value is born that way, and an aggregate
/// is given an id lazily, the first time something could alias it (a binding,
/// or a non-consuming projection out of it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RegionId(u32);

/// One live name for a region, and where that name was pushed. The span is
/// what lets the alias check report a *stack-resident* alias, which has no name
/// of its own to cite: an aggregate spends most of its life on the virtual
/// stack in this language, so the ability to locate one there is the difference
/// between catching the hazard and only catching the spelling of it where
/// both ends happen to be bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Alias {
    pub(super) set: AliasSetId,
    pub(super) span: Span,
}

/// The regions one value may denote, interned in `Provenance` so a `Slot` can
/// carry them by id and stay `Copy`. A value denotes more than one region only
/// where control flow merged two arms that named different places: the merge
/// cannot know which arm ran, so it keeps both and the borrow check tests every
/// member. Same trick as `DerivId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AliasSetId(u32);

/// One outstanding derivation from a place, interned in `Provenance` so a
/// `Slot` can carry it by id and stay `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct DerivId(u32);

/// What one live reference traces back to. Created by a fresh borrow
/// (`&v`/`&!v`), by naming a reference local (a reborrow), and by every
/// projection step, which copies its parent's chain rather than starting a new
/// one — that is what lets a check ask "does anything still trace back to this
/// place" without scanning for a particular value's identity.
#[derive(Debug, Clone)]
pub(super) struct Deriv {
    /// The place the reference was taken from: an owned aggregate local for a
    /// fresh borrow, the reference local itself for a reborrow.
    pub(super) place: String,
    /// The owned local at the bottom of the chain, if the chain starts at one.
    /// A reborrow of a reference *parameter* has none: its referent lives in an
    /// ancestor frame, so there is no place in this body to protect.
    pub(super) owned_root: Option<String>,
    /// R3 (P7 slice 2): whether that root is a module `static:` rather than a
    /// local. Recorded at the borrow, never re-derived by looking `owned_root`
    /// up in the static table: locals are not mangled and statics are, so a
    /// local spelled `COUNT__m0` in a module declaring `static: COUNT` answers
    /// that lookup and would inherit a static's exemptions.
    pub(super) static_root: bool,
    /// Whether `place` is a reference local this was reborrowed from, which is
    /// what the suspend rule is keyed on. Binding into a local does *not*
    /// clear this: the place stays suspended for as long as the bound
    /// reference is live. What lets `push-byte` name `b` three times is
    /// last-use liveness (6f) ending that suspension once the bound name's
    /// last use has passed, not the binding itself.
    pub(super) reborrow: bool,
    pub(super) mutable: bool,
    /// Whether any projection step stands between the place and this
    /// reference: the path-disjointness note is only apt when one does.
    pub(super) projected: bool,
    /// Where the borrow was taken, so a conflict can name both ends.
    pub(super) span: Span,
}

impl Deriv {
    /// The places this derivation keeps suspended, which is what a branch
    /// join has to agree on. Both halves are consulted by a hazard check
    /// (`owned_root` by the consume/borrow-conflict scans, the reborrowed
    /// reference local by the suspend rule), so both belong in the key: a join
    /// keeps only one arm's derivation, and any place the discarded arm
    /// suspended would silently stop being protected.
    pub(super) fn suspension(&self) -> (Option<&str>, Option<&str>) {
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
pub(super) struct SelfTailMarker {
    pub(super) name: String,
    pub(super) input_count: usize,
    /// Slice 10a (R11/R12): the ground declared inputs, against which the
    /// back-edge's self-call arguments (`stack[base..]`) are explicitly
    /// unified. Grounded once at the marker's sole set site, since the
    /// back-edge arm has no `sig`/`Subst` of its own.
    pub(super) ground_inputs: Vec<Type>,
    /// Slice 10a (R11): the ground declared outputs the back-edge arm
    /// produces (the old code fictionally produced the non-quotation inputs,
    /// which is right only for `while`'s state-threading shape).
    pub(super) ground_outputs: Vec<Type>,
    /// Slice 10a (R11): the bottom-aligned index map -- ground output `i`
    /// forwards provenance from the `index_map[i]`-th non-quotation carried
    /// input (both counted deepest-first), or `None` when `i` is beyond the
    /// carried-input count or the types differ. Phase 6 (R14) forwards
    /// `surviving`/`quot` along it.
    pub(super) index_map: Vec<Option<usize>>,
}

#[derive(Debug, Default)]
pub(super) struct Provenance {
    pub(super) derivs: Vec<Deriv>,
    pub(super) regions: u32,
    /// The interned region of one non-consuming projection out of a parent
    /// region, so two peeks of the same field yield one id.
    pub(super) fields: HashMap<(u32, String), RegionId>,
    /// The interned region sets, indexed by `AliasSetId`.
    pub(super) alias_sets: Vec<Vec<RegionId>>,
    /// Each field region's immediate parent: a name for a struct's
    /// field is still a name for part of the whole struct, so the alias check
    /// has to test region *overlap* along this chain, not bare equality.
    pub(super) parents: HashMap<u32, RegionId>,
    /// R6 (slice 8b): the resolved operand type of every `drop` call site in
    /// this body, in the order the walk reaches them. Nothing in the walk
    /// reads it back; the body walkers hand it to `check`, which needs the
    /// *type* each `drop` resolves to in order to tell `drop@File` from a
    /// `drop` of a plain `i64` — a distinction no purely syntactic pass over
    /// callee names can make. It rides this arena for the same reason the
    /// arena is threaded at all: an `if` arm clones `Scope`, so an
    /// observation kept there would die with the arm.
    pub(super) dropped: Vec<Type>,
    /// D2/R4: the per-check quotation-literal side table, indexed by `QuotId`.
    /// A quotation `Slot`/`Binding` carries only a `QuotId`, so the body it
    /// marks is interned here and spliced from here at `call`/`times`. Rides
    /// this arena because it is the one scratch already threaded `&mut`
    /// through the walk, so a quotation pushed in one `if` arm and read in a
    /// merge outlives the arm's cloned `Scope`.
    pub(super) quotations: Vec<QuotBody>,
    /// 6f: each quotation's free-name set (every outer local its body reads,
    /// sigil-stripped, minus whatever the body binds and shadows itself),
    /// computed once at intern time and indexed in lockstep with
    /// `quotations` by `QuotId`. A pure function of the literal's own AST, so
    /// it needs no invalidation and is safe to cache: whether a captured name
    /// is actually still live is a separate, per-query question answered by
    /// `capture_alive_names`, not baked in here.
    pub(super) quotation_captures: Vec<HashSet<String>>,
    /// 7b/R19: the side table of surviving capture sets, keyed by
    /// `SurvivingCaptureSetId`, mirroring how `quotation_captures` stores
    /// capture sets by `QuotId`. An erased capturing quotation's aggregate and
    /// borrow captures (never its scalar snapshots) ride here so
    /// `capture_alive_names` (R20) and the R22 escape guard can read them past
    /// erasure, when the `QuotRef::Known` marker is gone.
    pub(super) surviving_sets: Vec<SurvivingSet>,
    /// R6/R14: the self-tail combinator currently being spliced (its name and
    /// its declared input arity), set for the duration of that body splice. A
    /// tail-position call to that same name reached inside the spliced body is
    /// the loop back-edge, not a re-splice: it discharges the two move/borrow
    /// obligations and produces the combinator's carried state, terminating
    /// the branch. Saved and restored around the splice so loops compose.
    pub(super) self_tail_combinator: Option<SelfTailMarker>,
    /// R18/R21: a monotonic counter minting a fresh suffix each time a
    /// combinator body is spliced, so the callee's `| ... |` locals are
    /// alpha-renamed to names no caller local (or outer combinator, under
    /// transitive inlining) can collide with. Term-splice binds names in the
    /// caller's scope (R18: binding, not string rewriting), so without this a
    /// nested `each` inside a `map` would re-bind the outer `arr`/`f`.
    pub(super) inline_uid: u32,
}

impl Provenance {
    pub(super) fn fresh_region(&mut self) -> RegionId {
        let id = RegionId(self.regions);
        self.regions += 1;
        id
    }

    pub(super) fn intern_alias_set(&mut self, mut regions: Vec<RegionId>) -> AliasSetId {
        regions.sort_unstable_by_key(|r| r.0);
        regions.dedup();
        if let Some(i) = self.alias_sets.iter().position(|s| *s == regions) {
            return AliasSetId(i as u32);
        }
        self.alias_sets.push(regions);
        AliasSetId((self.alias_sets.len() - 1) as u32)
    }

    pub(super) fn alias_set_of(&mut self, region: RegionId) -> AliasSetId {
        self.intern_alias_set(vec![region])
    }

    pub(super) fn alias_regions(&self, id: AliasSetId) -> &[RegionId] {
        &self.alias_sets[id.0 as usize]
    }

    /// Both arms' regions, since either runtime path may have produced the
    /// merged value.
    pub(super) fn alias_union(&mut self, a: AliasSetId, b: AliasSetId) -> AliasSetId {
        let mut regions = self.alias_regions(a).to_vec();
        regions.extend_from_slice(self.alias_regions(b));
        self.intern_alias_set(regions)
    }

    /// Whether any region of one value overlaps any region of the other.
    pub(super) fn alias_sets_overlap(&self, a: AliasSetId, b: AliasSetId) -> bool {
        self.alias_regions(a).iter().any(|x| {
            self.alias_regions(b)
                .iter()
                .any(|y| self.regions_overlap(*x, *y))
        })
    }

    /// The same field projected out of every region the parent may denote.
    pub(super) fn field_alias_set(&mut self, parent: AliasSetId, segment: &str) -> AliasSetId {
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
    pub(super) fn field_region(&mut self, parent: RegionId, segment: &str) -> RegionId {
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
    pub(super) fn regions_overlap(&self, a: RegionId, b: RegionId) -> bool {
        a == b || self.is_ancestor(a, b) || self.is_ancestor(b, a)
    }

    pub(super) fn is_ancestor(&self, ancestor: RegionId, mut descendant: RegionId) -> bool {
        while let Some(&parent) = self.parents.get(&descendant.0) {
            if parent == ancestor {
                return true;
            }
            descendant = parent;
        }
        false
    }

    pub(super) fn deriv(&self, id: DerivId) -> &Deriv {
        &self.derivs[id.0 as usize]
    }

    /// The free names cached for quotation `id` at intern time (`capture_names`).
    pub(super) fn quotation_captures(&self, id: QuotId) -> &HashSet<String> {
        &self.quotation_captures[id.0]
    }

    /// 7b/R19: the surviving capture set behind `id`.
    pub(super) fn surviving_set(&self, id: SurvivingCaptureSetId) -> &[SurvivingCapture] {
        &self.surviving_sets[id.0 as usize].members
    }

    /// Review fix: whether the closure behind `id` needed a stack-allocated
    /// env bundle (2+ total captures, R16) rather than an inline single-word
    /// env. Read by the R22 word-output escape guard alongside `frame_rooted`.
    pub(super) fn surviving_set_is_bundle(&self, id: SurvivingCaptureSetId) -> bool {
        self.surviving_sets[id.0 as usize].bundle
    }

    /// 7b/R19: intern a surviving capture set, or `None` if it holds nothing
    /// the R22 word-output guard must watch. That is empty members *and* no
    /// bundle: a closure snapshotting only scalars into an inline env has no
    /// referent and no bundle storage that can go dead. But an all-scalar 2+
    /// capture still allocates a *stack* env bundle (R16) whose own storage
    /// dies at return, so it must keep a set (empty members, `bundle = true`)
    /// to carry that signal onto a carrier for R22 to reject.
    pub(super) fn intern_surviving_set(
        &mut self,
        mut members: Vec<SurvivingCapture>,
        bundle: bool,
    ) -> Option<SurvivingCaptureSetId> {
        members.sort_by(|a, b| a.name.cmp(&b.name));
        members.dedup();
        if members.is_empty() && !bundle {
            return None;
        }
        let id = SurvivingCaptureSetId(self.surviving_sets.len() as u32);
        self.surviving_sets.push(SurvivingSet { members, bundle });
        Some(id)
    }

    /// 7b/R23: a fresh interned set holding the union of two surviving sets
    /// (either or both may be absent). Never mutates an existing set in place,
    /// so a joined value's set is independent of its arms'.
    pub(super) fn union_surviving(
        &mut self,
        a: Option<SurvivingCaptureSetId>,
        b: Option<SurvivingCaptureSetId>,
    ) -> Option<SurvivingCaptureSetId> {
        let mut members: Vec<SurvivingCapture> = Vec::new();
        let mut bundle = false;
        for id in [a, b].into_iter().flatten() {
            members.extend_from_slice(self.surviving_set(id));
            bundle |= self.surviving_set_is_bundle(id);
        }
        self.intern_surviving_set(members, bundle)
    }

    pub(super) fn add(&mut self, deriv: Deriv) -> DerivId {
        let id = DerivId(self.derivs.len() as u32);
        self.derivs.push(deriv);
        id
    }

    /// A fresh borrow of an owned aggregate place, or (R3) of a module static.
    pub(super) fn borrow(
        &mut self,
        place: &str,
        mutable: bool,
        static_root: bool,
        span: Span,
    ) -> DerivId {
        self.add(Deriv {
            place: place.to_string(),
            owned_root: Some(place.to_string()),
            static_root,
            reborrow: false,
            mutable,
            projected: false,
            span,
        })
    }

    /// Naming a reference local reborrows it — a new chain rooted at that
    /// local, keeping whatever owned place the old chain had already reached.
    pub(super) fn reborrow(
        &mut self,
        place: &str,
        held: Option<DerivId>,
        mutable: bool,
        span: Span,
    ) -> DerivId {
        let owned_root = held.and_then(|id| self.deriv(id).owned_root.clone());
        let static_root = held.is_some_and(|id| self.deriv(id).static_root);
        self.add(Deriv {
            place: place.to_string(),
            owned_root,
            static_root,
            reborrow: true,
            mutable,
            projected: false,
            span,
        })
    }

    /// One projection step — the same place, one step further from it.
    pub(super) fn project(&mut self, parent: Option<DerivId>) -> Option<DerivId> {
        let deriv = Deriv {
            projected: true,
            ..self.deriv(parent?).clone()
        };
        Some(self.add(deriv))
    }
}

/// R14: the move-state of one linear local, a three-value lattice. `Moved` and
/// `MaybeMoved` carry the site that consumed the value, so a later use can name
/// it; `MaybeMoved` is the join of disagreeing arms (consumed on one path only),
/// which is neither usable nor accepted as disposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MoveState {
    Live,
    Moved(Span),
    MaybeMoved(Span),
}

/// The move-state of every *linear* name in scope, carried by the `Scope` the
/// walker threads (R14). A Copy local never appears: it carries no ownership
/// obligation, so mentioning it twice is ordinary reuse.
#[derive(Debug, Clone, Default)]
pub(super) struct Moves {
    pub(super) states: HashMap<String, MoveState>,
}

impl Moves {
    /// R3 (D2): mentioning a linear local moves its value out. `Ok(())` for a
    /// Copy local (absent from the map) or a first mention; `Err(site)` names
    /// the move that already consumed it.
    pub(super) fn take(&mut self, name: &str, span: Span) -> Result<(), Span> {
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
    pub(super) fn moved_site(&self, name: &str) -> Option<Span> {
        match self.states.get(name) {
            Some(MoveState::Moved(site) | MoveState::MaybeMoved(site)) => Some(*site),
            _ => None,
        }
    }

    /// The locals still holding an unconsumed value: `Live` (never mentioned)
    /// or `MaybeMoved` (consumed on one branch only), name-sorted so a scope
    /// with two of them always reports the same one.
    pub(super) fn unconsumed(&self) -> Vec<&str> {
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
    pub(super) fn join(then_arm: Moves, else_arm: Moves) -> Moves {
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
pub(super) struct Scope {
    pub(super) bound: Vec<Binding>,
    pub(super) moves: Moves,
}

/// One name in scope, with the provenance a borrow check reads off it: which
/// region an aggregate binding denotes and which derivation a reference
/// binding holds.
#[derive(Debug, Clone)]
pub(super) struct Binding {
    pub(super) name: String,
    pub(super) ty: Type,
    pub(super) aliases: Option<AliasSetId>,
    pub(super) deriv: Option<DerivId>,
    /// D2/R4: a bound quotation's marker. A local read reconstructs a fresh
    /// `Slot` that drops every non-`ty` side channel, so unlike a shuffle a
    /// bind is a *second*, explicit forwarding site: this field carries the
    /// marker across the bind and back onto the reconstructed slot.
    pub(super) quot: Option<QuotRef>,
    /// 7b/R19: the surviving capture set an erased quotation (or an aggregate
    /// carrying one) holds, carried across the bind exactly like `quot` so a
    /// stored-closure binding keeps its captures' referents live (R20).
    pub(super) surviving: Option<SurvivingCaptureSetId>,
}

impl Scope {
    pub(super) fn depth(&self) -> usize {
        self.bound.len()
    }

    pub(super) fn local(&self, name: &str) -> Option<&Binding> {
        self.bound.iter().find(|b| b.name == name)
    }

    pub(super) fn local_type(&self, name: &str) -> Option<Type> {
        self.local(name).map(|b| b.ty)
    }

    /// Bring `name` into scope. A linear value also enters the move-state map,
    /// so forgetting it is caught at the end of its block. An aggregate
    /// with no region of its own gets one here: a binding is the first point at
    /// which a second name could denote the same address.
    pub(super) fn bind(&mut self, name: &str, slot: Slot, linear: bool, prov: &mut Provenance) {
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
            deriv: slot.deriv,
            quot: slot.quot,
            surviving: slot.surviving,
        });
    }

    /// Take every name bound past `depth` out of scope, returning the first one
    /// (name-sorted, so a block leaking two always reports the same one) still
    /// holding a linear value.
    pub(super) fn leave(&mut self, depth: usize) -> Option<(String, Type, MoveState)> {
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

/// The bare local name a `Call` denotes, with any leading `&!`/`&` borrow
/// sigil stripped (matching `ast::rename_call`'s notion of a use of a local).
pub(super) fn call_local(name: &str) -> &str {
    name.strip_prefix("&!")
        .or_else(|| name.strip_prefix('&'))
        .unwrap_or(name)
}

/// Every free name a quotation body reads: a `Call` whose bare-or-sigilled
/// target is not itself bound *inside* this body (shadowing), at any depth,
/// recursing into `if` arms and nested quotation literals. Computed once per
/// literal at intern time and cached on `Provenance` by `QuotId` -- it is a
/// pure property of the literal's own AST, independent of where or when it
/// gets called. Over-includes ordinary word names (there is no way to tell a
/// local reference from a word call at this syntactic layer), which is
/// harmless: `capture_alive_names` only ever intersects this set against
/// actual scope bindings, so a name that never denotes a local simply never
/// matches anything.
pub(super) fn capture_names(body: &[Term]) -> HashSet<String> {
    let mut out = HashSet::new();
    capture_names_into(body, &mut HashSet::new(), &mut out);
    out
}

pub(super) fn capture_names_into(
    terms: &[Term],
    shadowed: &mut HashSet<String>,
    out: &mut HashSet<String>,
) {
    for term in terms {
        match &term.kind {
            TermKind::Bind(names) => {
                shadowed.extend(names.iter().cloned());
            }
            TermKind::Call(name) => {
                let local = call_local(name);
                if !shadowed.contains(local) {
                    out.insert(local.to_string());
                }
            }
            TermKind::Quotation(inner, _, _) => {
                capture_names_into(inner, &mut shadowed.clone(), out);
            }
            _ => {}
        }
    }
}

/// Q1/D3: the last-use index, within one `check_terms` invocation's term list,
/// of every reference/aggregate name that invocation *binds*. A name used but
/// not bound here (an outer local inherited into this block) is absent from
/// `last_use` UNLESS the caller has established it as `outer_releasable`
/// (D6's relaxation, below): once granted, it is tracked exactly like a name
/// this invocation binds. A name is *used* by a `TermKind::Call` whose
/// bare-or-sigilled target resolves to it, at any depth; a use inside a
/// nested `if` arm or quotation literal is attributed to the index of the
/// top-level term of *this* list that contains it (Q3's conservative max).
///
/// This is a floor, not the whole story, for a quotation: a literal bound to
/// a local (`[ ... ] | q |`) does not execute at its own position, it
/// executes wherever `q` is later used, so a name it captures must not die
/// before `q` itself does -- and, transitively, before whatever *else*
/// captures `q`'s name does. Rather than infer that syntactically here, the
/// checker already records the association on both carriers a quotation can
/// occupy (`Slot.quot`, `Binding.quot`); `capture_alive_names` reads that
/// directly, at query time, in `live_derivs` and `aliasing_origin`. This scan
/// treats every quotation literal exactly like an `if` arm, giving each
/// capture its floor; `capture_alive_names` is strictly additive on top.
///
/// D6, relaxed: a name bound by an *ancestor* invocation is only ever a
/// candidate for early death here if the caller first proves it has no
/// residual use anywhere past this block (`outer_releasable`, computed at
/// each recursion site from "not referenced in the remaining sibling terms"
/// composed with whatever the caller itself was granted). An `if` arm is
/// execute-once, so a granted name gets the same fine last-use tracking as a
/// name bound here (`back_edge = false`): it may die at its own last call
/// inside the arm. A `times`/quotation body can run more than once or be
/// invoked from elsewhere, so a granted name used anywhere inside is pinned
/// live for the *whole* body (`back_edge = true`, sentinel `usize::MAX`);
/// only a name unused anywhere in the body dies, and it dies throughout.
/// Either way this only *adds* candidates for death beyond what the plain
/// bound-here rule already grants (D1: monotone).
pub(super) struct Liveness {
    pub(super) last_use: HashMap<String, usize>,
    pub(super) outer_releasable: HashSet<String>,
}

/// Sentinel `last_use` for a name proven used somewhere inside a back-edge
/// body: never `< at` for any real term index, so it stays live for the
/// whole body rather than dying at its first (or any) use inside it.
pub(super) const IMMORTAL_IN_BODY: usize = usize::MAX;

impl Liveness {
    pub(super) fn scan(
        terms: &[Term],
        outer_releasable: &HashSet<String>,
        back_edge: bool,
    ) -> Self {
        let mut last_use = HashMap::new();
        let mut bound: HashSet<String> = HashSet::new();
        for (i, term) in terms.iter().enumerate() {
            match &term.kind {
                TermKind::Bind(names) => {
                    for name in names {
                        bound.insert(name.clone());
                        // The bind index is the floor of a name's last use: a
                        // binding never mentioned again is dead from the term
                        // after its bind (D5), not live for the whole block.
                        last_use.insert(name.clone(), i);
                    }
                }
                TermKind::Call(name) => {
                    let local = call_local(name);
                    if bound.contains(local) {
                        last_use.insert(local.to_string(), i);
                    } else if outer_releasable.contains(local) {
                        Self::record_granted_use(&mut last_use, local, i, back_edge);
                    }
                }
                // A nested block is its own `check_terms` invocation with its
                // own binds; here we only look for uses of names *this* list
                // bound (or was granted), attributed to the containing
                // top-level index (Q3's conservative max). Nested binds are
                // not collected: they belong to the nested invocation's own
                // scan. This is only a floor
                // (`capture_alive_names` extends it for a quotation the
                // checker later finds is still reachable, bound or not).
                TermKind::Quotation(inner, _, _) => {
                    Self::nested_uses(inner, &bound, outer_releasable, back_edge, i, &mut last_use);
                }
                _ => {}
            }
        }
        Liveness {
            last_use,
            outer_releasable: outer_releasable.clone(),
        }
    }

    /// Record a use of a granted (outer-releasable) name at index `at`: fine
    /// last-use tracking for an execute-once block, or the immortal sentinel
    /// for a back-edge body (see the struct doc).
    pub(super) fn record_granted_use(
        last_use: &mut HashMap<String, usize>,
        local: &str,
        at: usize,
        back_edge: bool,
    ) {
        let value = if back_edge { IMMORTAL_IN_BODY } else { at };
        let entry = last_use.entry(local.to_string()).or_insert(value);
        *entry = (*entry).max(value);
    }

    pub(super) fn nested_uses(
        terms: &[Term],
        bound: &HashSet<String>,
        outer_releasable: &HashSet<String>,
        back_edge: bool,
        at: usize,
        last_use: &mut HashMap<String, usize>,
    ) {
        for term in terms {
            match &term.kind {
                TermKind::Call(name) => {
                    let local = call_local(name);
                    if bound.contains(local) {
                        let entry = last_use.entry(local.to_string()).or_insert(at);
                        *entry = (*entry).max(at);
                    } else if outer_releasable.contains(local) {
                        Self::record_granted_use(last_use, local, at, back_edge);
                    }
                }
                TermKind::Quotation(inner, _, _) => {
                    Self::nested_uses(inner, bound, outer_releasable, back_edge, at, last_use);
                }
                _ => {}
            }
        }
    }

    /// A binding is dead at term index `at` iff its last use (its bind index
    /// when never mentioned again) is strictly before `at`. A name this
    /// invocation neither bound nor used at all is dead throughout iff the
    /// caller granted it (`outer_releasable`); otherwise D6's original rule
    /// holds unchanged: an outer name with no entry is never dead here.
    pub(super) fn dead(&self, name: &str, at: usize) -> bool {
        match self.last_use.get(name) {
            Some(&last) => last < at,
            None => self.outer_releasable.contains(name),
        }
    }
}

/// Whether `name` (already sigil-stripped) is referenced by a `TermKind::Call`
/// anywhere in `terms`, at any nesting depth (an `if` arm, a quotation
/// literal). Used to ask "is there a residual use past this point" for the
/// D6 relaxation above; a name still in scope may not be rebound while live
/// (`rebound_local_error`), so there is no shadowing case to exclude here.
pub(super) fn references(terms: &[Term], name: &str) -> bool {
    terms.iter().any(|term| match &term.kind {
        TermKind::Call(n) => call_local(n) == name,
        TermKind::Quotation(inner, _, _) => references(inner, name),
        _ => false,
    })
}

/// D6 relaxation: the set of ancestor-bound names safe to grant into a nested
/// block starting right after term index `at` in the current invocation.
///
/// A name bound *within* the current invocation (position `>= base_depth`, so
/// nothing outside this invocation could ever need it) qualifies iff it is not
/// referenced anywhere in `rest` (the current invocation's own remaining
/// sibling terms after `at`) -- a sibling `if` arm's own uses stay invisible
/// here because they live *inside* `terms[at]`, not in `rest`.
///
/// A name bound by an *ancestor* invocation must already have been granted to
/// this one, and is asked of this invocation's own `Liveness` instead: `rest`
/// alone is the wrong question inside a `back_edge = true` body, where
/// execution wraps around to term 0 and so a use *earlier* in the body is
/// still ahead of this block. `dead` answers that correctly, because a granted
/// name mentioned anywhere in a back-edge body is recorded `IMMORTAL_IN_BODY`.
/// The index is `at + 1`, not `at`: `nested_uses` attributes a use found
/// inside `terms[at]` to `at` itself, so `dead(name, at)` is false exactly
/// when the block being granted into is the user -- the entire reason to
/// grant. Asking at `at + 1` reproduces "no residual use after this term",
/// which is what `!references(rest, name)` meant, and since `scan` and
/// `references` traverse the identical nesting variants this branch grants a
/// strict subset of what the plain rule would.
pub(super) fn releasable_into(
    scope: &Scope,
    base_depth: usize,
    outer_releasable: &HashSet<String>,
    rest: &[Term],
    live: &Liveness,
    at: usize,
) -> HashSet<String> {
    scope
        .bound
        .iter()
        .enumerate()
        .filter(|(idx, b)| {
            if *idx >= base_depth {
                !references(rest, &b.name)
            } else {
                outer_releasable.contains(&b.name) && live.dead(&b.name, at + 1)
            }
        })
        .map(|(_, b)| b.name.clone())
        .collect()
}

/// Every name kept alive by a still-live quotation, at query index `at`: a
/// quotation on the virtual stack is unconditionally live (same as any other
/// stack-resident value -- nothing has consumed it yet, whether or not it is
/// ever bound), and a quotation held by a scope binding is live if that
/// binding itself is (by the ordinary last-use rule, *or* because it is
/// itself a name this same computation has already found alive). The second
/// disjunct is what makes a quotation capturing a quotation transitive
/// (`[ q1 call ] | q2 |`): once `q2` is found alive, `q1` -- a name `q2`'s
/// body merely calls -- is added, and the next pass then reads `q1`'s own
/// capture set. The graph is acyclic (a quotation can only capture a name
/// already bound earlier in program order), so this always terminates.
///
/// Only ever *extends* what `live_derivs`/`aliasing_origin` already treat as
/// live; it is never consulted to shorten anything (D1: monotone).
/// 7b/R20: the same rule extended past erasure. A `Known` marker's captures
/// come from `quotation_captures`; an *erased* closure (or an aggregate
/// carrying one) has no marker, so `include_surviving` unions the names from
/// its `surviving` set (R19) instead -- keeping a captured borrow's referent
/// live exactly as the `Known` case does. `include_surviving = false` yields
/// the pre-erasure (marker-only) view the R24 past-last-use check diffs
/// against, so that check fires only for captures the surviving set added.
pub(super) fn capture_alive_names(
    stack: &[Slot],
    scope: &Scope,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
) -> HashSet<String> {
    capture_alive_names_impl(stack, scope, prov, live, at, true)
}

pub(super) fn capture_alive_names_impl(
    stack: &[Slot],
    scope: &Scope,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
    include_surviving: bool,
) -> HashSet<String> {
    let mut alive: HashSet<String> = HashSet::new();
    let mut changed = true;
    while changed {
        changed = false;
        for slot in stack {
            if let Some(QuotRef::Known(id)) = slot.quot {
                for name in prov.quotation_captures(id) {
                    if alive.insert(name.clone()) {
                        changed = true;
                    }
                }
            }
            // A stack-resident value is unconditionally live: an erased
            // closure (or carrier) there keeps its surviving captures alive.
            if include_surviving {
                if let Some(set) = slot.surviving {
                    for member in prov.surviving_set(set) {
                        if alive.insert(member.name.clone()) {
                            changed = true;
                        }
                    }
                }
            }
        }
        for b in &scope.bound {
            let base_alive = !live.dead(&b.name, at);
            let alive_here = base_alive || alive.contains(&b.name);
            if let (true, Some(QuotRef::Known(id))) = (alive_here, b.quot) {
                for name in prov.quotation_captures(id) {
                    if alive.insert(name.clone()) {
                        changed = true;
                    }
                }
            }
            if include_surviving && alive_here {
                if let Some(set) = b.surviving {
                    for member in prov.surviving_set(set) {
                        if alive.insert(member.name.clone()) {
                            changed = true;
                        }
                    }
                }
            }
        }
    }
    alive
}

/// 7b/R24: if the reference-local holding derivation `id` is dead by ordinary
/// liveness (its last syntactic use is past) and is kept alive *only* by an
/// erased closure's surviving-set union (R20) -- not by a still-`Known` marker
/// -- return its name. This is the signal that a conflicting borrow/consume of
/// its referent is reading a captured reference past its last use, so the
/// past-last-use wording (R24) applies instead of the generic
/// conflicting-borrow one. Dropping R20's union empties the `full` set, so this
/// returns `None` and the rejection disappears entirely (mutation test M2).
pub(super) fn past_last_use_capture(
    stack: &[Slot],
    scope: &Scope,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
    id: DerivId,
) -> Option<String> {
    let holder = scope.bound.iter().find(|b| b.deriv == Some(id))?;
    if !live.dead(&holder.name, at) {
        return None;
    }
    let full = capture_alive_names_impl(stack, scope, prov, live, at, true);
    if !full.contains(&holder.name) {
        return None;
    }
    let known = capture_alive_names_impl(stack, scope, prov, live, at, false);
    if known.contains(&holder.name) {
        return None;
    }
    Some(holder.name.clone())
}

/// Every derivation still live — held by a slot on the virtual stack, or by
/// a reference-typed local whose binding is not yet dead at term index `at`
/// (Q1/D2), or whose name a still-live quotation captures
/// (`capture_alive_names`). A reference is live from the term that creates it
/// until the term that consumes its slot; a reference *local* is live from
/// its bind to its last use in this block, extended for as long as a
/// quotation that can still reach it survives.
pub(super) fn live_derivs<'a>(
    stack: &'a [Slot],
    scope: &'a Scope,
    prov: &'a Provenance,
    live: &'a Liveness,
    at: usize,
) -> impl Iterator<Item = DerivId> + 'a {
    let captured = capture_alive_names(stack, scope, prov, live, at);
    stack.iter().filter_map(|slot| slot.deriv).chain(
        scope
            .bound
            .iter()
            .filter(move |b| !live.dead(&b.name, at) || captured.contains(&b.name))
            .filter_map(|b| b.deriv),
    )
}

/// The first live derivation satisfying `pred`. The scan is over
/// provenance, never over value identity: a reference two projection steps
/// removed from a place is still a derivation of that place.
pub(super) fn live_deriv(
    stack: &[Slot],
    scope: &Scope,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
    mut pred: impl FnMut(&Deriv) -> bool,
) -> Option<DerivId> {
    live_derivs(stack, scope, prov, live, at).find(|id| pred(prov.deriv(*id)))
}

/// A live borrow rooted at the owned place `place`, whatever its
/// mutability and however many projection steps away.
pub(super) fn live_borrow_of(
    stack: &[Slot],
    scope: &Scope,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
    place: &str,
) -> Option<DerivId> {
    live_deriv(stack, scope, prov, live, at, |d| {
        d.owned_root.as_deref() == Some(place)
    })
}

/// The naming side: a live *mutable* borrow rooted at `place`, which any new
/// name for that place would then silently observe mutations through.
pub(super) fn live_mutable_borrow_of(
    stack: &[Slot],
    scope: &Scope,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
    place: &str,
) -> Option<DerivId> {
    live_deriv(stack, scope, prov, live, at, |d| {
        d.mutable && d.owned_root.as_deref() == Some(place)
    })
}

/// The interior region a non-consuming projection out of `parent` denotes,
/// minting `parent`'s own region lazily. P7 slice 1's `&f`/`&!f` hands out a
/// *reference* to the interior, which aliases its parent whatever the
/// field's width.
pub(super) fn projected_region(
    parent: &mut Slot,
    segment: &str,
    span: Span,
    prov: &mut Provenance,
) -> Alias {
    let base = match parent.alias {
        Some(alias) => alias.set,
        None => {
            let region = prov.fresh_region();
            let set = prov.alias_set_of(region);
            parent.alias = Some(Alias { set, span });
            set
        }
    };
    Alias {
        set: prov.field_alias_set(base, segment),
        span,
    }
}

/// Where a second live name for a region is, when the diagnostic has to
/// point at it. A bound local reports its name, which is what the user has to
/// change; a value still on the virtual stack has no name, so it reports the
/// site that pushed it instead.
pub(super) enum AliasOrigin<'a> {
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
/// it never aliases; nor is a name whose last use has passed (Q6/D8): filtering
/// each candidate by its *own* last use preserves overlap — a dead name A is
/// dropped while any still-live name B overlapping `place` keeps rejecting, so
/// the borrow is accepted only when no live name can observe the mutation.
pub(super) fn aliasing_origin<'a>(
    stack: &[Slot],
    scope: &'a Scope,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
    place: &str,
) -> Option<AliasOrigin<'a>> {
    let set = scope.local(place)?.aliases?;
    let overlaps = |other: AliasSetId| prov.alias_sets_overlap(set, other);
    let captured = capture_alive_names(stack, scope, prov, live, at);
    let mut names: Vec<&str> = scope
        .bound
        .iter()
        .filter(|b| {
            b.name != place
                && b.aliases.is_some_and(&overlaps)
                && scope.moves.moved_site(&b.name).is_none()
                && (!live.dead(&b.name, at) || captured.contains(&b.name))
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
pub(super) enum BlockEnd {
    Body(u32),
    Arm { token: &'static str, span: Span },
}

/// Error context for the shared stack simulation: a full word (with its
/// declared effect to cite) or a bare REPL line (no signature to cite).
/// Both carry the struct/enum registries `is_copy` needs to resolve a
/// `Type::Struct`/`Type::Enum`'s linearity, so `dup`/`over`/back-edge checking
/// works identically whether the caller is a compiled word or a REPL line.
pub(super) enum Ctx<'a> {
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
        /// R1 (P7 slice 2): the closure's `static:` declarations, which the
        /// borrow-typing arm consults for the second kind of place a `&`/`&!`
        /// can name. Scoped to `module` at every lookup: a static is
        /// module-private.
        statics: &'a [StaticDecl],
        /// R2 (slice 8b): the owning module of the word being checked, the
        /// caller module D1's `drop` gate and 8a's operator fix scope a name's
        /// visibility against.
        module: u32,
        /// R3 (slice 8b): the import closure's per-module data, `Some` on the
        /// native build path and `None` on the REPL path (`infer_line` builds
        /// `Ctx::Line`, and a retained poly word passes `None`): the gate reads
        /// it and never fires when it is absent.
        modules: Option<&'a [ModuleInfo]>,
        /// Slice 10c (review fix, Phase 1): whether lowering actually builds a
        /// splice-time back-edge for this word's own self-tail call
        /// (`has_self_tail_call`, the same predicate `inline_combinator` and
        /// every lowering site consult). The back-edge-only linear/reference
        /// guards (R15) must gate on this, not on the syntactic `tail` flag
        /// alone: `TailWalk` declines (a forwarded quotation reached through a
        /// mid-body local, an ambiguous name, a forwarding cycle) in cases
        /// where the positional `tail` flag still reaches the recursive call,
        /// and there lowering emits an ordinary `Instr::Call` with no back
        /// edge to guard.
        self_tail_call: bool,
        /// P7 slice 3a phase 2 (R2): the live generic instantiator, `Some`
        /// on the native build path (`check::check`), `None` everywhere else
        /// (the REPL never declares its own generic `type:`, so no session
        /// poly word's signature can ever carry a `PolyType::Generic` -- see
        /// `GenericTypes`'s own doc). A `RefCell` because `Ctx` otherwise only
        /// ever borrows immutably, but grounding a generic mid-body-walk
        /// (`unify_poly_input`/`apply_subst`'s `Generic` arms, and
        /// `poly_call_term`'s construction arm) needs to mint through it.
        generics: Option<&'a RefCell<GenericTypes>>,
    },
    Line {
        structs: &'a [StructDecl],
        enums: &'a [EnumDecl],
    },
}

/// The `Ctx` for checking `word`'s body: shared by the body walkers and the
/// binding-name rejections so all of them cite the same declared effect.
/// `combs` is the tail-splice view `has_self_tail_call` reads to decide
/// `self_tail_call`; pass an empty index at a call site whose checking path
/// never reaches the back-edge guard (it stays `false`, matching lowering,
/// which never back-edges there either).
pub(super) fn word_ctx<'a>(
    word: &'a WordDef,
    structs: &'a [StructDecl],
    enums: &'a [EnumDecl],
    statics: &'a [StaticDecl],
    modules: Option<&'a [ModuleInfo]>,
    combs: &CombinatorIndex,
    generics: Option<&'a RefCell<GenericTypes>>,
) -> Ctx<'a> {
    Ctx::Word {
        name: crate::resolve::demangle_word(&word.name),
        mangled: &word.name,
        effect: &word.effect,
        structs,
        enums,
        statics,
        module: word.module,
        modules,
        self_tail_call: has_self_tail_call(word, combs),
        generics,
    }
}

impl Ctx<'_> {
    pub(super) fn structs(&self) -> &[StructDecl] {
        match self {
            Ctx::Word { structs, .. } | Ctx::Line { structs, .. } => structs,
        }
    }

    pub(super) fn enums(&self) -> &[EnumDecl] {
        match self {
            Ctx::Word { enums, .. } | Ctx::Line { enums, .. } => enums,
        }
    }

    /// R1 (P7 slice 2): the declared type of the static `name`, or `None`
    /// when no static of that name reached this check. `resolve::mangle` is
    /// unconditional per module (R2), so by the time `check::check` runs,
    /// `name` already carries its declaring module baked in (`COUNT__m1`) --
    /// matching on it alone is what module-private lookup means post-mangle.
    /// An additional `s.module == ctx.module()` filter is not just redundant
    /// but wrong: a combinator splice checks the caller's own quotation
    /// arguments under the callee's module (`inline_combinator`'s
    /// `ctx.with_module`), so `ctx.module()` need not be the module that
    /// mangled `name` even though the lookup is still correct. A REPL line
    /// declares no statics.
    pub(super) fn static_type(&self, name: &str) -> Option<Type> {
        match self {
            Ctx::Word { statics, .. } => statics.iter().find(|s| s.name == name).map(|s| s.ty),
            Ctx::Line { .. } => None,
        }
    }

    /// R2 (slice 8b): the caller module a scoped-name visibility check runs
    /// against. A bare REPL line denotes module 0.
    pub(super) fn module(&self) -> u32 {
        match self {
            Ctx::Word { module, .. } => *module,
            Ctx::Line { .. } => 0,
        }
    }

    /// R3 (slice 8b): the import closure's per-module data, or `None` on the
    /// REPL path, where the `drop` import-visibility gate does not fire.
    pub(super) fn modules(&self) -> Option<&[ModuleInfo]> {
        match self {
            Ctx::Word { modules, .. } => *modules,
            Ctx::Line { .. } => None,
        }
    }

    /// The enclosing word's name, for recognizing a self-tail-call back-edge
    /// (R15). A bare REPL line has no word to recurse into.
    pub(super) fn word_name(&self) -> Option<&str> {
        match self {
            Ctx::Word { name, .. } => Some(name),
            Ctx::Line { .. } => None,
        }
    }

    pub(super) fn mangled_name(&self) -> Option<&str> {
        match self {
            Ctx::Word { mangled, .. } => Some(mangled),
            Ctx::Line { .. } => None,
        }
    }

    /// The enclosing word's own declared effect, for recognizing which
    /// struct (not just which name) a `drop` override's body is exempt for
    /// (D3's destructure guard). A bare REPL line has no declared effect.
    pub(super) fn effect(&self) -> Option<&StackEffect> {
        match self {
            Ctx::Word { effect, .. } => Some(effect),
            Ctx::Line { .. } => None,
        }
    }

    /// R11: the enclosing word's declared output row, the context a branch join
    /// in tail position materializes its quotation arms against (the merged
    /// slot maps to the output at the same index). A bare REPL line has no
    /// declared row, so a materializing join there stays a located error.
    pub(super) fn declared_outputs(&self) -> Option<&[TypedSlot]> {
        match self {
            Ctx::Word { effect, .. } => Some(&effect.outputs),
            Ctx::Line { .. } => None,
        }
    }

    /// Slice 10c (review fix, Phase 1): whether the enclosing word's own
    /// self-tail call actually lowers to a splice-time back-edge. Gates the
    /// back-edge-only guards (R15) so they fire exactly where lowering
    /// back-edges; see the field doc on `Ctx::Word::self_tail_call`. A bare
    /// REPL line has no word to recurse into, so it is never a back-edge.
    pub(super) fn is_self_tail_call(&self) -> bool {
        match self {
            Ctx::Word { self_tail_call, .. } => *self_tail_call,
            Ctx::Line { .. } => false,
        }
    }
}

impl<'a> Ctx<'a> {
    /// D1 fix (slice 8b, bug 3): rebuild this `Ctx` with `module` swapped to
    /// the module that actually declares the term about to be checked, not
    /// the caller's. `inline_combinator` splices a combinator's body into the
    /// *caller's* `Ctx` so its locals/effect/name still read right in
    /// diagnostics, but a module-scoped visibility gate (D1's drop-import
    /// check, 8a's operator scoping) run against `ctx.module()` while
    /// checking that spliced body must see the combinator's own declaring
    /// module, or a library combinator disposing its own resource gets
    /// attributed to whichever module happened to call it. A no-op on
    /// `Ctx::Line`, which has no module to scope against.
    pub(super) fn with_module(&self, module: u32) -> Ctx<'a> {
        match *self {
            Ctx::Word {
                name,
                mangled,
                effect,
                structs,
                enums,
                statics,
                modules,
                self_tail_call,
                generics,
                ..
            } => Ctx::Word {
                name,
                mangled,
                effect,
                structs,
                enums,
                statics,
                module,
                modules,
                self_tail_call,
                generics,
            },
            Ctx::Line { structs, enums } => Ctx::Line { structs, enums },
        }
    }

    /// P7 slice 3a phase 2 (R2): the live generic instantiator, or `None` on
    /// a path that can never carry a `PolyType::Generic` in the first place
    /// (a bare REPL line, or a session-defined poly word). A grounding arm
    /// (`unify_poly_input`/`apply_subst`) that sees `None` here has reached a
    /// `PolyType::Generic` some path never grounds, and stays the Phase 1
    /// not-yet-groundable error rather than mint through a table that is not
    /// there.
    pub(super) fn generics(&self) -> Option<&'a RefCell<GenericTypes>> {
        match self {
            Ctx::Word { generics, .. } => *generics,
            Ctx::Line { .. } => None,
        }
    }
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
    fn bare_word(name: &str, module: u32) -> WordDef {
        WordDef {
            name: name.to_string(),
            effect: StackEffect::default(),
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            declares_inline: false,
            module,
            span: Span::default(),
            declared_globals: None,
        }
    }
    /// A `Res` owned by `defining`, mangled as `resolve` would in a multi-module
    /// build (`Res__m{defining}`), so `check_drop_import_visibility`'s demangle
    /// is exercised for real.
    fn res_struct(defining: u32, has_drop_overload: bool) -> StructDecl {
        StructDecl {
            name: format!("Res__m{defining}"),
            name_static: "Res",
            fields: vec![("n".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload,
            is_bundle: false,
            module: defining,
        }
    }
    /// Run `check_shuffle`'s `"drop"` arm on a single `Res` operand under a
    /// caller-module `Ctx::Word` built with `modules`.
    fn drop_res(
        structs: &[StructDecl],
        modules: Option<&[ModuleInfo]>,
        caller: u32,
    ) -> Result<Option<Vec<Slot>>, String> {
        let word = bare_word("main", caller);
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = word_ctx(
            &word,
            structs,
            &enums,
            &[],
            modules,
            &CombinatorIndex::new(),
            None,
        );
        let arrays: Vec<ArrayDecl> = Vec::new();
        let mut prov = Provenance::default();
        // The term is written in the caller's own module, so its span says so:
        // a hardcoded `module: 0` contradicts what a real build produces, and
        // the visibility gate reads `span.module`.
        let span = Span {
            line: 2,
            col: 1,
            module: caller,
        };
        let mut stack = vec![Slot::computed(Type::Struct(StructId::from_index(0), "Res"))];
        let scope = Scope::default();
        let live = Liveness {
            last_use: HashMap::new(),
            outer_releasable: HashSet::new(),
        };
        check_shuffle(
            "drop", span, &mut stack, &ctx, &arrays, &mut prov, &scope, &live, 0,
        )
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
    const FILE_RESOURCE: &str = "type: File fd i64 ; : drop ( File -- ) | f | f File> . ;";
    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is (R3),
    /// not by any compiler-known bit. Always the first struct in a source
    /// string that uses it, so every other struct's `StructId` shifts up by
    /// one relative to a spy-free program.
    const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy> . ;\n";
    fn struct_ty(module: &Module, name: &str) -> Type {
        let idx = module
            .structs
            .iter()
            .position(|s| s.name == name)
            .expect("declared struct");
        Type::Struct(StructId::from_index(idx), module.structs[idx].name_static)
    }
    fn infer_src(src: &str, entry: &[Type]) -> Result<Vec<Type>, String> {
        let tokens = lex(src).unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        // `bool` is `Type::Enum(BOOL_ENUM_ID, ..)` (Slice 9): a real REPL
        // session seeds this at index 0 (`Session::new`); this bare-line
        // helper mirrors that so a `bool`-producing comparison resolves.
        let bool_enums = [crate::ast::bool_enum_decl()];
        infer_line(
            &terms,
            entry,
            &HashMap::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &[],
            &bool_enums,
            &HashMap::new(),
            &CombinatorEnv::default(),
        )
        .map(|(stack, _insts, _overloads, _fields, _variant_fields)| stack)
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
        let mut generics = crate::ast::GenericTypes::with_bases(structs.len(), enums.len());
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
            &mut generics,
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
            &mut generics,
        )
        .unwrap();
        assert_eq!(
            arrays.len(),
            1,
            "two files' [i64 8] dedupe to one ArrayId in the shared registry"
        );
    }
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
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
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
            let scope = Scope::default();
            let live = Liveness {
                last_use: HashMap::new(),
                outer_releasable: HashSet::new(),
            };
            let out = check_shuffle(
                name, span, &mut stack, &ctx, &arrays, &mut prov, &scope, &live, 0,
            )
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
    /// R2: `Ctx::Word` carries its word's owning module; `Ctx::Line` denotes 0.
    #[test]
    fn ctx_word_carries_owning_module() {
        let word = bare_word("main", 3);
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            None,
        );
        assert_eq!(ctx.module(), 3);
        assert!(ctx.modules().is_none());
    }
    #[test]
    fn ctx_line_is_module_zero() {
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        assert_eq!(ctx.module(), 0);
        assert!(ctx.modules().is_none());
    }
    #[test]
    fn drop_of_locally_declared_override_is_ok() {
        // caller == defining: the override is the caller's own, always visible.
        let structs = vec![res_struct(0, true)];
        let modules = vec![ModuleInfo::default()];
        assert!(drop_res(&structs, Some(&modules), 0).is_ok());
    }
    #[test]
    fn drop_of_selectively_imported_type_is_ok() {
        let structs = vec![res_struct(0, true)];
        let mut caller = ModuleInfo::default();
        caller.selective.insert("Res".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        assert!(drop_res(&structs, Some(&modules), 1).is_ok());
    }
    #[test]
    fn drop_of_qualified_only_imported_type_is_error() {
        let structs = vec![res_struct(0, true)];
        let mut caller = ModuleInfo::default();
        caller.imports.insert("lib".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        let err = drop_res(&structs, Some(&modules), 1).unwrap_err();
        // R5: the exact located diagnostic, not merely that it fails.
        assert_eq!(
            err,
            "error: cannot `drop` a value of type `lib::Res` in `main` (line 2)\n  disposing it runs a `drop` destructor declared in module `lib`, which this module has not imported by name\n  note: add `Res` to the import (`import: lib | Res | \"...\"`), or dispose it in a module that declares `Res`"
        );
    }
    #[test]
    fn drop_of_transitively_reachable_type_with_no_direct_import_is_error() {
        // Round-2 fix: `Res` is declared by module 0 (`deep`), reached by the
        // caller (module 2, `main`) only through module 1 (`mid`), which
        // `main` never imports directly. The caller's import map has no
        // qualifier mapping to module 0, so the diagnostic must not fabricate
        // one -- naming the struct's own bare name as if it were a module
        // qualifier (the pre-fix behavior) read as a valid but wrong import
        // spelling.
        let structs = vec![res_struct(0, true)];
        let mid = ModuleInfo::default();
        let mut caller = ModuleInfo::default();
        caller.imports.insert("mid".to_string(), 1);
        let modules = vec![ModuleInfo::default(), mid, caller];
        let err = drop_res(&structs, Some(&modules), 2).unwrap_err();
        assert_eq!(
            err,
            "error: cannot `drop` a value of type `Res` in `main` (line 2)\n  disposing it runs a `drop` destructor declared in a module this module never imports directly -- it is only reachable transitively, through another module's import\n  note: import the module that declares `Res` directly, then add `Res` to that import"
        );
    }
    #[test]
    fn drop_of_plain_struct_no_override_is_ungated() {
        // No override: the gate is never reached, the value disposes structurally.
        let structs = vec![res_struct(0, false)];
        let mut caller = ModuleInfo::default();
        caller.imports.insert("lib".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        assert!(drop_res(&structs, Some(&modules), 1).is_ok());
    }
    #[test]
    fn check_shuffle_with_no_modules_is_ungated() {
        // R8's contract: with `modules: None` (the REPL path) an override is
        // never gated -- disposing it is byte-for-byte what it was before 8b.
        let structs = vec![res_struct(0, true)];
        assert!(drop_res(&structs, None, 1).is_ok());
    }
    #[test]
    fn merged_quotations_are_rejected_at_the_join() {
        // Cu2 (R7): two *different* quotations merged at an `if` join are
        // rejected at the join (not at consumption), because `lower_if` would
        // otherwise build a `Phi` over two phantoms. The *same* `Known` id in
        // both arms (one literal bound before the `if`, read in each) is safe:
        // `lower_if`'s `t == e` fast path emits no `Phi`, so it must not error.
        let different = check_src(": main ( -- ) true ~[ [ 1 + ] ] ~[ [ 1 - ] ] if drop ;\n")
            .expect_err("two different quotations at a join should be rejected");
        assert!(
            different.contains("these two branches leave different quotations"),
            "the join guard should fire, got: {different}"
        );
        check_src(": main ( -- ) [ + ] | q | true ~[ q ] ~[ q ] if drop ;\n")
            .expect("the same `Known` id in both arms is safe and must not error");
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
            vec![Type::I64, Type::BOOL]
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
            &HashMap::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &module.structs,
            &module.enums,
            &HashMap::new(),
            &CombinatorEnv::default(),
        )
        .unwrap_err();
        assert!(
            err.contains("`File` is linear because it defines `drop`"),
            "unexpected message: {err}"
        );
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
        scope.bind("s", Slot::computed(Type::BOOL), true, prov);
        let leaked = scope.leave(depth).expect("an unconsumed linear local");
        assert_eq!((leaked.0.as_str(), leaked.1), ("s", Type::BOOL));
        assert_eq!(leaked.2, MoveState::Live);
    }
    #[test]
    fn fill_diagnostics_unchanged_after_site_parameterization() {
        // D2: `fill`'s rendered diagnostics must stay byte-identical to
        // before the shared gate existed. Assert the full strings, not
        // `contains("fill")`.
        let linear_err =
            check_src(&format!("{SPY_DEF}: w ( -- ) 0 Spy 3 fill drop ;")).unwrap_err();
        assert_eq!(
            linear_err,
            "error: linear array elements are not supported yet in `w` (line 3)\n  `fill` would replicate a `Spy` across every slot, but `Spy` is linear and has no `Copy` instance\n  note: declared ( -- )"
        );
        let ref_err = check_src(": w ( &i64 -- ) 3 fill drop ;").unwrap_err();
        assert_eq!(
            ref_err,
            "error: a reference cannot be stored in `w` (line 1)\n  the element `fill` would store has type `&i64`\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow"
        );
    }
    #[test]
    fn operator_dispatch_resolves_the_exact_row_type() {
        // Guards that resolution yields the right stack-effect type: a
        // homogeneous op over `u8` yields `u8`, a comparison yields the
        // 32-bit flag,
        // `.` yields nothing. Note these all resolve identically through the
        // numeric fallback too, so this does *not* prove the table pass is
        // used; `check_not_on_literal_count_is_not_a_literal_for_fill` is the
        // guard that the exact-match table row actually drives dispatch.
        assert_eq!(
            infer_src("5 >u8 3 >u8 +", &[]).unwrap(),
            vec![Type::from_name("u8").unwrap()]
        );
        // Slice 10c: the comparison *primitive*, which is what carries the
        // per-numeric-type rows now; `<` is a `lib/` word over it and resolves
        // through the word environment this bare-line helper does not build.
        assert_eq!(infer_src("5 >u8 3 >u8 u<", &[]).unwrap(), vec![Type::U32]);
        assert_eq!(infer_src("5 .", &[]).unwrap(), Vec::<Type>::new());
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
    fn intern_ref_type_dedups_per_referent_and_mutability() {
        let mut refs = Vec::new();
        let a = intern_ref_type(&mut refs, Type::I64, true);
        let b = intern_ref_type(&mut refs, Type::I64, true);
        let c = intern_ref_type(&mut refs, Type::BOOL, true);
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
    /// R1/R2 (P7 slice 2 review fix): post-mangle, module-private scoping
    /// lives in the name itself (`resolve::mangle` bakes `COUNT` declared in
    /// module 1 as `COUNT__m1`, unconditionally, before `check::check` ever
    /// runs), so lookup matches on the mangled name alone. A combinator
    /// splice checks the caller's own quotation arguments under the callee's
    /// module (`inline_combinator`'s `ctx.with_module`), so `ctx.module()`
    /// can legitimately differ from the module that mangled `name` --
    /// filtering on `s.module == ctx.module()` (the prior behavior) rejected
    /// exactly that case, making every static-touching program that called an
    /// imported combinator fail to check.
    #[test]
    fn static_lookup_matches_by_mangled_name_under_a_splice_rescoped_ctx() {
        let statics = vec![StaticDecl {
            name: "COUNT__m1".to_string(),
            ty: Type::I64,
            init: crate::ast::StaticInit::Zero,
            module: 1,
            span: Span::default(),
        }];
        let enums: Vec<EnumDecl> = Vec::new();
        let structs: Vec<StructDecl> = Vec::new();
        // `main` declared `COUNT` in module 1, but is checked here under
        // module 0 -- exactly what `ctx.with_module(comb.word.module)` does
        // while splicing a module-0 combinator's body around `main`'s own
        // `~[ ... ]` argument.
        let owner = bare_word("main", 0);
        let ctx = word_ctx(
            &owner,
            &structs,
            &enums,
            &statics,
            None,
            &CombinatorIndex::new(),
            None,
        );
        assert_eq!(
            ctx.static_type("COUNT__m1"),
            Some(Type::I64),
            "a mangled name resolves even under a splice-rescoped ctx"
        );
        assert_eq!(
            ctx.static_type("COUNT__m0"),
            None,
            "a different module's mangled name is simply absent, no filter needed"
        );
    }
    #[test]
    fn scope_bind_keeps_the_reborrow_and_the_owned_root() {
        // The fix this replaces: a bound reference used to release the place
        // it was reborrowed from, which silently dropped protection for a
        // reborrow of a reference *parameter* (no `owned_root` either, so
        // nothing was left to suspend). Binding must be a no-op on
        // provenance now: what ends a suspension is last-use liveness (6f),
        // not the bind. `Scope::bind` stores `slot.deriv` verbatim (no
        // `Provenance` transform in between), so this asserts that directly.
        let mut prov = Provenance::default();
        let mut scope = Scope::default();
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
        let fresh = prov.borrow("v", true, false, span);
        let reborrow = prov.reborrow("r", Some(fresh), true, span);
        let projected = prov.project(Some(reborrow)).expect("a projection");
        assert!(prov.deriv(projected).reborrow, "still suspends `r`");
        assert!(prov.deriv(projected).projected, "R7's note is apt here");
        assert_eq!(prov.deriv(projected).owned_root.as_deref(), Some("v"));

        scope.bind(
            "e",
            Slot::derived(Type::I64, Some(projected)),
            false,
            &mut prov,
        );
        let bound = scope
            .local("e")
            .and_then(|b| b.deriv)
            .expect("a bound deriv");
        assert!(
            prov.deriv(bound).reborrow,
            "`r` stays suspended after binding"
        );
        assert_eq!(
            prov.deriv(bound).owned_root.as_deref(),
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
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
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
    /// Slice 10a (R11): the bottom-aligned index map, exercised directly on
    /// `back_edge_declared_shape` (a monomorphic effect grounds with no
    /// `Subst`, so no parser is needed). Covers the `times`-shape (empty),
    /// the `while`-shape (1<->1), an asymmetric shape (deepest output <-
    /// deepest carried input), a longer output list (overflow -> `None`), and
    /// a type mismatch at the aligned position (-> `None`).
    #[test]
    fn back_edge_index_map_is_bottom_aligned() {
        let quot = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
        fn imap(inputs: Vec<Type>, outputs: Vec<Type>) -> Vec<Option<usize>> {
            use crate::ast::{StackEffect, TypedSlot};
            let w = WordDef {
                name: "w".to_string(),
                effect: StackEffect {
                    inputs: inputs
                        .into_iter()
                        .map(|ty| TypedSlot { name: None, ty })
                        .collect(),
                    outputs: outputs
                        .into_iter()
                        .map(|ty| TypedSlot { name: None, ty })
                        .collect(),
                },
                body: WordBody::Terms { terms: Vec::new() },
                poly: None,
                declares_inline: false,
                module: 0,
                span: Span::default(),
                declared_globals: None,
            };
            let ctx = Ctx::Line {
                structs: &[],
                enums: &[],
            };
            let mut arrays = Vec::new();
            let mut refs = Vec::new();
            back_edge_declared_shape(&w, None, "w", Span::default(), &ctx, &mut arrays, &mut refs)
                .unwrap()
                .2
        }
        // `times`-shape: zero fixed outputs -> empty map.
        assert_eq!(
            imap(vec![Type::I64, quot], Vec::new()),
            Vec::<Option<usize>>::new()
        );
        // `while`-shape: one carried in, one out, same type.
        assert_eq!(imap(vec![Type::I64, quot], vec![Type::I64]), vec![Some(0)]);
        // Asymmetric: two carried, one out -> output 0 <- deepest carried.
        assert_eq!(
            imap(vec![Type::I64, Type::I64, quot], vec![Type::I64]),
            vec![Some(0)]
        );
        // More outputs than carried inputs: the overflowing output is `None`.
        assert_eq!(
            imap(vec![Type::I64, quot], vec![Type::I64, Type::I64]),
            vec![Some(0), None]
        );
        // Type differs at the aligned position -> `None`.
        assert_eq!(imap(vec![Type::I64, quot], vec![Type::Str]), vec![None]);
    }
    /// Slice 10a (R11): the recon-4 `my-times` -- which *consumes* its counters
    /// -- used to fail with a spurious `if` branch-depth mismatch, because the
    /// back-edge produced the non-quotation inputs instead of the (empty,
    /// row-only) ground declared outputs. It now checks.
    #[test]
    fn back_edge_produces_ground_declared_outputs() {
        let src = ": my-times inline ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
                   | f | | to | | from |\n\
                   from to < ~[\n\
                   from f call\n\
                   from 1 + to f my-times\n\
                   ] ~[\n\
                   ] if ;\n\
                   : main ( -- ) 0 0 5 ~[ + ] my-times . ;\n";
        check_src(src)
            .expect("my-times checks: the back-edge produces the ground declared outputs");
    }

    /// U1 (R1): a `Copy` binding as `releasable_into` reads it -- only `name`
    /// is consulted, so the provenance side channels stay empty.
    fn binding(name: &str) -> Binding {
        Binding {
            name: name.to_string(),
            ty: Type::I64,
            aliases: None,
            deriv: None,
            quot: None,
            surviving: None,
        }
    }

    /// U1 (R1): the grant handed into a nested block of a `back_edge = true`
    /// body, over a `Liveness` really built by `scan`. `a` is mentioned at
    /// term 0, *before* the block, so the pre-R1 rule ("not referenced in the
    /// remaining siblings") grants it -- and a back-edge body wraps around to
    /// term 0, so the next iteration reads it again. `unused` is the control
    /// for the other half: an ancestor name the body never mentions at all is
    /// still granted, through `dead`'s `None` arm, so the tightening is shown
    /// not to withhold from every ancestor indiscriminately.
    #[test]
    fn releasable_into_withholds_a_name_used_in_a_back_edge_body() {
        let tokens = lex("a drop true ~[ 1 . ] ~[ ] if").expect("lexing should succeed");
        let terms = match crate::parser::parse_line(&tokens).expect("parsing should succeed") {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        let at = terms
            .iter()
            .position(|t| matches!(&t.kind, TermKind::Quotation(..)))
            .expect("the line's block is a branch-arm quotation literal");
        let rest = &terms[at + 1..];
        let outer: HashSet<String> = ["a", "unused"].iter().map(|n| n.to_string()).collect();
        let live = Liveness::scan(&terms, &outer, true);
        // Both names are ancestor-bound relative to this invocation
        // (`base_depth` past the last of them), so both take R1's new branch.
        let scope = Scope {
            bound: vec![binding("a"), binding("unused")],
            moves: Moves::default(),
        };
        let granted = releasable_into(&scope, 2, &outer, rest, &live, at);
        assert!(
            !references(rest, "a"),
            "the pre-R1 rule would grant `a`: nothing after the block mentions it"
        );
        assert!(
            !granted.contains("a"),
            "`a` is used earlier in a back-edge body, so it is live again on the \
             next iteration and must not be granted into the block: {granted:?}"
        );
        assert!(
            granted.contains("unused"),
            "an ancestor name the body never mentions is still granted: {granted:?}"
        );
    }
}
