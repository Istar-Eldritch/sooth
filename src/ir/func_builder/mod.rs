//! `FuncBuilder`: the compile-time virtual-stack lowering machine. Holds the
//! per-function block/value/loop bookkeeping and the term-lowering methods that
//! turn a word body into SSA-shaped blocks. Depends on `types`, `layout`, and
//! `destructors`; the lowering driver (parent module) drives it.

mod calls;
mod quotation;
mod word_families;

use super::*;

/// R10: the `IrType` a word returns — its one output, or the synthesized
/// bundle struct for two or more. The single derivation both the lowering env's
/// `ret_ty` and `lower_word`'s own `ret` go through, so a caller reading the
/// env and the callee it calls can never disagree about the return shape.
/// Falls back to the first output where no bundle was interned (the REPL's
/// registries, D2): that path keeps its pre-slice lowering rather than
/// half-entering the bundle ABI.
pub(super) fn word_ret_ty(outputs: &[TypedSlot], structs: &Structs) -> Option<IrType> {
    match bundle_of(outputs, structs) {
        Some(id) => Some(IrType::Struct(id)),
        None => outputs.first().map(|slot| ir_type_of(slot.ty)),
    }
}

/// R10: the bundle a word with these declared outputs returns through.
pub(super) fn bundle_of(outputs: &[TypedSlot], structs: &Structs) -> Option<StructId> {
    if outputs.len() < 2 {
        return None;
    }
    let tys: Vec<Type> = outputs.iter().map(|slot| slot.ty).collect();
    structs.bundle_for(&tys)
}

fn is_aggregate(ty: IrType, enums: &Enums) -> bool {
    match ty {
        // Slice 9 (R1): a zero-payload enum's value is a bare scalar, not an
        // address-having aggregate -- it takes the header-`Phi` path below
        // like any other scalar, never the stable-slot staging.
        IrType::Enum(id) => !enums.layouts[id.index()].is_scalar,
        IrType::Struct(_) | IrType::Array(_) | IrType::Quotation(_) => true,
        _ => false,
    }
}

/// 7b/R16: the lowering twin of the checker's `capture_names`/`call_local`
/// (which live in `check.rs`): every free name a quotation body reads, at any
/// depth, that is not bound *inside* the body. A `&`/`&!` sigil is stripped so
/// a borrow of a local resolves to the local's name. `materialize_quot_value`
/// intersects this against `self.locals` to find the captured value.
fn free_locals_into(terms: &[Term], shadowed: &mut HashSet<String>, out: &mut HashSet<String>) {
    for term in terms {
        match &term.kind {
            TermKind::Bind(names) => shadowed.extend(names.iter().cloned()),
            TermKind::Call(name) => {
                let local = name
                    .strip_prefix("&!")
                    .or_else(|| name.strip_prefix('&'))
                    .unwrap_or(name);
                if !shadowed.contains(local) {
                    out.insert(local.to_string());
                }
            }
            TermKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                free_locals_into(then_branch, &mut shadowed.clone(), out);
                free_locals_into(else_branch, &mut shadowed.clone(), out);
            }
            TermKind::Quotation(inner) => free_locals_into(inner, &mut shadowed.clone(), out),
            _ => {}
        }
    }
}

/// R10: whether a combinator body has a tail-position call to itself, the
/// lowering twin of the checker's `tail_position_calls`/`has_self_tail_call`
/// (which take a `&WordBody` this splice site does not hold). The syntactic
/// tail rule is the same: the final term of the body, or the final term of
/// either arm of a terminal `if`, recursively.
///
/// The *conclusion* drawn from it no longer is. Since slice 8a,
/// `has_self_tail_call` additionally refuses a builtin-named word, because
/// the same name in tail position may resolve to the builtin rather than to
/// the enclosing word; this function still decides on the bare name. The two
/// only agree today because a builtin-named combinator cannot exist: a
/// combinator takes a quotation operand, and `check_operator`'s R11 guard
/// rejects a quotation operand to any builtin name before the env combinator
/// lookup runs. Nothing pins that, so if the R11 guard ever narrows, this
/// needs the same refusal or check and lowering will disagree about whether a
/// splice is a loop.
fn body_tail_calls_self(body: &[Term], name: &str) -> bool {
    match body.last().map(|t| &t.kind) {
        Some(TermKind::Call(n)) => n == name,
        Some(TermKind::If {
            then_branch,
            else_branch,
            ..
        }) => body_tail_calls_self(then_branch, name) || body_tail_calls_self(else_branch, name),
        _ => false,
    }
}

/// Per-carried-slot loop metadata (R2), in full carried-slot order. A scalar
/// keeps its header phi; an aggregate carries no header phi but a stable
/// entry-hoisted slot (the pointer the body reads every iteration) plus a
/// staging temp and blit `size` for the back-edge read-before-write copy (R4).
pub(super) enum CarriedSlot {
    Scalar {
        phi: Value,
    },
    Aggregate {
        stable: Value,
        temp: Value,
        size: u32,
    },
}

/// R15/D4: the five fields `save_loop_state`/`restore_loop_state` snapshot
/// around a spliced nested loop-opening construct. See `save_loop_state`'s
/// doc for why this must be all five (`alloca_home` is the 6d addition).
struct LoopStateSnapshot {
    header: Option<BlockId>,
    entry_block: Option<BlockId>,
    alloca_home: Option<BlockId>,
    carried_slots: Vec<CarriedSlot>,
    back_edges: Vec<(BlockId, Vec<Value>)>,
}

/// Slice 7a (R9): a quotation literal a materialization boundary turned into a
/// runtime `(code, env)` value. Each one mints exactly one standalone `IrFunc`
/// (named `symbol`, its declared `effect` its signature, `body` its terms), the
/// callee an `Instr::FuncAddr` in the value's `code` slot names. Collected per
/// `FuncBuilder`, drained by the lowering driver's worklist so a materialized
/// body that itself materializes a nested quotation is lowered too.
#[derive(Clone)]
pub(super) struct MaterializedQuot {
    pub(super) symbol: String,
    pub(super) effect: &'static QuotEffect,
    pub(super) body: Vec<Term>,
    /// 7b/R16: the captured locals this closure snapshots into its `env`, in a
    /// stable order (sorted by name) shared by the build side
    /// (`materialize_quot_value`) and the read side (`lower_word_parts`). Empty
    /// for a non-capturing literal (the 7a null-env shape); one entry stored
    /// inline in the `env` word; two or more packed into a stack bundle the
    /// `env` word points at.
    pub(super) captures: Vec<EnvCapture>,
}

/// 7b/R16: one of a materialized closure's captured locals, as the lowered
/// body needs to bind it. `referent` is `Some` when the capture is a reference
/// (its `ty` is `Ptr`), carrying the referent shape a projection through it
/// needs; `None` marks a scalar snapshot, whose env word is reinterpreted back
/// to `ty` at the body's entry.
#[derive(Clone)]
pub(super) struct EnvCapture {
    pub(super) name: String,
    pub(super) ty: IrType,
    pub(super) referent: Option<IrType>,
}

/// 7b/R17: the env-parameter plan for a lowered body. A user word (or REPL
/// line) has none; a materialized quotation body always takes one trailing
/// `Ptr` env parameter, uniform so `lower_indirect_call` can pass the env slot
/// without knowing the callee, and binds a captured local when it has one.
pub(super) enum EnvPlan {
    None,
    Env(Vec<EnvCapture>),
}

pub(super) struct FuncBuilder<'a> {
    env: &'a HashMap<String, Arity>,
    resolve: Resolver<'a>,
    pub(super) structs: &'a Structs,
    pub(super) enums: &'a Enums,
    pub(super) arrays: &'a Arrays,
    cells: &'a Cells,
    /// The per-`RefId` referent `IrType`: needed to resolve a
    /// reference-mode clause scrutinee's `EnumId` when the referent itself is
    /// an enum.
    refs: &'a Refs,
    /// R14: the per-call-site instantiation table. A `Call` term whose span
    /// keys an entry here is a call to a polymorphic word and resolves to that
    /// entry's mangled symbol and per-θ output shape, not the name-keyed
    /// `env`/`resolve`. Empty on the REPL/destructor/test paths.
    pub(super) instantiations: &'a HashMap<Span, CallInst>,
    /// Slice 8a phase 2 (R7): the call sites the checker resolved to a user
    /// overload of a builtin-named word, span -> resolved callee name.
    /// Consulted before the name-directed builtin dispatch in `lower_call`, so
    /// a recorded `Vec2 +` site emits an `Instr::Call` to the user word
    /// instead of `Bin(Add)`. Empty on every corpus/REPL/test path (the
    /// checker records nothing there), so their lowering is byte-for-byte.
    pub(super) builtin_overloads: &'a HashMap<Span, String>,
    /// R14: the fixed input arity of each polymorphic word, name-keyed. How
    /// many args a polymorphic call pops (the `CallInst` carries the output
    /// shape, but the input count is name-constant across θ, so it lives here).
    pub(super) poly_arities: &'a HashMap<String, usize>,
    /// R19/R20: monomorphic quotation-taking words (combinators), name-keyed
    /// to their bodies. A `Call` of such a name is spliced in place rather
    /// than lowered to an `Instr::Call`, mirroring the checker's inliner
    /// (R18): the callee mints no `IrFunc` (it is absent from `funcs`/`env`),
    /// so its only reachable form is the splice. Empty on the REPL/destructor/
    /// test paths.
    pub(super) combinators: &'a HashMap<String, Vec<Term>>,
    /// Name of the word currently being lowered, used by the tail-call ->
    /// back-edge transform (R7) to recognize a self-call.
    pub(super) cur_word_name: String,
    /// Slice 7a (R11): the `IrType` of each declared output, in order. A branch
    /// join in tail position maps its merged slot `i` to `cur_outputs[i]`; a
    /// differing phantom-quotation pair whose target is `IrType::Quotation` is
    /// materialized into a `(code, env)` value in each arm and `Phi`-joined.
    /// Empty on the REPL-line path (a line has no declared row, so the checker
    /// never lets a materializing join reach here).
    pub(super) cur_outputs: Vec<IrType>,
    /// R10: the self-tail combinator whose body is currently being spliced
    /// (its mangled name and the length of the loop-carried state row). A
    /// tail-position call to that same name inside the splice is the loop
    /// back-edge, not a re-splice: it pops its loop-invariant quotation
    /// argument(s) (everything above the carried row, resolved statically via
    /// the local binding, not carried in a phi) and jumps to the header.
    /// Saved and restored around the splice so loops compose.
    cur_combinator: Option<(String, usize)>,
    /// The loop header block (R6), `Some` iff this word is self-tail-recursive
    /// and is being lowered as a loop. Tail self-calls back-edge to it (R7).
    pub(super) header: Option<BlockId>,
    /// Per loop-carried slot metadata (input arity many), in full slot order
    /// (R2). A scalar slot carries its header phi; an aggregate slot carries
    /// its entry-hoisted stable slot, staging temp, and blit size instead of a
    /// phi. `finalize_loop` dispatches on this per slot.
    pub(super) carried_slots: Vec<CarriedSlot>,
    /// Collected back-edges (R8): each is `(pred block, one arg value per
    /// carried slot)`. Finalized into the header phis after the body lowers,
    /// since the operands are only known on the back-edges.
    pub(super) back_edges: Vec<(BlockId, Vec<Value>)>,
    /// The loop's entry block (the block that ran before `begin_loop`'s jump
    /// to the header), `Some` alongside `header`. An `Alloc` emitted while
    /// looping is hoisted here (R6 constant-stack corollary): QBE's `alloc*`
    /// bumps the frame pointer on every execution and never reclaims it
    /// within a function, so an aggregate constructed on the back-edge (e.g.
    /// a clause's variant re-scrutinee) would otherwise grow the frame by one
    /// slot per iteration and blow the stack well before the loop's constant-
    /// stack guarantee is exercised. Hoisting reserves one fixed slot per
    /// static alloc site, reused (overwritten) every iteration instead. This is
    /// safe even when a loop constructs an inline aggregate into a same-site
    /// slot before reading the prior iteration's value, because a carried
    /// aggregate is snapshotted onto its own stable slot on the back-edge (R4),
    /// so hoisting no longer depends on the body's read-before-overwrite order.
    ///
    /// 6d/D2: `entry_block` is the per-loop *preheader* only. It changes with
    /// every nested loop (the block current when that loop opens) and is where
    /// a carried aggregate's seeding `Blit` lands, so the blit re-runs once per
    /// entry to *this* loop. The alloca-home role it used to double as (where
    /// hoisted `Alloc`s land) moved to `alloca_home`.
    pub(super) entry_block: Option<BlockId>,
    /// 6d/D2: the invariant per-function alloca home, where `push_alloc`
    /// hoists every `Alloc` so QBE's frame-bumping `alloc*` runs once per call
    /// rather than once per iteration. Set on the *outermost* loop only (the
    /// function's true entry, reached once per call) and kept across nested
    /// loops, so an inner-loop `Alloc` still hoists to a block reached once per
    /// call. `push_alloc` used to route into `entry_block` and rely silently
    /// on the accident that, for a *top-level* loop, the preheader is that
    /// once-per-call block; the moment a loop nests, the preheader is reached
    /// once per outer iteration and the frame grows. Tracking the alloca home
    /// separately from the preheader is the whole 6d fix.
    pub(super) alloca_home: Option<BlockId>,
    /// Whether the current block has already been sealed (by a back-edge Jmp or
    /// another terminator), so no fall-through Ret/Jmp should follow.
    pub(super) terminated: bool,
    pub(super) blocks: Vec<Block>,
    cur_id: BlockId,
    pub(super) cur_instrs: Vec<Instr>,
    next_value: u32,
    next_block: u32,
    pub(super) stack: Vec<Value>,
    /// The names in scope, innermost-last (R2, R10): leaving a block truncates
    /// this to its length at block entry. A bound value is SSA and outlives the
    /// name, so teardown frees nothing.
    pub(super) locals: Vec<(String, Value)>,
    pub(super) value_types: Vec<IrType>,
    /// Compile-time integer value of each `Const`-defined `Value`, for the
    /// `fill` count (M1: the count is a checker-guaranteed literal) and the
    /// element/array-shape lookup. A shuffle reuses a value id, so a duped
    /// literal keeps its recorded value.
    pub(super) const_vals: HashMap<Value, i64>,
    /// The referent `IrType` of every reference-typed `Value`. A
    /// reference lowers to the opaque `IrType::Ptr`, which deliberately says
    /// nothing about what it points at, so the shape a projection or an access
    /// needs — a field offset, an element stride, an aggregate's blit size —
    /// is carried here instead. Seeded from a word's declared reference
    /// parameters and extended by each projection.
    pub(super) ref_inner: HashMap<Value, IrType>,
    /// R12: the quotation-literal body table, indexed by `QuotId`. A quotation
    /// literal lowers to a phantom `Value` that defines no `Instr`; its body is
    /// interned here and spliced in place at `call`/`times` (D5 fusion), never
    /// emitted as a runtime code value.
    pub(super) quot_defs: Vec<Vec<Term>>,
    /// R12: the phantom quotation `Value` -> its `QuotId`. A shuffle/bind moves
    /// the phantom verbatim (`self.locals`/`self.stack` carry `Value` ids), so
    /// no `Binding` analogue is needed here (D2); `call`/`times` resolve the
    /// body through this map.
    pub(super) quot_bodies: HashMap<Value, QuotId>,
    /// R18/R21: a monotonic per-function suffix counter, mirroring the
    /// checker's, so a combinator body spliced here is alpha-renamed exactly as
    /// it was for checking. Without it a passed-down literal's captured name
    /// would rebind to an inner combinator's same-named local (dynamic, not
    /// lexical, capture).
    inline_uid: u32,
    /// Slice 7a (R9): the quotation literals this func materialized, in mint
    /// order. Drained by the lowering driver, which lowers each into its own
    /// `IrFunc`. Deduped by `symbol` at mint (one literal materialized twice in
    /// one func shares one callee).
    pub(super) materialized: Vec<MaterializedQuot>,
}

impl<'a> FuncBuilder<'a> {
    pub(super) fn new(
        env: &'a HashMap<String, Arity>,
        resolve: Resolver<'a>,
        regs: Registries<'a>,
        cur_word_name: String,
    ) -> Self {
        let Registries {
            structs,
            enums,
            arrays,
            cells,
            refs,
        } = regs;
        FuncBuilder {
            env,
            resolve,
            structs,
            enums,
            arrays,
            cells,
            refs,
            instantiations: empty_instantiations(),
            builtin_overloads: empty_builtin_overloads(),
            poly_arities: empty_poly_arities(),
            combinators: empty_combinators(),
            cur_word_name,
            cur_outputs: Vec::new(),
            cur_combinator: None,
            header: None,
            carried_slots: Vec::new(),
            back_edges: Vec::new(),
            entry_block: None,
            alloca_home: None,
            terminated: false,
            blocks: Vec::new(),
            cur_id: BlockId(0),
            cur_instrs: Vec::new(),
            next_value: 0,
            next_block: 1, // block 0 is the entry, already current
            stack: Vec::new(),
            locals: Vec::new(),
            value_types: Vec::new(),
            const_vals: HashMap::new(),
            ref_inner: HashMap::new(),
            quot_defs: Vec::new(),
            quot_bodies: HashMap::new(),
            inline_uid: 0,
            materialized: Vec::new(),
        }
    }

    pub(super) fn fresh_value(&mut self, ty: IrType) -> Value {
        let v = Value(self.next_value);
        self.next_value += 1;
        self.value_types.push(ty);
        v
    }

    pub(super) fn value_type(&self, v: Value) -> IrType {
        self.value_types[v.0 as usize]
    }

    fn fresh_block(&mut self) -> BlockId {
        let b = BlockId(self.next_block);
        self.next_block += 1;
        b
    }

    pub(super) fn push_instr(&mut self, instr: Instr) {
        self.cur_instrs.push(instr);
    }

    /// Hoist an `Alloc` into the invariant alloca home while looping
    /// (`alloca_home` is `Some`); otherwise emit it into the current block (the
    /// no-loop path). It appends whatever `Instr` it is given, not only an
    /// `Alloc`. 6d/D2: this used to route into `entry_block` (the preheader),
    /// which is reached once per call only for a top-level loop; the alloca
    /// home is the function's true entry, reached once per call at any nesting
    /// depth. See `alloca_home`'s doc for why. The carried-aggregate seeding
    /// `Blit` no longer rides this path (R3): it must land in the preheader,
    /// not the alloca home, so it re-seeds once per loop entry.
    pub(super) fn push_alloc(&mut self, instr: Instr) {
        match self.alloca_home {
            Some(home) => {
                let block = self
                    .blocks
                    .iter_mut()
                    .find(|b| b.id == home)
                    .expect("alloca home block");
                block.instrs.push(instr);
            }
            None => self.push_instr(instr),
        }
    }

    /// R15/D4: save the five fields that together mean "a loop is open"
    /// (`header`, `entry_block`, `alloca_home`, `carried_slots`, `back_edges`)
    /// before splicing a nested loop-opening construct (a `times` term or a
    /// self-tail combinator), pairing with `restore_loop_state` after.
    /// `finalize_loop` clears only `carried_slots`/`back_edges`, never
    /// `header`/`entry_block`, so without this save/restore a later `Alloc`
    /// (or a second sequential loop) would wrongly hoist into the spliced
    /// loop's now-dead entry block. `alloca_home` joins the set (6d/D4): the
    /// outermost loop sets it and it must be cleared again after the splice so
    /// a second sequential top-level loop reseats it to its own entry. One
    /// shared helper for both mid-body call sites means the saved set cannot
    /// drift between them.
    fn save_loop_state(&mut self) -> LoopStateSnapshot {
        LoopStateSnapshot {
            header: self.header,
            entry_block: self.entry_block,
            alloca_home: self.alloca_home,
            carried_slots: mem::take(&mut self.carried_slots),
            back_edges: mem::take(&mut self.back_edges),
        }
    }

    /// The inverse of `save_loop_state`: restore the caller's pre-splice loop
    /// state from its snapshot.
    fn restore_loop_state(&mut self, snapshot: LoopStateSnapshot) {
        self.header = snapshot.header;
        self.entry_block = snapshot.entry_block;
        self.alloca_home = snapshot.alloca_home;
        self.carried_slots = snapshot.carried_slots;
        self.back_edges = snapshot.back_edges;
    }

    /// Seal the current block with `term` and append it to the function.
    pub(super) fn seal_block(&mut self, term: Terminator) {
        let instrs = mem::take(&mut self.cur_instrs);
        self.blocks.push(Block {
            id: self.cur_id,
            instrs,
            term,
        });
    }

    /// Begin a fresh (empty) block; `cur_instrs` is already empty after a seal.
    fn start_block(&mut self, id: BlockId) {
        self.cur_id = id;
    }

    /// Slice 7a (R11): re-open an already-sealed block for appending, so a
    /// branch join can add its arm's quotation materialization *after* both
    /// arms are lowered (the differing-pair decision needs both stacks). Pulls
    /// the block out of `blocks`, restores it as the current block, and returns
    /// its original position and terminator for `reseal_block_at`.
    fn reopen_block(&mut self, id: BlockId) -> (usize, Terminator) {
        let pos = self
            .blocks
            .iter()
            .position(|b| b.id == id)
            .expect("reopen: a sealed block");
        let block = self.blocks.remove(pos);
        self.cur_id = block.id;
        self.cur_instrs = block.instrs;
        (pos, block.term)
    }

    /// The inverse of `reopen_block`: re-seal the current block with `term` at
    /// its original position, so block order (entry first) is unchanged.
    fn reseal_block_at(&mut self, pos: usize, term: Terminator) {
        let instrs = mem::take(&mut self.cur_instrs);
        self.blocks.insert(
            pos,
            Block {
                id: self.cur_id,
                instrs,
                term,
            },
        );
    }

    /// R6/R1-R3: open the loop shape. The current (entry) block binds `params`,
    /// jumps to a fresh header, and the header carries one phi per *scalar*
    /// carried slot, each seeded with the entry arm `(entry, param)`. Returns
    /// the values the body reads instead of the raw params (a scalar's phi
    /// output, an aggregate's stable-slot pointer). An input arity of 0 yields
    /// a header with zero phis (just a back-edge target), handled without
    /// special-casing.
    ///
    /// When `stage_aggregates` is on (the user self-tail-call loop, R1a), each
    /// aggregate-typed carried slot instead gets an entry-hoisted stable slot
    /// (no header phi, R2), an entry-arm init blit copying the incoming param
    /// into it (R3), and a staging temp for the back-edge read-before-write
    /// copy (R4). When it is off (the two fused destructor synthesizers), every
    /// slot takes the scalar path, keeping their lowering byte-for-byte.
    ///
    /// A base case that returns the carried aggregate returns a pointer into
    /// this frame's stable slot; that is safe only because an aggregate return
    /// lowers to `ret %ptr` under a `:S`/`:E`/`:A` return type and QBE copies
    /// the aggregate out by value at the boundary, as the by-value
    /// aggregate-return ABI already relies on.
    pub(super) fn begin_loop(&mut self, params: &[Value], stage_aggregates: bool) -> Vec<Value> {
        let entry = self.cur_id;
        let header = self.fresh_block();
        self.seal_block(Terminator::Jmp(header));
        self.start_block(header);
        self.header = Some(header);
        self.entry_block = Some(entry);
        // 6d/R2: the *outermost* loop fixes the alloca home to its entry (the
        // function's true entry, reached once per call, since no block forks
        // before the first loop). A nested loop sees it already set and keeps
        // the outer home, so an inner-loop `Alloc` still hoists to a
        // once-per-call block instead of this per-outer-iteration preheader.
        if self.alloca_home.is_none() {
            self.alloca_home = Some(entry);
        }
        let mut outs = Vec::with_capacity(params.len());
        for &p in params {
            let ty = self.value_type(p);
            if stage_aggregates && is_aggregate(ty, self.enums) {
                // R1: one stable slot (the pointer the body reads) and one
                // staging temp per aggregate slot; both route through
                // `push_alloc` into the invariant alloca home.
                let size = self.value_size(ty);
                let stable = self.alloc_aggregate(ty);
                let temp = self.alloc_aggregate(ty);
                // R3: seed the stable slot with the incoming param once per
                // entry to *this* loop, so iteration 1 reads an initialised
                // value and a re-entered inner loop re-seeds per outer
                // iteration. The seeding `Blit` goes into this loop's
                // preheader (`entry`) directly, *not* through `push_alloc`
                // (which would hoist it to the alloca home and seed it only
                // once per call, the slice-3 aliasing bug). A zero-size
                // aggregate has no bytes to copy.
                if size > 0 {
                    let block = self
                        .blocks
                        .iter_mut()
                        .find(|b| b.id == entry)
                        .expect("preheader block");
                    block.instrs.push(Instr::Blit(p, stable, size));
                }
                self.carried_slots
                    .push(CarriedSlot::Aggregate { stable, temp, size });
                outs.push(stable);
            } else {
                let out = self.fresh_value(ty);
                self.push_instr(Instr::Phi(out, vec![(entry, p)]));
                self.carried_slots.push(CarriedSlot::Scalar { phi: out });
                outs.push(out);
            }
        }
        outs
    }

    /// R8/R4: after the body lowers, finalize the loop. A scalar slot gets each
    /// collected back-edge's operand appended to its header phi. An aggregate
    /// slot instead gets a read-before-write staging blit pair appended to each
    /// back-edge's predecessor block: a forwarded-in-place arg (exactly its own
    /// stable slot) emits nothing, every other arg is snapshotted into its temp
    /// (read phase) before being stored into its stable slot (write phase), so
    /// an arg that reads a stable slot (a swap) or points into one (an interior
    /// `field_value` pointer) is copied out before any store lands, with no
    /// aliasing analysis. The scalar phi back-patch mutates the header while the
    /// staging blits append to predecessor blocks, so the two run as separate
    /// passes rather than under one borrow.
    pub(super) fn finalize_loop(&mut self) {
        let header = self.header.expect("finalize_loop: loop mode");
        let slots = mem::take(&mut self.carried_slots);
        let back_edges = mem::take(&mut self.back_edges);
        // Pass 1: scalar phi back-patch, header block only.
        let block = self
            .blocks
            .iter_mut()
            .find(|b| b.id == header)
            .expect("header block");
        for instr in &mut block.instrs {
            if let Instr::Phi(v, arms) = instr {
                if let Some(slot) = slots
                    .iter()
                    .position(|s| matches!(s, CarriedSlot::Scalar { phi } if *phi == *v))
                {
                    for (pred, vals) in &back_edges {
                        arms.push((*pred, vals[slot]));
                    }
                }
            }
        }
        // Pass 2: aggregate staging blits, per predecessor block. All read-phase
        // snapshots precede all write-phase stores; the predecessor is already
        // sealed with its `Jmp` to the header, so appending to `block.instrs`
        // lands the blits before the stored terminator.
        for (pred, vals) in &back_edges {
            let mut reads = Vec::new();
            let mut writes = Vec::new();
            for (slot, meta) in slots.iter().enumerate() {
                if let CarriedSlot::Aggregate { stable, temp, size } = *meta {
                    if size == 0 || vals[slot] == stable {
                        continue;
                    }
                    reads.push(Instr::Blit(vals[slot], temp, size));
                    writes.push(Instr::Blit(temp, stable, size));
                }
            }
            if reads.is_empty() {
                continue;
            }
            let block = self
                .blocks
                .iter_mut()
                .find(|b| b.id == *pred)
                .expect("back-edge predecessor block");
            block.instrs.append(&mut reads);
            block.instrs.append(&mut writes);
        }
    }

    /// A two-block compare-and-select (`max`/`max-total`'s shared shape,
    /// R12/R13): branch on `cond`, run each closure in its own block to
    /// produce that arm's value, and join with one `Phi`. Simpler than
    /// `lower_if`/`seal_arm` because a select's arms never back-edge (they
    /// lower no user terms, just a handful of value-producing instructions),
    /// so both predecessors always reach the join.
    fn emit_select(
        &mut self,
        cond: Value,
        then_fn: impl FnOnce(&mut Self) -> Value,
        else_fn: impl FnOnce(&mut Self) -> Value,
    ) -> Value {
        let then_id = self.fresh_block();
        let else_id = self.fresh_block();
        let join_id = self.fresh_block();
        self.seal_block(Terminator::Jnz(cond, then_id, else_id));

        self.start_block(then_id);
        self.terminated = false;
        let then_val = then_fn(self);
        let then_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(else_id);
        self.terminated = false;
        let else_val = else_fn(self);
        let else_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(join_id);
        self.terminated = false;
        let ty = self.value_type(then_val);
        let v = self.fresh_value(ty);
        self.push_instr(Instr::Phi(
            v,
            vec![(then_pred, then_val), (else_pred, else_val)],
        ));
        v
    }

    /// R13: the `total_cmp` bit-pattern key for one `max-total` operand.
    /// Reinterprets `operand`'s IEEE bits as an unsigned integer (an 8-byte
    /// scratch slot, stored/reloaded at the operand's own width — `Store`/
    /// `Load` already dispatch on the value's declared `IrType`, R20), then
    /// maps the bits to a monotone key: flip every bit if the sign bit is
    /// set, else flip only the sign bit. Comparing two keys as unsigned
    /// integers then reproduces the total order without ever comparing the
    /// floats themselves.
    fn total_order_key(&mut self, operand: Value, bits: u8) -> Value {
        let uty = IrType::Int {
            bits,
            signed: false,
        };
        let slot = self.fresh_value(IrType::Ptr);
        self.push_alloc(Instr::Alloc(slot, 8, 8));
        self.push_instr(Instr::Store(slot, operand));
        let raw = self.fresh_value(uty);
        self.push_instr(Instr::Load(raw, slot));

        let sign_mask: i64 = 1i64 << (bits - 1);
        let mask_v = self.fresh_value(uty);
        self.push_instr(Instr::Const(mask_v, sign_mask));
        let masked = self.fresh_value(uty);
        self.push_instr(Instr::Bin(masked, BinOp::And, raw, mask_v));
        let zero_u = self.fresh_value(uty);
        self.push_instr(Instr::Const(zero_u, 0));
        let is_neg = self.fresh_value(IrType::Bool);
        self.push_instr(Instr::Cmp(is_neg, CmpOp::Ne, masked, zero_u));

        self.emit_select(
            is_neg,
            |b| {
                let all_ones = b.fresh_value(uty);
                b.push_instr(Instr::Const(all_ones, -1));
                let key = b.fresh_value(uty);
                b.push_instr(Instr::Bin(key, BinOp::Xor, raw, all_ones));
                key
            },
            |b| {
                let key = b.fresh_value(uty);
                b.push_instr(Instr::Bin(key, BinOp::Xor, raw, mask_v));
                key
            },
        )
    }

    /// `tail` (R1) is true when this `if` is itself in tail position; it then
    /// hands tail position to the last term of both arms, so a self-call at the
    /// end of either arm back-edges (R7). An arm that back-edges leaves the
    /// builder `terminated` and contributes no predecessor to the join; the
    /// join is elided entirely when both arms back-edge (R8, both-arms-tail).
    fn lower_if(&mut self, then_branch: &[Term], else_branch: &[Term], tail: bool) {
        let test = self.stack.pop().expect("if: test value");
        let then_id = self.fresh_block();
        let else_id = self.fresh_block();
        let join_id = self.fresh_block();

        let post_pop = self.stack.clone();
        // R2: each arm is a block, so a name it binds is out of scope at its
        // terminator; the checker has already rejected any use past there.
        let locals_depth = self.locals.len();
        self.seal_block(Terminator::Jnz(test, then_id, else_id));

        self.start_block(then_id);
        self.terminated = false;
        self.stack = post_pop.clone();
        self.lower_terms(then_branch, tail);
        let then_arm = self.seal_arm(join_id);
        self.locals.truncate(locals_depth);

        self.start_block(else_id);
        self.terminated = false;
        self.stack = post_pop;
        self.lower_terms(else_branch, tail);
        let else_arm = self.seal_arm(join_id);
        self.locals.truncate(locals_depth);

        match (then_arm, else_arm) {
            (None, None) => {
                // Both arms back-edged to the loop header; the join is
                // unreachable and the enclosing body is terminated.
                self.terminated = true;
            }
            (Some((_, s)), None) | (None, Some((_, s))) => {
                // A single fall-through predecessor: values flow directly, no
                // phi needed.
                self.start_block(join_id);
                self.terminated = false;
                self.stack = s;
            }
            (Some((then_pred, then_stack)), Some((else_pred, else_stack))) => {
                // R11: two arms leaving *different* quotation phantoms are a
                // materialization boundary the checker already accepted; each
                // arm mints its `(code, env)` value in its own (sealed) block
                // so the join `Phi`s two real aggregates, not two phantoms
                // (which define no bytes). The same phantom in both arms
                // (`t == e`) stays a forwarded marker and is spliced past the
                // join, so it is never materialized here.
                let (then_stack, else_stack) =
                    self.materialize_join_quotations(then_pred, then_stack, else_pred, else_stack);
                self.start_block(join_id);
                self.terminated = false;
                let mut join_stack = Vec::with_capacity(then_stack.len());
                for (t, e) in then_stack.into_iter().zip(else_stack) {
                    if t == e {
                        join_stack.push(t);
                    } else {
                        let ty = self.value_type(t);
                        let v = self.fresh_value(ty);
                        self.push_instr(Instr::Phi(v, vec![(then_pred, t), (else_pred, e)]));
                        // A merged reference is still `Ptr`, which says
                        // nothing about its referent; carry the shape across
                        // the join so a projection past it still resolves.
                        if let Some(&referent) = self.ref_inner.get(&t) {
                            self.ref_inner.insert(v, referent);
                        }
                        join_stack.push(v);
                    }
                }
                self.stack = join_stack;
            }
        }
    }

    /// Seal a just-lowered `if` arm: if it back-edged (terminated) it jumps
    /// nowhere here and yields no join predecessor; otherwise it jumps to the
    /// join, yielding `(pred, stack)`.
    fn seal_arm(&mut self, join_id: BlockId) -> Option<(BlockId, Vec<Value>)> {
        if self.terminated {
            None
        } else {
            let s = self.stack.clone();
            let pred = self.cur_id;
            self.seal_block(Terminator::Jmp(join_id));
            Some((pred, s))
        }
    }

    /// Lower a clause-style word (R16): load the scrutinee's discriminant into
    /// a temp, dispatch N-way (a `Cmp(Eq)`-tag compare-chain to each variant's
    /// clause block, the last variant the terminal fall-through since coverage
    /// is exhaustive), and merge every clause's outputs at a single join block
    /// with one `Phi` per declared output over all N clause predecessors.
    ///
    /// This is deliberately *not* the 2-predecessor `lower_if` shape: the join
    /// has N predecessors and M outputs.
    pub(super) fn lower_clauses(
        &mut self,
        clauses: &[Clause],
        params: &[Value],
        scrutinee_ty: Type,
    ) {
        // A clause word is self-tail-recursive iff a header was opened (R6);
        // its clause bodies then carry tail position (D7).
        let tail = self.header.is_some();
        let scrutinee = *params.last().expect("clause word has a scrutinee input");
        let stack_below: Vec<Value> = params[..params.len() - 1].to_vec();
        // Threaded from the already-checked frontend `Type` rather than
        // re-derived from the lowered scrutinee's `IrType` — because a
        // `&!Enum` scrutinee lowers to the opaque `IrType::Ptr`, not
        // `IrType::Enum(id)`, so reading `self.value_type(scrutinee)` here
        // would make the enum arm below a reachable panic in reference mode.
        let (scrut_id, ref_mutable) = match scrutinee_ty {
            Type::Enum(id, _) => (id, None),
            Type::Ref(rid, mutable, _) => match self.refs.referent[rid.index()] {
                IrType::Enum(id) => (id, Some(mutable)),
                _ => unreachable!("checked: reference-mode clause scrutinee's referent is an enum"),
            },
            _ => unreachable!("checked: a clause word's top input is an enum"),
        };
        let payload_offset = self.enums.layouts[scrut_id.index()].payload_offset;
        let n = self.enums.layouts[scrut_id.index()].variants.len();

        // Slice 9 (R1/R5): a non-reference scrutinee of a zero-payload enum
        // (`Bool`, generally any all-unit-variant enum) is already the bare
        // discriminant, not a pointer to tagged storage; a reference
        // scrutinee is always a pointer regardless of the referent's own
        // representation, so it still needs the `FieldLoad`.
        let scrutinee_is_value =
            ref_mutable.is_none() && self.enums.layouts[scrut_id.index()].is_scalar;

        // Map each variant index to the clause handling it (checker-guaranteed
        // exact coverage), so dispatch on tag == variant_index lands correctly
        // regardless of clause source order.
        let clause_ids = self.dispatch_on_tag(scrutinee, scrut_id, scrutinee_is_value);
        let join_id = self.fresh_block();
        let mut clause_for_variant: Vec<Option<&Clause>> = vec![None; n];
        for clause in clauses {
            let EnumWord::Construct(_, vi) = self.enums.words[&clause.variant];
            clause_for_variant[vi] = Some(clause);
        }

        let mut clause_ends: Vec<(BlockId, Vec<Value>)> = Vec::with_capacity(n);
        for vi in 0..n {
            let clause = clause_for_variant[vi].expect("checked: exhaustive coverage");
            self.start_block(clause_ids[vi]);
            self.locals.clear();
            self.stack = stack_below.clone();
            // Push the variant's payload first-deepest, loading each field from
            // `payload_offset + field.offset`. In reference mode every
            // field is pushed as a reference to its own storage inside the
            // scrutinee (its address, never its value), registered in
            // `ref_inner` so a later access/projection through it resolves the
            // right shape — the same `IrType::Ptr` any other reference lowers
            // to.
            let fields = self.enums.layouts[scrut_id.index()].variants[vi]
                .fields
                .clone();
            for field in &fields {
                let adjusted = FieldLayout {
                    offset: payload_offset + field.offset,
                    ..*field
                };
                match ref_mutable {
                    Some(_) => {
                        let fptr = self.field_ptr(scrutinee, adjusted.offset);
                        self.push_reference(fptr, adjusted.ty);
                    }
                    None => self.load_field_onto_stack(scrutinee, adjusted),
                }
            }
            // Bind clause-body `| names |` locals (top N, leftmost deepest).
            let take = clause.locals.len();
            let bound = self.stack.split_off(self.stack.len() - take);
            for (name, value) in clause.locals.iter().zip(bound) {
                self.locals.push((name.clone(), value));
            }
            // R7/R9: a clause whose body ends in a tail self-call back-edges to
            // the shared loop header and contributes no join predecessor;
            // `tail` is true iff this word is self-tail-recursive. The header
            // phi preds (entry + tail clause ends) and the dispatch-join phi
            // preds (non-tail clause ends) therefore stay disjoint.
            self.terminated = false;
            self.lower_terms(&clause.body, tail);
            if !self.terminated {
                let result = self.stack.clone();
                let pred = self.cur_id;
                self.seal_block(Terminator::Jmp(join_id));
                clause_ends.push((pred, result));
            }
        }

        // Every clause back-edged: the join is unreachable and the word is
        // terminated (no fall-through Ret).
        if clause_ends.is_empty() {
            self.terminated = true;
            return;
        }

        // Single join block: one phi per declared output, merging the
        // fall-through clause predecessors.
        self.start_block(join_id);
        self.terminated = false;
        let m = clause_ends[0].1.len();
        let mut join_stack = Vec::with_capacity(m);
        for out_i in 0..m {
            let arms: Vec<(BlockId, Value)> = clause_ends
                .iter()
                .map(|(pred, st)| (*pred, st[out_i]))
                .collect();
            let ty = self.value_type(arms[0].1);
            let v = self.fresh_value(ty);
            self.push_instr(Instr::Phi(v, arms));
            join_stack.push(v);
        }
        self.stack = join_stack;
    }
}
