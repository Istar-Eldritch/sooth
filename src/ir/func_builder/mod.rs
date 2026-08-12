//! `FuncBuilder`: the compile-time virtual-stack lowering machine. Holds the
//! per-function block/value/loop bookkeeping and the term-lowering methods that
//! turn a word body into SSA-shaped blocks. Depends on `types`, `layout`, and
//! `destructors`; the lowering driver (parent module) drives it.

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

    pub(super) fn lower_terms(&mut self, terms: &[Term], tail: bool) {
        // Only the final term of a body can be in tail position (R1); a term
        // followed by any further term is not. This positional `tail` threading
        // is the same syntactic rule as the checker's `tail_position_calls`
        // (src/check.rs); the two must stay in lockstep if the rule changes.
        let last = terms.len().wrapping_sub(1);
        for (i, term) in terms.iter().enumerate() {
            self.lower_term(term, tail && i == last);
        }
    }

    pub(super) fn lower_term(&mut self, term: &Term, tail: bool) {
        match &term.kind {
            TermKind::IntLit(n) => {
                let v = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(v, *n));
                self.const_vals.insert(v, *n);
                self.stack.push(v);
            }
            TermKind::FloatLit(x) => {
                let v = self.fresh_value(IrType::Float { bits: 64 });
                self.push_instr(Instr::ConstF(v, *x));
                self.stack.push(v);
            }
            TermKind::StrLit(s) => {
                let v = self.fresh_value(IrType::Str);
                self.push_instr(Instr::StrLit(v, s.clone()));
                self.stack.push(v);
            }
            TermKind::Call(name) => self.lower_call(name, term.span, tail),
            TermKind::Bind(names) => {
                // R10: a binding is a compile-time rebinding of SSA values, so
                // it emits nothing. Leftmost name takes the deepest value.
                let bound = self.stack.split_off(self.stack.len() - names.len());
                for (name, value) in names.iter().zip(bound) {
                    self.locals.push((name.clone(), value));
                }
            }
            TermKind::If {
                then_branch,
                else_branch,
                ..
            } => self.lower_if(then_branch, else_branch, tail),
            // R12: a quotation literal interns its body and lowers to a phantom
            // `Value` with a placeholder `IrType` and *no* `Instr`. The checker
            // guarantees this phantom reaches only `call`/`times`/shuffle/bind
            // or a materialization boundary (a store, a word output, or a
            // branch join, R11) -- where it is turned into a real `(code, env)`
            // aggregate *before* it enters a `Phi`, operand, terminator, or
            // runtime code value; it never enters one as a bare phantom. `I64`
            // is the plainest non-aggregate placeholder (the IR side has no
            // `if`-condition concern, so the checker's `Cstr` choice does not
            // bind here).
            TermKind::Quotation(body) => {
                let id = QuotId(self.quot_defs.len());
                self.quot_defs.push(body.clone());
                let v = self.fresh_value(IrType::I64);
                self.quot_bodies.insert(v, id);
                self.stack.push(v);
            }
            // Slice 6h (D3): one `Instr::Alloc` for the array's inline
            // aggregate, then a byte-granular zero-init loop of exactly
            // `ArrayLayout::size` bytes. Byte granularity (not a word store)
            // because an array is not word-padded, so a wider store would
            // overrun the allocation; the runtime cost is a deliberate trade
            // for one obviously-correct path (code size stays O(1) in Count).
            TermKind::ArrayCtor(ty) => {
                let Type::Array(id, _) = *ty else {
                    unreachable!("the parser only ever interns a Type::Array for an ArrayCtor term")
                };
                // Save/restore the enclosing loop state so this loop composes
                // with a later `Alloc` or loop (see `save_loop_state`).
                let saved_loop_state = self.save_loop_state();
                let size = self.arrays.layouts[id.index()].size;
                // The destination reaches the loop body and the exit block by
                // dominance; it must not be carried, or the back-edge staging
                // blit would copy a stale snapshot over each iteration's store.
                let dst = self.alloc_array(id);

                let seed = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(seed, 0));
                self.const_vals.insert(seed, 0);
                // Carry only the induction index, with `stage_aggregates =
                // false`.
                let outs = self.begin_loop(&[seed], false);
                let index_phi = outs[0];

                // Header (current after `begin_loop`): loop while index < size.
                let bound = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(bound, size as i64));
                self.const_vals.insert(bound, size as i64);
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Lt, index_phi, bound));
                let body_block = self.fresh_block();
                let exit_block = self.fresh_block();
                self.seal_block(Terminator::Jnz(cmp, body_block, exit_block));

                // Body: store one zero byte at `dst + index`. `ElemAddr` with
                // `stride = 1` gives byte addressing off the runtime index; a
                // `FieldStore` of an 8-bit-typed `Const` picks `storeb`.
                self.start_block(body_block);
                self.terminated = false;
                let fptr = self.elem_addr(dst, index_phi, 1);
                let zero = self.fresh_value(IrType::Int {
                    bits: 8,
                    signed: false,
                });
                self.push_instr(Instr::Const(zero, 0));
                self.push_instr(Instr::FieldStore(fptr, zero));

                // Back-edge: index + 1.
                let one = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(one, 1));
                self.const_vals.insert(one, 1);
                let index_next = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Bin(index_next, BinOp::Add, index_phi, one));
                self.back_edges.push((self.cur_id, vec![index_next]));
                self.seal_block(Terminator::Jmp(
                    self.header.expect("array ctor loop header"),
                ));

                self.finalize_loop();

                // Exit: the array is the constructor's result. The fixed loop
                // body never terminates, so `terminated` is already false here
                // (unlike the `times` arm, whose spliced user body can); the
                // reset keeps this arm identical to the loop template so it
                // stays correct if the body ever gains a terminating term.
                self.start_block(exit_block);
                self.terminated = false;
                self.stack.push(dst);

                self.restore_loop_state(saved_loop_state);
            }
        }
    }

    /// R10: lower a self-tail combinator (`while`) as a splice-time loop,
    /// composing the `times` arm's mid-body loop opening with the whole-word
    /// transform's self-call-driven back-edge. The body's leading quotation
    /// binding(s) are lowered *before* `begin_loop`, so the loop-invariant
    /// `Copy` quotation phantom is bound to a local (resolved statically each
    /// iteration) and excluded from the loop-carried phis; only the state row
    /// is carried. `stage_aggregates = true` reuses the slice-3 aggregate
    /// staging verbatim for a carried-aggregate state. The enclosing loop
    /// state is saved and restored so loops compose, exactly as the `times`
    /// arm does. A tail-position self-call inside the body is emitted as a
    /// back-edge (`lower_call`, keyed on `cur_combinator`), never a re-splice.
    pub(super) fn lower_self_tail_combinator(&mut self, name: &str, body: &[Term]) {
        let saved_loop_state = self.save_loop_state();
        let saved_combinator = self.cur_combinator.take();
        let locals_depth = self.locals.len();

        // Lower the leading quotation binding(s) before opening the loop: a
        // `Bind` term that pops any quotation phantom becomes a local, so the
        // phantom is not carried in a phi (R10). Everything after is the loop
        // body.
        let mut split = 0;
        while split < body.len() {
            if let TermKind::Bind(names) = &body[split].kind {
                let base = self.stack.len() - names.len();
                let binds_quot = self.stack[base..]
                    .iter()
                    .any(|v| self.quot_bodies.contains_key(v));
                if binds_quot {
                    self.lower_term(&body[split], false);
                    split += 1;
                    continue;
                }
            }
            break;
        }

        // The carried row is whatever remains: the caller's residual plus the
        // threaded state. `begin_loop` seals the entry block, opens the
        // header, and returns one value per carried slot (a scalar phi output
        // or an aggregate stable slot).
        let row_len = self.stack.len();
        let params = mem::take(&mut self.stack);
        let outs = self.begin_loop(&params, true);
        self.stack = outs;
        self.cur_combinator = Some((name.to_string(), row_len));

        // Lower the loop body with `tail = true`, so its tail-position
        // self-call is recognized as the back-edge. The base-case arm falls
        // through, leaving the state on the stack as the loop's result.
        self.lower_terms(&body[split..], true);
        self.finalize_loop();

        self.locals.truncate(locals_depth);
        self.cur_combinator = saved_combinator;
        self.restore_loop_state(saved_loop_state);
    }

    pub(super) fn lower_call(&mut self, name: &str, span: Span, tail: bool) {
        let line = span.line;
        if let Some(&(_, value)) = self.locals.iter().find(|(n, _)| n == name) {
            self.stack.push(value); // i64 is Copy; reuse the value id.
            return;
        }
        // Slice 8a phase 4 (R7): a call site the checker resolved to a user
        // overload of a builtin-named word must dispatch to that word, not
        // the name-directed builtin arms below (a literal name match like
        // "+" would otherwise always win) nor the self-tail/back-edge checks
        // further down (a word named `.` overloading print must not be
        // miscategorized as a self-tail call on `.`). Same env-lookup +
        // resolve + bundle-unpack shape as the ordinary user-word path in the
        // `_` arm below, since this *is* that path, reached early.
        if let Some(sym_name) = self.builtin_overloads.get(&span).cloned() {
            let (in_arity, out_arity, ret_ty) = *self
                .env
                .get(&sym_name)
                .expect("checked user overload exists");
            let split = self.stack.len() - in_arity;
            let args = self.stack.split_off(split);
            let bundle = match ret_ty {
                Some(IrType::Struct(id)) if self.structs.layouts[id.index()].bundle => Some(id),
                _ => None,
            };
            let ret = if out_arity == 1 || bundle.is_some() {
                Some(self.fresh_value(ret_ty.unwrap_or(IrType::I64)))
            } else {
                None
            };
            let sym = (self.resolve)(&sym_name);
            self.push_instr(Instr::Call(ret, sym, args));
            if let Some(v) = ret {
                self.stack.push(v);
            }
            if let Some(id) = bundle {
                self.unpack_bundle(id);
            }
            return;
        }
        // R10: a tail-position self-call inside a self-tail combinator splice
        // is the loop back-edge. The loop carries only the state row (length
        // recorded when the loop opened); the quotation argument(s) above it
        // are loop-invariant, resolved statically through the local binding,
        // so they are dropped here and not fed to a phi. Intercepted before
        // the combinator dispatch below (which would re-splice) and distinct
        // from the whole-word self-tail back-edge (keyed on `cur_word_name`).
        if tail && self.header.is_some() {
            if let Some((cname, row_len)) = self.cur_combinator.clone() {
                if cname == name {
                    let drop_n = self.stack.len() - row_len;
                    self.stack.truncate(self.stack.len() - drop_n);
                    let args = mem::take(&mut self.stack);
                    self.back_edges.push((self.cur_id, args));
                    let header = self.header.expect("combinator loop header");
                    self.seal_block(Terminator::Jmp(header));
                    self.terminated = true;
                    return;
                }
            }
        }
        // R14/R11: a call to a polymorphic word resolves entirely through the
        // instantiation table keyed by this call site's span, never the
        // name-keyed `env`/`resolve` (which cannot distinguish one θ from
        // another). This is checked before the builtin/user dispatch below
        // because a polymorphic callee is always a user word whose name is
        // none of the builtins.
        if let Some(inst) = self.instantiations.get(&span).cloned() {
            self.lower_poly_call(&inst);
            return;
        }
        match name {
            // R13: `call`-of-literal fusion. Pop the phantom quotation `Value`,
            // resolve its body, and lower the body's terms in place, emitting
            // no `Instr::Call` and creating no runtime code value: `[ 1 + ]
            // call` lowers exactly as `1 +` (D5). `tail = false` is
            // load-bearing: the checker never sanctions a spliced term as a
            // self-tail call (R6/R13), so lowering must not back-edge here.
            "call" => {
                let v = self.stack.pop().expect("call: quotation on stack");
                // R10/D1: provenance decides. A phantom the checker resolved to
                // a literal splices its body (D5, no `Instr::Call`); a
                // materialized value whose identity was erased loads its `code`
                // slot and calls indirectly.
                match self.quot_bodies.get(&v).copied() {
                    Some(id) => {
                        let body = self.quot_defs[id.0].clone();
                        // The body is a block: a name it binds is out of scope
                        // after the splice, and the front-first local resolver
                        // would else read a stale entry on a later same-named
                        // bind. Mirror the `if` arm's save-and-truncate.
                        let locals_depth = self.locals.len();
                        self.lower_terms(&body, false);
                        self.locals.truncate(locals_depth);
                    }
                    None => self.lower_indirect_call(v),
                }
            }
            // R14: `times` lowers into a constant-stack loop, reusing
            // `begin_loop`/`finalize_loop` (D6). A synthesized index drives a
            // header `Jnz(index < count)`; the body reads the index as its top
            // input and returns the row on the back-edge (R18). `tail = false`
            // for the same reason as `call`.
            "times" => {
                // 6d/R5: a nested `times` is now legal (the checker's R18
                // rejection retired), so no `debug_assert` on `header.is_none()`
                // here; the hoist-target split (R1-R3) keeps it constant-stack.
                // R15: save the loop state (see `save_loop_state`'s doc) and
                // restore it after the loop, so a nested `times` composes and a
                // later `Alloc` in the same word does not hoist into this now-
                // dead `times` preheader.
                let saved_loop_state = self.save_loop_state();

                let qv = self.stack.pop().expect("times: quotation on stack");
                // R10/D1/D6: provenance decides, exactly as `call`. A phantom
                // the checker resolved to a literal splices its body per
                // iteration; a materialized value whose identity was erased is
                // indirect-called once per iteration (still constant stack).
                let quot_id = self.quot_bodies.get(&qv).copied();
                let count = self.stack.pop().expect("times: count on stack");

                // Synthesize the induction variable seeded 0; the row is the
                // remaining stack. `stage_aggregates = true` (R17): a carried
                // aggregate rides slice 3's entry-hoisted stable slot, and the
                // index gets a scalar phi.
                let seed = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(seed, 0));
                self.const_vals.insert(seed, 0);
                let mut params = mem::take(&mut self.stack);
                params.push(seed);
                let outs = self.begin_loop(&params, true);
                let index_phi = *outs.last().expect("times: index phi");
                let row_phis: Vec<Value> = outs[..outs.len() - 1].to_vec();

                // Header (current after `begin_loop`): loop while index < count.
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Lt, index_phi, count));
                let body_block = self.fresh_block();
                let exit_block = self.fresh_block();
                self.seal_block(Terminator::Jnz(cmp, body_block, exit_block));

                // Body: the row plus the index (top input), spliced `tail =
                // false`. `alloca_home` stays `Some` across the splice, so an
                // aggregate the body constructs hoists its `Alloc` into the
                // invariant alloca home (R17/6d), not the per-iteration body
                // block, at any nesting depth.
                self.start_block(body_block);
                self.terminated = false;
                self.stack = row_phis;
                self.stack.push(index_phi);
                let locals_depth = self.locals.len();
                match quot_id {
                    Some(id) => {
                        let body = self.quot_defs[id.0].clone();
                        self.lower_terms(&body, false);
                    }
                    None => self.lower_indirect_call(qv),
                }
                self.locals.truncate(locals_depth);

                // Back-edge: the body's result row plus index + 1.
                let one = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(one, 1));
                self.const_vals.insert(one, 1);
                let index_next = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Bin(index_next, BinOp::Add, index_phi, one));
                // With `tail = false` and no `Return` in a body, nothing can
                // terminate the body block, so a double seal is impossible.
                debug_assert!(
                    !self.terminated,
                    "a `tail = false` `times` body cannot terminate"
                );
                let mut args = mem::take(&mut self.stack);
                args.push(index_next);
                self.back_edges.push((self.cur_id, args));
                self.seal_block(Terminator::Jmp(self.header.expect("times loop header")));

                // Back-patch the scalar phis (row scalars + index) and append
                // the aggregate staging blits on the back-edge (unchanged from
                // slice 3).
                self.finalize_loop();

                // Exit: the carried row (scalar header-phi outputs / aggregate
                // stable slots), minus the trailing index. Reset `terminated`
                // (the body seal set it) or every term after the `times` is
                // silently dropped.
                self.start_block(exit_block);
                self.terminated = false;
                let mut exit_stack = outs;
                exit_stack.pop();
                self.stack = exit_stack;

                // R15: restore the pre-`times` loop state so the `times`
                // composes with a later `Alloc` or a second sequential `times`.
                self.restore_loop_state(saved_loop_state);
            }
            "dup" => {
                let top = *self.stack.last().expect("dup: non-empty stack");
                // A scalar is `Copy`: reuse the value id (dup emits nothing). A
                // struct or enum is copied by value: alloc a fresh slot and
                // blit the bytes, so mutating the copy leaves the original
                // intact (an enum is all-Copy too, D3).
                match self.value_type(top) {
                    IrType::Struct(id) => {
                        let copy = self.alloc_struct(id);
                        let size = self.structs.layouts[id.index()].size;
                        if size > 0 {
                            self.push_instr(Instr::Blit(top, copy, size));
                        }
                        self.stack.push(copy);
                    }
                    // Slice 9 (R1): a zero-payload enum's value is a bare
                    // scalar -- `Copy` like any other, reuse the value id.
                    IrType::Enum(id) if self.enums.layouts[id.index()].is_scalar => {
                        self.stack.push(top);
                    }
                    IrType::Enum(id) => {
                        let copy = self.alloc_enum(id);
                        let size = self.enums.layouts[id.index()].size;
                        if size > 0 {
                            self.push_instr(Instr::Blit(top, copy, size));
                        }
                        self.stack.push(copy);
                    }
                    IrType::Array(id) => {
                        let copy = self.alloc_array(id);
                        let size = self.arrays.layouts[id.index()].size;
                        if size > 0 {
                            self.push_instr(Instr::Blit(top, copy, size));
                        }
                        self.stack.push(copy);
                    }
                    _ => self.stack.push(top),
                }
            }
            "drop" => {
                let v = self.stack.pop().expect("drop: non-empty stack");
                self.emit_drop(v);
            }
            "swap" => {
                let n = self.stack.len();
                self.stack.swap(n - 1, n - 2);
            }
            "over" => {
                let below = self.stack[self.stack.len() - 2];
                self.stack.push(below);
            }
            "rot" => {
                // a b c -> b c a
                let n = self.stack.len();
                let a = self.stack[n - 3];
                self.stack[n - 3] = self.stack[n - 2];
                self.stack[n - 2] = self.stack[n - 1];
                self.stack[n - 1] = a;
            }
            "+" | "-" | "*" | "/" | "mod" | "and" | "or" | "xor" | "shl" | "shr" => {
                let op = match name {
                    "+" => BinOp::Add,
                    "-" => BinOp::Sub,
                    "*" => BinOp::Mul,
                    "/" => BinOp::Div,
                    "mod" => BinOp::Rem,
                    "and" => BinOp::And,
                    "or" => BinOp::Or,
                    "xor" => BinOp::Xor,
                    "shl" => BinOp::Shl,
                    _ => BinOp::Shr,
                };
                let rhs = self.stack.pop().expect("bin: rhs");
                let lhs = self.stack.pop().expect("bin: lhs");
                // Arithmetic/bitwise ops are homogeneous in their result
                // (checker-guaranteed): the result carries the lhs's type, so
                // the backend picks its width. `shl`/`shr`'s rhs is always an
                // `i64` count, not the lhs's type.
                let ty = self.value_type(lhs);
                let v = self.fresh_value(ty);
                self.push_instr(Instr::Bin(v, op, lhs, rhs));
                self.stack.push(v);
            }
            "not" => {
                // No unary QBE op: `not` is `xor operand, mask`. On an integer,
                // complement is `xor operand, -1` at the operand's own width
                // (`-1` is all-ones at any width in two's complement, so it
                // works whether the register is `w` or `l`). On a `bool`,
                // `not` is logical negation of a canonical 0/1 value, which
                // flips only the low bit (`xor operand, 1`); `xor -1` would
                // give -1/-2, not 0/1.
                let operand = self.stack.pop().expect("not: operand");
                let ty = self.value_type(operand);
                // `not`'s builtin row is `int_types() + Bool` only (checker-
                // guaranteed, R4): the operand is never anything else, so
                // "not an int" identifies the `Bool` case exactly, whatever
                // `Bool`'s own `IrType` migration makes it (Slice 9: it is
                // `IrType::Enum(BOOL_ENUM_ID)`, not the retired `IrType::Bool`).
                let mask: i64 = if matches!(ty, IrType::Int { .. }) {
                    -1
                } else {
                    1
                };
                let mask_v = self.fresh_value(ty);
                self.push_instr(Instr::Const(mask_v, mask));
                let v = self.fresh_value(ty);
                self.push_instr(Instr::Bin(v, BinOp::Xor, operand, mask_v));
                self.stack.push(v);
            }
            "=" | "<" | ">" | "<=" | ">=" | "<>" => {
                let op = match name {
                    "=" => CmpOp::Eq,
                    "<" => CmpOp::Lt,
                    ">" => CmpOp::Gt,
                    "<=" => CmpOp::Le,
                    ">=" => CmpOp::Ge,
                    _ => CmpOp::Ne,
                };
                let rhs = self.stack.pop().expect("cmp: rhs");
                let lhs = self.stack.pop().expect("cmp: lhs");
                let v = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(v, op, lhs, rhs));
                self.stack.push(v);
            }
            // R12 (S6): `max` over the integer tower, inline compare-and-select
            // (`Cmp(Gt)` plus a two-block phi-join), no `Instr::Call`, no
            // monomorphization.
            "max" => {
                let rhs = self.stack.pop().expect("max: rhs");
                let lhs = self.stack.pop().expect("max: lhs");
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Gt, lhs, rhs));
                let v = self.emit_select(cmp, |_| lhs, |_| rhs);
                self.stack.push(v);
            }
            // R13 (S6): `max-total` over `f32`/`f64`, ordered by the
            // `total_cmp` bit-pattern rule (map each operand's IEEE bits to a
            // monotone unsigned key — flip every bit if the sign bit is set,
            // else flip only the sign bit — then integer-compare the keys),
            // so no float `>` is ever emitted.
            "max-total" => {
                let rhs = self.stack.pop().expect("max-total: rhs");
                let lhs = self.stack.pop().expect("max-total: lhs");
                let bits: u8 = match self.value_type(lhs) {
                    IrType::Float { bits } => bits,
                    other => unreachable!("checked: max-total operand is a float, got {other:?}"),
                };
                let lhs_key = self.total_order_key(lhs, bits);
                let rhs_key = self.total_order_key(rhs, bits);
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Gt, lhs_key, rhs_key));
                let v = self.emit_select(cmp, |_| lhs, |_| rhs);
                self.stack.push(v);
            }
            "." => {
                let v = self.stack.pop().expect("print: value");
                self.push_instr(Instr::Print(v));
            }
            "fill" => self.lower_array_word(name),
            "len" => {
                let top = *self.stack.last().expect("len: operand");
                if self.value_type(top) == IrType::Str {
                    // R8: consuming, unlike the array `len` fold: the
                    // length is carried at runtime, not derivable from the
                    // type.
                    self.stack.pop();
                    let v = self.fresh_value(IrType::Usize);
                    self.push_instr(Instr::StrLen(v, top));
                    self.stack.push(v);
                } else {
                    self.lower_array_word(name);
                }
            }
            "cstr" => {
                // R7: discard the length, keep the bytes pointer.
                let s = self.stack.pop().expect("cstr: str operand");
                let v = self.fresh_value(IrType::Cstr);
                self.push_instr(Instr::StrPtr(v, s));
                self.stack.push(v);
            }
            "^" | "^>" | "^|>" => self.lower_owned_cell_word(name),
            "@" | "!" | "+!" => self.lower_access_word(name),
            _ => {
                // R19: a call to a monomorphic combinator is inlined, not
                // lowered to an `Instr::Call` -- the callee mints no `IrFunc`
                // (R20), so its only reachable form is this splice. The
                // caller's quotation literals sit on `self.stack` as phantom
                // `Value`s already (a `TermKind::Quotation` earlier in this
                // body recorded each `Value -> QuotId`), so the spliced body's
                // own `call`/`times` resolves them with no extra plumbing.
                // `tail = false` and the locals-truncate mirror the `call`
                // splice above. Checked before the `&`/conversion/struct
                // dispatch since a combinator name is an ordinary word name.
                if let Some(body) = self.combinators.get(name) {
                    // R10: a self-tail combinator lowers to a splice-time loop
                    // (back-edge, not re-splice); every other combinator is a
                    // straight term-splice.
                    let self_tail = body_tail_calls_self(body, name);
                    // R18/R21: alpha-rename the callee body identically to the
                    // checker, so its `| ... |` locals are fresh and a
                    // passed-down literal keeps its lexical capture under
                    // transitive inlining.
                    let uid = self.inline_uid;
                    self.inline_uid += 1;
                    let body = crate::ast::alpha_rename_locals(body, uid);
                    if self_tail {
                        self.lower_self_tail_combinator(name, &body);
                    } else {
                        let locals_depth = self.locals.len();
                        self.lower_terms(&body, false);
                        self.locals.truncate(locals_depth);
                    }
                    return;
                }
                // Every `&`-led word: the two prefix borrow operators and the
                // reference-mode accessor family.
                if name.starts_with('&') {
                    self.lower_reference_word(name, line);
                    return;
                }
                // A conversion word `>iN`/`>uN`/`>f32`/`>f64`
                // (checker-guaranteed numeric source): pop one, push the
                // target-typed result. The backend reads the two `IrType`s to
                // pick the int/float conversion op (R18).
                if let Some(target) = name
                    .strip_prefix('>')
                    .filter(|r| !r.is_empty())
                    .and_then(Type::from_name)
                    .filter(Type::is_numeric)
                {
                    let src = self.stack.pop().expect("conv: source");
                    let dst = self.fresh_value(ir_type_of(target));
                    self.push_instr(Instr::Conv(dst, src));
                    self.stack.push(dst);
                    return;
                }
                // A generated struct word (`S`/`S>`/`S>fi`/`S<fi`/`S|>fi`) lowers to
                // alloc/blit/field-load-store inline, not a normal call.
                if let Some(&sw) = self.structs.words.get(name) {
                    self.lower_struct_word(sw);
                    return;
                }
                // A variant constructor lowers to alloc + tag store + field
                // stores inline, parallel to a struct constructor (R14/R15).
                if let Some(&ew) = self.enums.words.get(name) {
                    self.lower_enum_word(ew);
                    return;
                }
                // R7: a tail-position self-call is a back-edge to the loop
                // header, not a real call. `self.header` is `Some` iff the word
                // is self-tail-recursive (R6), and `tail` marks the syntactic
                // tail position (R1); a non-tail self-call (R10) falls through
                // to the ordinary `Instr::Call` below. Pop the args as the
                // back-edge phi operands (one per carried slot; a self-call's
                // input arity is the word's own signature, so the count always
                // matches the header phi count) and jump.
                //
                // R11: the back-edge is the defined destructor insertion point
                // for this iteration's non-forwarded affine values; in Phase 2
                // every type is `Copy`, so the drop set is empty and no drop
                // glue is emitted here.
                if tail && self.header.is_some() && name == self.cur_word_name {
                    let (in_arity, ..) = *self.env.get(name).expect("checked user word exists");
                    let split = self.stack.len() - in_arity;
                    let args = self.stack.split_off(split);
                    self.back_edges.push((self.cur_id, args));
                    self.seal_block(Terminator::Jmp(self.header.expect("loop header")));
                    self.terminated = true;
                    return;
                }
                let (in_arity, out_arity, ret_ty) =
                    *self.env.get(name).expect("checked user word exists");
                let split = self.stack.len() - in_arity;
                let args = self.stack.split_off(split);
                // R11: a multi-output callee returns one bundle, unpacked back
                // onto the stack below, so the lowering stack matches the
                // stack the checker verified. The discriminator is the
                // bundle's own flag, not `out_arity >= 2`: the REPL's env
                // derives a multi-output `ret_ty` from the first output alone
                // and interns no bundle, and must not enter this branch.
                let bundle = match ret_ty {
                    Some(IrType::Struct(id)) if self.structs.layouts[id.index()].bundle => Some(id),
                    _ => None,
                };
                let ret = if out_arity == 1 || bundle.is_some() {
                    Some(self.fresh_value(ret_ty.unwrap_or(IrType::I64)))
                } else {
                    None
                };
                let sym = (self.resolve)(name);
                self.push_instr(Instr::Call(ret, sym, args));
                if let Some(v) = ret {
                    self.stack.push(v);
                }
                if let Some(id) = bundle {
                    self.unpack_bundle(id);
                }
            }
        }
    }

    /// Push a reference `Value` (always `IrType::Ptr`) and record what it
    /// points at, since the `IrType` deliberately no longer says.
    fn push_reference(&mut self, ptr: Value, referent: IrType) {
        self.ref_inner.insert(ptr, referent);
        self.stack.push(ptr);
    }

    /// The referent shape of a reference `Value`.
    fn referent_of(&self, ptr: Value) -> IrType {
        *self
            .ref_inner
            .get(&ptr)
            .expect("checked: every reference value records its referent")
    }

    /// Lower a `&`-led word. No new `Instr` variant: a struct
    /// field projection is a `PtrOffset`, an array element projection an
    /// `ElemAddr` behind a runtime bounds guard, and a cell payload
    /// projection a `Load` of the pointer the place holds.
    fn lower_reference_word(&mut self, name: &str, line: u32) {
        let mutable = name.starts_with("&!");
        let rest = &name[if mutable { 2 } else { 1 }..];
        match rest {
            ">" => {
                let index = self.stack.pop().expect("&>: index");
                let base = self.stack.pop().expect("&>: array reference");
                let IrType::Array(id) = self.referent_of(base) else {
                    unreachable!("checked: `&>`'s receiver references an array")
                };
                let (stride, elem, count) = self.array_parts(id);
                self.bounds_check(index, count, line);
                let addr = self.elem_addr(base, index, stride);
                self.push_reference(addr, elem);
            }
            "^" => {
                let base = self.stack.pop().expect("&^: cell reference");
                let IrType::OwnedCell(id) = self.referent_of(base) else {
                    unreachable!("checked: `&^`'s receiver references an owning cell")
                };
                let payload = self.cells.payload[id.index()];
                // The place holds the cell's heap pointer; the payload lives
                // at that pointer, so the projection reads it out.
                let cell_ptr = self.fresh_value(IrType::Ptr);
                self.push_instr(Instr::Load(cell_ptr, base));
                self.push_reference(cell_ptr, payload);
            }
            _ => {
                if let Some(&StructWord::Get(id, fi)) = self.structs.words.get(rest) {
                    let base = self.stack.pop().expect("field projection: receiver");
                    let field = self.structs.layouts[id.index()].fields[fi];
                    let addr = self.field_ptr(base, field.offset);
                    self.push_reference(addr, field.ty);
                    return;
                }
                let value = self
                    .locals
                    .iter()
                    .find(|(n, _)| n == rest)
                    .map(|(_, v)| *v)
                    .expect("checked: a borrow's operand is a local");
                self.lower_borrow(value);
            }
        }
    }

    /// Borrow a local. An aggregate local's own value *is* a pointer to
    /// its storage, so the borrow is that pointer retyped as an opaque handle.
    /// A cell local's value is the heap pointer itself, an SSA temporary with
    /// no address of its own; `&^`/`&!^` reads a cell reference by loading the
    /// pointer out of the place holding it, so borrowing a cell local first
    /// gives it a place.
    fn lower_borrow(&mut self, value: Value) {
        let referent = self.value_type(value);
        let ptr = match referent {
            IrType::OwnedCell(_) => {
                let slot = self.fresh_value(IrType::Ptr);
                self.push_alloc(Instr::Alloc(slot, WORD_WIDTH, WORD_WIDTH));
                self.push_instr(Instr::Store(slot, value));
                slot
            }
            _ => {
                let p = self.fresh_value(IrType::Ptr);
                self.push_instr(Instr::PtrOffset(p, value, 0));
                p
            }
        };
        self.push_reference(ptr, referent);
    }

    /// `@` fetches through a reference, `!` stores, `+!` adds in place.
    /// The referent is checker-guaranteed `Copy`; a Copy *aggregate* is a real
    /// case, taking the `Alloc`+`Blit` / `Blit` path `dup` already uses for
    /// the same shape of copy.
    fn lower_access_word(&mut self, name: &str) {
        match name {
            "@" => {
                let ptr = self.stack.pop().expect("@: reference");
                let referent = self.referent_of(ptr);
                match referent {
                    // Slice 9 (R1): a reference to a zero-payload enum reads
                    // a scalar (a width-exact `FieldLoad`), not the
                    // interior-pointer aggregate view below.
                    IrType::Enum(id) if self.enums.layouts[id.index()].is_scalar => {
                        let v = self.fresh_value(referent);
                        self.push_instr(Instr::FieldLoad(v, ptr));
                        self.stack.push(v);
                    }
                    IrType::Struct(_)
                    | IrType::Enum(_)
                    | IrType::Array(_)
                    | IrType::Quotation(_) => {
                        let dst = self.alloc_aggregate(referent);
                        let size = self.value_size(referent);
                        if size > 0 {
                            self.push_instr(Instr::Blit(ptr, dst, size));
                        }
                        self.stack.push(dst);
                    }
                    _ => {
                        let v = self.fresh_value(referent);
                        self.push_instr(Instr::FieldLoad(v, ptr));
                        self.stack.push(v);
                    }
                }
            }
            "!" => {
                let val = self.stack.pop().expect("!: value");
                let ptr = self.stack.pop().expect("!: reference");
                let referent = self.referent_of(ptr);
                // R7/R9: a `&!Type::Quotation` store is a materialization
                // boundary (an array element or struct field via reference); a
                // phantom `val` becomes a real `(code, env)` aggregate first.
                let val = self.materialize_if_phantom(val, referent);
                match referent {
                    // Slice 9 (R1): a reference to a zero-payload enum stores
                    // as a scalar.
                    IrType::Enum(id) if self.enums.layouts[id.index()].is_scalar => {
                        self.push_instr(Instr::FieldStore(ptr, val));
                    }
                    IrType::Struct(_)
                    | IrType::Enum(_)
                    | IrType::Array(_)
                    | IrType::Quotation(_) => {
                        let size = self.value_size(referent);
                        if size > 0 {
                            self.push_instr(Instr::Blit(val, ptr, size));
                        }
                    }
                    _ => self.push_instr(Instr::FieldStore(ptr, val)),
                }
            }
            "+!" => {
                let val = self.stack.pop().expect("+!: addend");
                let ptr = self.stack.pop().expect("+!: reference");
                let referent = self.referent_of(ptr);
                let cur = self.fresh_value(referent);
                self.push_instr(Instr::FieldLoad(cur, ptr));
                let sum = self.fresh_value(referent);
                self.push_instr(Instr::Bin(sum, BinOp::Add, cur, val));
                self.push_instr(Instr::FieldStore(ptr, sum));
            }
            _ => unreachable!("lower_access_word only handles @/!/+!"),
        }
    }

    /// Alloc a fresh frame slot for struct `id`'s aggregate and yield it as a
    /// `Struct`-typed value (a pointer to the storage).
    pub(super) fn alloc_struct(&mut self, id: StructId) -> Value {
        let (size, align) = {
            let l = &self.structs.layouts[id.index()];
            (l.size, l.align)
        };
        let v = self.fresh_value(IrType::Struct(id));
        self.push_alloc(Instr::Alloc(v, size, align));
        v
    }

    /// Alloc a fresh frame slot for enum `id`'s tagged aggregate and yield it
    /// as an `Enum`-typed value (a pointer to the storage), mirroring
    /// `alloc_struct`.
    pub(super) fn alloc_enum(&mut self, id: EnumId) -> Value {
        let (size, align) = {
            let l = &self.enums.layouts[id.index()];
            (l.size, l.align)
        };
        let v = self.fresh_value(IrType::Enum(id));
        self.push_alloc(Instr::Alloc(v, size, align));
        v
    }

    /// Alloc a fresh frame slot for array `id`'s inline aggregate and yield it
    /// as an `Array`-typed value (a pointer to the storage), mirroring
    /// `alloc_struct`/`alloc_enum`.
    pub(super) fn alloc_array(&mut self, id: ArrayId) -> Value {
        let (size, align) = {
            let l = &self.arrays.layouts[id.index()];
            (l.size, l.align)
        };
        let v = self.fresh_value(IrType::Array(id));
        self.push_alloc(Instr::Alloc(v, size, align));
        v
    }

    /// The `(stride, element type, count)` of array `id`, copied out of the
    /// layout registry so the caller can then emit against `&mut self`.
    fn array_parts(&self, id: ArrayId) -> (u32, IrType, u32) {
        let l = &self.arrays.layouts[id.index()];
        (l.stride, l.elem, l.count)
    }

    /// The `ArrayId` whose layout has element `elem` and `count`: `fill`'s
    /// target shape, already interned by the checker (R10), found by structural
    /// match on the combined registry.
    fn array_id_of(&self, elem: IrType, count: u32) -> ArrayId {
        let idx = self
            .arrays
            .layouts
            .iter()
            .position(|l| l.elem == elem && l.count == count)
            .expect("fill's array shape is interned by the checker");
        ArrayId::from_index(idx)
    }

    /// The exact byte size of a value of `ty` (an aggregate's whole size, a
    /// scalar's width) — the blit length for a `fill` aggregate element.
    pub(super) fn value_size(&self, ty: IrType) -> u32 {
        match ty {
            IrType::Struct(id) => self.structs.layouts[id.index()].size,
            IrType::Enum(id) => self.enums.layouts[id.index()].size,
            IrType::Array(id) => self.arrays.layouts[id.index()].size,
            IrType::Quotation(_) => quotation_layout(WORD_WIDTH).size,
            other => scalar_size_align(other).0,
        }
    }

    /// `dst = base + index*stride`, a `Ptr` (R17): a `FieldLoad`/`FieldStore`
    /// through `dst` always follows: a reference projection (`&>`), the array
    /// constructor's byte-granular zero-init store, or `fill`'s counted
    /// element store (both slice 6h).
    fn elem_addr(&mut self, base: Value, index: Value, stride: u32) -> Value {
        let dst = self.fresh_value(IrType::Ptr);
        self.push_instr(Instr::ElemAddr(dst, base, index, stride as i64));
        dst
    }

    /// Store `val` (of element type `elem`) at element place `fptr`: a
    /// width-exact scalar `FieldStore`, or an aggregate `Blit` of the whole
    /// element. `fill`'s counted store loop is the only caller (slice 6h; the
    /// `Blit` arm accepts the loop's runtime `elem_addr` destination).
    fn store_elem(&mut self, fptr: Value, val: Value, elem: IrType) {
        match elem {
            // Slice 9 (R1): a zero-payload-enum element is a bare scalar.
            IrType::Enum(id) if self.enums.layouts[id.index()].is_scalar => {
                self.push_instr(Instr::FieldStore(fptr, val));
            }
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) | IrType::Quotation(_) => {
                let size = self.value_size(elem);
                if size > 0 {
                    self.push_instr(Instr::Blit(val, fptr, size));
                }
            }
            _ => self.push_instr(Instr::FieldStore(fptr, val)),
        }
    }

    /// Lower an array word inline: `fill` = one alloc + a counted store loop
    /// (slice 6h; was N unrolled stores, which was QBE-quadratic on one large
    /// straight-line block); `len` = a constant `usize` from the layout,
    /// non-consuming.
    fn lower_array_word(&mut self, name: &str) {
        match name {
            // Slice 6h (D4): one `Instr::Alloc` plus a `begin_loop`/
            // `finalize_loop`-bounded loop storing the `Copy` seed `n` times
            // via `elem_addr` (a runtime index) + `store_elem`, replacing the
            // N unrolled compile-time `field_ptr` stores. Code size stays O(1)
            // in `n`; the seed is replicated per iteration (never minted from
            // zeroed memory), so this arm keeps `fill`'s looser element gate.
            "fill" => {
                let count_v = self.stack.pop().expect("fill: count");
                let n = *self
                    .const_vals
                    .get(&count_v)
                    .expect("fill's count is a checked literal") as u32;
                let elem_v = self.stack.pop().expect("fill: element");
                let elem = self.value_type(elem_v);
                let id = self.array_id_of(elem, n);
                let (stride, _, _) = self.array_parts(id);

                // Save/restore the enclosing loop state so this loop composes
                // with a later `Alloc` or loop (see `save_loop_state`).
                let saved_loop_state = self.save_loop_state();
                // The destination reaches the loop body and the exit block by
                // dominance; it must not be carried, or the back-edge staging
                // blit would copy a stale snapshot over each iteration's store.
                let dst = self.alloc_array(id);

                let seed = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(seed, 0));
                self.const_vals.insert(seed, 0);
                // Carry only the induction index, with `stage_aggregates =
                // false`.
                let outs = self.begin_loop(&[seed], false);
                let index_phi = outs[0];

                // Header (current after `begin_loop`): loop while index < n.
                let bound = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(bound, n as i64));
                self.const_vals.insert(bound, n as i64);
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Lt, index_phi, bound));
                let body_block = self.fresh_block();
                let exit_block = self.fresh_block();
                self.seal_block(Terminator::Jnz(cmp, body_block, exit_block));

                // Body: store the seed at element `dst + index*stride`.
                // `elem_addr`'s runtime index replaces the unrolled
                // compile-time `field_ptr`; `store_elem`'s `Blit` arm accepts a
                // runtime destination, so an aggregate seed works too.
                self.start_block(body_block);
                self.terminated = false;
                let fptr = self.elem_addr(dst, index_phi, stride);
                self.store_elem(fptr, elem_v, elem);

                // Back-edge: index + 1.
                let one = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(one, 1));
                self.const_vals.insert(one, 1);
                let index_next = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Bin(index_next, BinOp::Add, index_phi, one));
                self.back_edges.push((self.cur_id, vec![index_next]));
                self.seal_block(Terminator::Jmp(self.header.expect("fill loop header")));

                self.finalize_loop();

                // Exit: the filled array is the result. The fixed loop body
                // never terminates, so `terminated` is already false here; the
                // reset keeps this arm identical to the loop template so terms
                // after `fill` are not dropped if the body ever gains one.
                self.start_block(exit_block);
                self.terminated = false;
                self.stack.push(dst);

                self.restore_loop_state(saved_loop_state);
            }
            "len" => {
                // Non-consuming (R10): the array stays; the constant folds in.
                let array = *self.stack.last().expect("len: array");
                let id = match self.value_type(array) {
                    IrType::Array(id) => id,
                    _ => unreachable!("checked: len's operand is an array"),
                };
                let (_, _, count) = self.array_parts(id);
                let v = self.fresh_value(IrType::Usize);
                self.push_instr(Instr::Const(v, count as i64));
                self.stack.push(v);
            }
            _ => unreachable!("lower_array_word only handles fill/len"),
        }
    }

    /// The `OwnedCellId` whose payload shape is `payload`: `^`'s target shape,
    /// already interned by the checker, found by structural match on the
    /// combined registry, mirroring `array_id_of`.
    fn cell_id_of(&self, payload: IrType) -> OwnedCellId {
        let idx = self
            .cells
            .payload
            .iter()
            .position(|&p| p == payload)
            .expect("^'s payload shape is interned by the checker");
        OwnedCellId::from_index(idx)
    }

    /// Alloc a fresh frame slot for aggregate `ty` (a `Struct`/`Enum`/`Array`),
    /// dispatching to the matching per-kind helper. Shared by a cell's
    /// unwrap/peek, which must never alias the cell's own storage.
    fn alloc_aggregate(&mut self, ty: IrType) -> Value {
        match ty {
            IrType::Struct(id) => self.alloc_struct(id),
            IrType::Enum(id) => self.alloc_enum(id),
            IrType::Array(id) => self.alloc_array(id),
            IrType::Quotation(sig) => {
                let layout = quotation_layout(WORD_WIDTH);
                let v = self.fresh_value(IrType::Quotation(sig));
                self.push_alloc(Instr::Alloc(v, layout.size, layout.align));
                v
            }
            _ => unreachable!("alloc_aggregate: not an aggregate IrType"),
        }
    }

    /// Never alias the cell: an aggregate payload gets a fresh frame slot and
    /// a `Blit` out, so a later `free` never leaves the caller holding a
    /// dangling interior pointer.
    pub(super) fn load_owned_payload(&mut self, cell_ptr: Value, payload_ty: IrType) -> Value {
        match payload_ty {
            // Slice 9 (R1): a zero-payload-enum payload is a bare scalar.
            IrType::Enum(id) if self.enums.layouts[id.index()].is_scalar => {
                let v = self.fresh_value(payload_ty);
                self.push_instr(Instr::FieldLoad(v, cell_ptr));
                v
            }
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                let dst = self.alloc_aggregate(payload_ty);
                let size = self.value_size(payload_ty);
                if size > 0 {
                    self.push_instr(Instr::Blit(cell_ptr, dst, size));
                }
                dst
            }
            _ => {
                let v = self.fresh_value(payload_ty);
                self.push_instr(Instr::FieldLoad(v, cell_ptr));
                v
            }
        }
    }

    /// Drop every linear field of one aggregate level (a struct's own fields,
    /// or an enum variant's, offsets already adjusted) except `skip`, the
    /// field the disposal path continues through. The continuing field
    /// is read after every other read of this level, so it is skipped here
    /// rather than dropped in place.
    pub(super) fn drop_level_fields(
        &mut self,
        base: Value,
        fields: &[FieldLayout],
        skip: Option<usize>,
    ) {
        for (fi, field) in fields.iter().enumerate() {
            if Some(fi) != skip && field_is_linear(field.ty, self.structs, self.enums, self.arrays)
            {
                let v = self.field_value(base, *field);
                self.emit_drop(v);
            }
        }
    }

    /// One `Unwrap` step: copy the cell's payload out and free the cell.
    ///
    /// **Every read of data held in the payload's frame slot must already be
    /// emitted.** `push_alloc` hoists the copy-out's `Alloc` into the entry
    /// block, so one slot per step site is reused by every iteration and the
    /// copy-out blits the next value over the memory the current one occupies.
    /// A field load, tag read or sibling drop emitted after this call
    /// would read the wrong value, and would do so with the alloc/free trace
    /// still perfectly balanced. A scalar payload (the inner step of `^^Self`)
    /// takes `load_owned_payload`'s plain-`FieldLoad` branch, has no slot, and
    /// so has no ordering hazard of its own.
    fn emit_unwrap(&mut self, cell_ptr: Value, cell: OwnedCellId) -> Value {
        let payload_ty = self.cells.payload[cell.index()];
        let next = self.load_owned_payload(cell_ptr, payload_ty);
        let size = self.value_size(payload_ty);
        let size_v = self.fresh_value(IrType::I64);
        self.push_instr(Instr::Const(size_v, size as i64));
        self.push_instr(Instr::Call(
            None,
            FREE_SYMBOL.to_string(),
            vec![cell_ptr, size_v],
        ));
        next
    }

    /// Emit the rest of a fused destructor loop's iteration from `cur`, whose
    /// own `IrType` names the level the next step reads. An empty `steps` is
    /// the end of one full trip around the path: `cur` is a fresh value
    /// of the loop's own type, so it back-edges to the header.
    pub(super) fn emit_path_steps(&mut self, cur: Value, steps: &[PathStep]) {
        let Some(first) = steps.first() else {
            self.back_edges.push((self.cur_id, vec![cur]));
            self.seal_block(Terminator::Jmp(self.header.expect("loop header")));
            self.terminated = true;
            return;
        };
        match *first {
            // `cur` is itself the cell (the inner step of `^^Self`).
            PathStep::Unwrap { field: None, cell } => {
                let next = self.emit_unwrap(cur, cell);
                self.emit_path_steps(next, &steps[1..]);
            }
            PathStep::Branch {
                enum_id,
                ref variants,
            } => self.emit_branch(cur, enum_id, variants),
            PathStep::Project { .. } | PathStep::Unwrap { field: Some(_), .. } => {
                // Only a struct level is reached with a field step still
                // pending: an enum expands into a `Branch`, and a cell into a
                // fieldless `Unwrap`.
                let IrType::Struct(id) = self.value_type(cur) else {
                    unreachable!("a field step reads a struct level")
                };
                let fields = self.structs.layouts[id.index()].fields.clone();
                self.emit_field_level(cur, &fields, steps);
            }
        }
    }

    /// Emit `steps` from the aggregate level (`base`, `fields`) their first
    /// step reads: drop that level's other fields, then take the
    /// continuing field byval (`Project`) or through its cell (`Unwrap`).
    pub(super) fn emit_field_level(
        &mut self,
        base: Value,
        fields: &[FieldLayout],
        steps: &[PathStep],
    ) {
        let (first, rest) = steps.split_first().expect("a level's path is non-empty");
        let (fi, cell) = match *first {
            PathStep::Project { field } => (field, None),
            PathStep::Unwrap {
                field: Some(field),
                cell,
            } => (field, Some(cell)),
            _ => unreachable!("a level's path starts at one of its own fields"),
        };
        self.drop_level_fields(base, fields, Some(fi));
        let field = self.field_value(base, fields[fi]);
        let next = match cell {
            Some(cell) => self.emit_unwrap(field, cell),
            None => field,
        };
        self.emit_path_steps(next, rest);
    }

    /// Dispatch on `node`'s tag and emit each variant's own continuation: a
    /// variant that does not continue toward `Self` drops its fields and
    /// leaves the loop, one that does walks its own steps and back-edges.
    /// More than one variant may continue, and each back-edges
    /// independently.
    ///
    /// Every variant block resets `terminated` right after `start_block`, so
    /// the trailing seal fires for a base case and is skipped for a block a
    /// back-edge or a nested dispatch already sealed. All arms end
    /// sealed and nothing follows a dispatch in the same sequence, so the
    /// whole `Branch` reports itself terminated to its own caller.
    pub(super) fn emit_branch(
        &mut self,
        node: Value,
        id: EnumId,
        variants: &[Option<Vec<PathStep>>],
    ) {
        let payload_offset = self.enums.layouts[id.index()].payload_offset;
        let layouts = self.enums.layouts[id.index()].variants.clone();
        let blocks = self.dispatch_on_tag(node, id, false);
        for (vi, &block) in blocks.iter().enumerate() {
            self.start_block(block);
            self.terminated = false;
            let fields: Vec<FieldLayout> = layouts[vi]
                .fields
                .iter()
                .map(|field| FieldLayout {
                    offset: payload_offset + field.offset,
                    ..*field
                })
                .collect();
            match &variants[vi] {
                Some(steps) => self.emit_field_level(node, &fields, steps),
                None => self.drop_level_fields(node, &fields, None),
            }
            if !self.terminated {
                self.seal_block(Terminator::Ret(None));
            }
        }
        self.terminated = true;
    }

    /// Store `val` (of `payload_ty`) into the cell at `cell_ptr`: the mirror
    /// of `load_owned_payload`. A scalar payload is a width-exact
    /// `FieldStore`; an aggregate is a `Blit` from its frame slot; a
    /// zero-sized payload writes nothing.
    fn store_owned_payload(&mut self, cell_ptr: Value, val: Value, payload_ty: IrType) {
        match payload_ty {
            // Slice 9 (R1): a zero-payload-enum payload is a bare scalar.
            IrType::Enum(id) if self.enums.layouts[id.index()].is_scalar => {
                self.push_instr(Instr::FieldStore(cell_ptr, val));
            }
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) => {
                let size = self.value_size(payload_ty);
                if size > 0 {
                    self.push_instr(Instr::Blit(val, cell_ptr, size));
                }
            }
            _ => self.push_instr(Instr::FieldStore(cell_ptr, val)),
        }
    }

    /// `^>` materialises the payload before freeing the cell, so the freed
    /// pointer is never handed to the stack.
    fn lower_owned_cell_word(&mut self, name: &str) {
        match name {
            "^" => {
                let payload_val = self.stack.pop().expect("^: payload");
                let payload_ty = self.value_type(payload_val);
                let id = self.cell_id_of(payload_ty);
                let size = self.value_size(payload_ty);
                let size_v = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(size_v, size as i64));
                let ptr = self.fresh_value(IrType::OwnedCell(id));
                self.push_instr(Instr::Call(
                    Some(ptr),
                    ALLOC_SYMBOL.to_string(),
                    vec![size_v],
                ));
                self.store_owned_payload(ptr, payload_val, payload_ty);
                self.stack.push(ptr);
            }
            "^>" => {
                let cell = self.stack.pop().expect("^>: cell");
                let id = match self.value_type(cell) {
                    IrType::OwnedCell(id) => id,
                    _ => unreachable!("checked: ^>'s operand is a cell"),
                };
                let payload_ty = self.cells.payload[id.index()];
                let val = self.load_owned_payload(cell, payload_ty);
                let size = self.value_size(payload_ty);
                let size_v = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(size_v, size as i64));
                self.push_instr(Instr::Call(
                    None,
                    FREE_SYMBOL.to_string(),
                    vec![cell, size_v],
                ));
                self.stack.push(val);
            }
            "^|>" => {
                // Non-consuming: the cell stays on the stack, the payload
                // copy is pushed atop it.
                let cell = *self.stack.last().expect("^|>: cell");
                let id = match self.value_type(cell) {
                    IrType::OwnedCell(id) => id,
                    _ => unreachable!("checked: ^|>'s operand is a cell"),
                };
                let payload_ty = self.cells.payload[id.index()];
                let val = self.load_owned_payload(cell, payload_ty);
                self.stack.push(val);
            }
            _ => unreachable!("lower_owned_cell_word only handles ^/^>/^|>"),
        }
    }

    /// Emit the runtime bounds guard for a dynamic array index (R19/D6): an
    /// `index < N` compare jumps to the continuation, otherwise a trap block
    /// calls the out-of-bounds helper (a located len+index message to stderr,
    /// then a nonzero exit) so an out-of-range access aborts rather than
    /// corrupting. A checked compile-time literal index (X4, R11) already had
    /// its bounds verified, so it skips the guard entirely and stays
    /// trap-free.
    fn bounds_check(&mut self, index: Value, count: u32, line: u32) {
        if self.const_vals.contains_key(&index) {
            return;
        }
        let n = self.fresh_value(IrType::Usize);
        self.push_instr(Instr::Const(n, i64::from(count)));
        let cond = self.fresh_value(IrType::Bool);
        self.push_instr(Instr::Cmp(cond, CmpOp::Lt, index, n));
        let ok = self.fresh_block();
        let trap = self.fresh_block();
        self.seal_block(Terminator::Jnz(cond, ok, trap));

        // The trap block never falls through: the helper exits, so the `Jmp`
        // to `ok` is an unreachable CFG edge that keeps the block validly
        // terminated regardless of the enclosing word's return type.
        self.start_block(trap);
        let line_v = self.fresh_value(IrType::Usize);
        self.push_instr(Instr::Const(line_v, i64::from(line)));
        let len_v = self.fresh_value(IrType::Usize);
        self.push_instr(Instr::Const(len_v, i64::from(count)));
        self.push_instr(Instr::Call(
            None,
            OOB_TRAP_SYMBOL.to_string(),
            vec![line_v, index, len_v],
        ));
        self.seal_block(Terminator::Jmp(ok));

        self.start_block(ok);
    }

    /// A `Ptr`-typed value for `base + offset` (a scalar field's address).
    pub(super) fn field_ptr(&mut self, base: Value, offset: u32) -> Value {
        let p = self.fresh_value(IrType::Ptr);
        self.push_instr(Instr::PtrOffset(p, base, offset as i64));
        p
    }

    /// A nested-aggregate field's value: its interior address, typed as the
    /// inner struct/enum. No copy — the owning aggregate is consumed by the
    /// getter/destructure/clause, so aliasing its storage is sound; a later
    /// `dup` or word-return copies the bytes.
    fn field_aggregate_value(&mut self, base: Value, offset: u32, inner: IrType) -> Value {
        let v = self.fresh_value(inner);
        self.push_instr(Instr::PtrOffset(v, base, offset as i64));
        v
    }

    /// Store `val` into field `field` at `fptr`: a width-exact scalar store, or
    /// an aggregate blit for a nested struct/enum/quotation field. A quotation
    /// field is the constructor/setter materialization boundary (R7/R9): a
    /// phantom `val` becomes a real `(code, env)` aggregate before the blit.
    fn store_field(&mut self, fptr: Value, val: Value, field: FieldLayout) {
        let val = self.materialize_if_phantom(val, field.ty);
        match field.ty {
            // Slice 9 (R1): a zero-payload-enum field is a bare scalar, not
            // an aggregate to blit -- a width-exact `FieldStore`, like `Bool`
            // was before this migration.
            IrType::Enum(id) if self.enums.layouts[id.index()].is_scalar => {
                self.push_instr(Instr::FieldStore(fptr, val));
            }
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) | IrType::Quotation(_) => {
                if field.size > 0 {
                    self.push_instr(Instr::Blit(val, fptr, field.size));
                }
            }
            _ => self.push_instr(Instr::FieldStore(fptr, val)),
        }
    }

    /// Field `field` of aggregate `base` as a value: a width-exact scalar load,
    /// or the interior pointer as a nested struct/enum/quotation value (the
    /// getter reads a stored quotation back out as a runtime value).
    fn field_value(&mut self, base: Value, field: FieldLayout) -> Value {
        match field.ty {
            // Slice 9 (R1): a zero-payload-enum field loads as a scalar, not
            // as the interior-pointer aggregate view below.
            IrType::Enum(id) if self.enums.layouts[id.index()].is_scalar => {
                let fptr = self.field_ptr(base, field.offset);
                let v = self.fresh_value(field.ty);
                self.push_instr(Instr::FieldLoad(v, fptr));
                v
            }
            IrType::Struct(_) | IrType::Enum(_) | IrType::Array(_) | IrType::Quotation(_) => {
                self.field_aggregate_value(base, field.offset, field.ty)
            }
            _ => {
                let fptr = self.field_ptr(base, field.offset);
                let v = self.fresh_value(field.ty);
                self.push_instr(Instr::FieldLoad(v, fptr));
                v
            }
        }
    }

    fn load_field_onto_stack(&mut self, base: Value, field: FieldLayout) {
        let v = self.field_value(base, field);
        self.stack.push(v);
    }

    /// Slice 7a (R9/D5): at a materialization boundary, turn a phantom
    /// quotation `Value` (a `call`/`times`-splice marker that defines no
    /// runtime bytes) into a real `(code, env)` aggregate of signature `sig`.
    /// A `Value` already carrying an aggregate (a getter/`@`/word-return
    /// result, or another boundary's output) is returned untouched -- only a
    /// phantom recorded in `quot_bodies` is built here, so a boundary can call
    /// this unconditionally on whatever it is about to store/return.
    pub(super) fn materialize_if_phantom(&mut self, val: Value, ty: IrType) -> Value {
        match ty {
            IrType::Quotation(sig) if self.quot_bodies.contains_key(&val) => {
                self.materialize_quot_value(val, sig)
            }
            _ => val,
        }
    }

    /// Slice 7a/7b (R9/R16): build the runtime `(code, env)` aggregate for
    /// phantom quotation `phantom`, recording the callee `IrFunc` to mint
    /// (deduped by symbol). `code` gets the callee's address (`FuncAddr`). The
    /// `env` slot is 7a's null pointer for a non-capturing literal (byte for
    /// byte), else the one captured local's live value snapshotted from the
    /// enclosing frame -- a scalar's value, or a reference's pointer. The
    /// symbol is `{enclosing}__quot{id}`, unique because word names are unique
    /// and `QuotId` is per-function.
    fn materialize_quot_value(&mut self, phantom: Value, sig: QuotSigId) -> Value {
        let id = self.quot_bodies[&phantom];
        let symbol = format!("{}__quot{}", self.cur_word_name, id.0);
        // The captured locals (sorted by name), resolved against the live frame
        // at this boundary.
        let captures = self.quotation_captures(id);
        if !self.materialized.iter().any(|m| m.symbol == symbol) {
            let env_caps = captures
                .iter()
                .map(|(name, value)| EnvCapture {
                    name: name.clone(),
                    ty: self.value_type(*value),
                    referent: self.ref_inner.get(value).copied(),
                })
                .collect();
            self.materialized.push(MaterializedQuot {
                symbol: symbol.clone(),
                effect: sig.0,
                body: self.quot_defs[id.0].clone(),
                captures: env_caps,
            });
        }
        let layout = quotation_layout(WORD_WIDTH);
        let dst = self.fresh_value(IrType::Quotation(sig));
        self.push_alloc(Instr::Alloc(dst, layout.size, layout.align));
        let code = self.fresh_value(IrType::Code);
        self.push_instr(Instr::FuncAddr(code, symbol));
        let code_ptr = self.field_ptr(dst, layout.code_offset);
        self.push_instr(Instr::FieldStore(code_ptr, code));
        let env = self.build_env(&captures);
        let env_ptr = self.field_ptr(dst, layout.env_offset);
        self.push_instr(Instr::FieldStore(env_ptr, env));
        dst
    }

    /// 7b/R16: the `env` word for a materialized closure. Q2a's ladder: zero
    /// captures -> a null pointer (byte for byte the 7a shape); exactly one ->
    /// the capture's live value inline (a scalar's value or a reference's
    /// pointer, one word); two or more -> a stack-allocated bundle of one word
    /// per capture, each `FieldStore`d in the sorted order the body reads, with
    /// the `env` word pointing at it.
    fn build_env(&mut self, captures: &[(String, Value)]) -> Value {
        match captures {
            [] => {
                let z = self.fresh_value(IrType::Ptr);
                self.push_instr(Instr::Const(z, 0));
                z
            }
            [(_, value)] => *value,
            many => {
                let size = many.len() as u32 * WORD_WIDTH;
                let bundle = self.fresh_value(IrType::Ptr);
                self.push_alloc(Instr::Alloc(bundle, size, WORD_WIDTH));
                for (i, (_, value)) in many.iter().enumerate() {
                    let slot = self.field_ptr(bundle, i as u32 * WORD_WIDTH);
                    self.push_instr(Instr::FieldStore(slot, *value));
                }
                bundle
            }
        }
    }

    /// 7b/R16: the enclosing locals a quotation body captures, as `(name, live
    /// value)` pairs resolved against `self.locals` at the materialization
    /// boundary, sorted by name so the build order matches the body's read
    /// order. A free global word contributes nothing (it is not a local).
    fn quotation_captures(&self, id: QuotId) -> Vec<(String, Value)> {
        let body = &self.quot_defs[id.0];
        let mut names = HashSet::new();
        free_locals_into(body, &mut HashSet::new(), &mut names);
        let mut names: Vec<String> = names.into_iter().collect();
        names.sort();
        names
            .into_iter()
            .filter_map(|name| {
                self.locals
                    .iter()
                    .rev()
                    .find(|(n, _)| *n == name)
                    .map(|(n, value)| (n.clone(), *value))
            })
            .collect()
    }

    /// Slice 7a (R11): at a branch join, materialize each arm's escaping
    /// quotation phantom into a runtime `(code, env)` value so the join `Phi`s
    /// two real aggregates. A slot needs materializing when the two arms leave
    /// *different* phantoms (`t != e`, both in `quot_bodies`); a shared phantom
    /// (`t == e`, a literal bound before the `if`) is forwarded untouched and
    /// spliced past the join. The target signature is the declared output at
    /// that slot (`cur_outputs[i]`) when the join is a word-body tail, else the
    /// referent of the `&!Quotation` store target directly below it
    /// (`ref_inner`, an in-frame `&!ref if..end !`) -- the two contexts the
    /// checker erases a differing join in. Materialization is appended to each
    /// arm's already-sealed block
    /// (via `reopen_block`), since the differing-pair decision needs both
    /// arms' stacks, known only after both are lowered.
    fn materialize_join_quotations(
        &mut self,
        then_pred: BlockId,
        mut then_stack: Vec<Value>,
        else_pred: BlockId,
        mut else_stack: Vec<Value>,
    ) -> (Vec<Value>, Vec<Value>) {
        let mut jobs: Vec<(usize, QuotSigId)> = Vec::new();
        for (i, (t, e)) in then_stack.iter().zip(&else_stack).enumerate() {
            if t != e && self.quot_bodies.contains_key(t) && self.quot_bodies.contains_key(e) {
                let target = match self.cur_outputs.get(i) {
                    Some(&IrType::Quotation(sig)) => Some(sig),
                    _ => match i
                        .checked_sub(1)
                        .and_then(|b| self.ref_inner.get(&then_stack[b]))
                    {
                        Some(&IrType::Quotation(sig)) => Some(sig),
                        _ => None,
                    },
                };
                match target {
                    Some(sig) => jobs.push((i, sig)),
                    None => unreachable!(
                        "a differing quotation join slot maps to a declared quotation output or an in-frame store target"
                    ),
                }
            }
        }
        if jobs.is_empty() {
            return (then_stack, else_stack);
        }
        let (then_pos, then_term) = self.reopen_block(then_pred);
        for &(i, sig) in &jobs {
            then_stack[i] = self.materialize_quot_value(then_stack[i], sig);
        }
        self.reseal_block_at(then_pos, then_term);
        let (else_pos, else_term) = self.reopen_block(else_pred);
        for &(i, sig) in &jobs {
            else_stack[i] = self.materialize_quot_value(else_stack[i], sig);
        }
        self.reseal_block_at(else_pos, else_term);
        (then_stack, else_stack)
    }

    /// Slice 7a (R10): lower `call` on a materialized quotation value whose
    /// identity the checker could not resolve to a literal. Load the value's
    /// `code` slot and call it indirectly -- its inputs the stack below the
    /// value, its outputs pushed back (a multi-output quotation returns a
    /// bundle, unpacked, exactly as an ordinary word call). The runtime-
    /// dispatch counterpart to `call`-of-literal fusion (D5); no splice.
    fn lower_indirect_call(&mut self, v: Value) {
        let IrType::Quotation(sig) = self.value_type(v) else {
            unreachable!("a non-literal `call` operand is a materialized quotation value")
        };
        let eff = sig.0;
        let split = self.stack.len() - eff.inputs.len();
        let mut args = self.stack.split_off(split);
        let layout = quotation_layout(WORD_WIDTH);
        let code_ptr = self.field_ptr(v, layout.code_offset);
        let code = self.fresh_value(IrType::Code);
        self.push_instr(Instr::FieldLoad(code, code_ptr));
        // 7b/R17: pass the env word as the trailing argument. Every materialized
        // body takes an env param (null for a non-capturing one), so this is
        // uniform without knowing which callee a merged/stored value holds.
        let env_ptr = self.field_ptr(v, layout.env_offset);
        let env = self.fresh_value(IrType::Ptr);
        self.push_instr(Instr::FieldLoad(env, env_ptr));
        args.push(env);
        let outs: Vec<TypedSlot> = eff
            .outputs
            .iter()
            .map(|&ty| TypedSlot { name: None, ty })
            .collect();
        let bundle = bundle_of(&outs, self.structs);
        let ret_ty = word_ret_ty(&outs, self.structs);
        let ret = if eff.outputs.len() == 1 || bundle.is_some() {
            Some(self.fresh_value(ret_ty.unwrap_or(IrType::I64)))
        } else {
            None
        };
        self.push_instr(Instr::CallIndirect(ret, code, args));
        if let Some(r) = ret {
            self.stack.push(r);
        }
        if let Some(id) = bundle {
            self.unpack_bundle(id);
        }
    }

    /// Dispatch on `scrutinee`'s runtime tag (enum `id`): seal a compare chain
    /// (`n == 1` short-circuits to a bare `Jmp`; otherwise load the tag once
    /// and `Cmp`/`Jnz` variant-by-variant, the last compare's false edge
    /// falling straight through to the final variant with no default/trap
    /// block) and return one freshly allocated, not-yet-started block per
    /// variant in declaration order. Shared by `lower_clauses` (a clause
    /// word's scrutinee) and `synthesize_enum_destructor` (the same shape,
    /// only what each variant block does next differs).
    ///
    /// `scrutinee_is_value` (Slice 9): whether `scrutinee` already *is* the
    /// discriminant (a zero-payload enum's bare scalar value) rather than a
    /// pointer to tagged storage needing a `FieldLoad`. A linear enum is
    /// never scalar (zero fields implies `is_linear == false`), so
    /// `synthesize_enum_destructor`'s call is always the pointer case; a
    /// non-reference clause scrutinee of a scalar enum is the value case.
    fn dispatch_on_tag(
        &mut self,
        scrutinee: Value,
        id: EnumId,
        scrutinee_is_value: bool,
    ) -> Vec<BlockId> {
        let (tag_ty, tag_offset, n) = {
            let l = &self.enums.layouts[id.index()];
            (l.tag_ty, l.tag_offset, l.variants.len())
        };
        let variant_ids: Vec<BlockId> = (0..n).map(|_| self.fresh_block()).collect();
        if n == 1 {
            self.seal_block(Terminator::Jmp(variant_ids[0]));
        } else {
            let tag = if scrutinee_is_value {
                scrutinee
            } else {
                let tag = self.fresh_value(tag_ty);
                let tag_ptr = self.field_ptr(scrutinee, tag_offset);
                self.push_instr(Instr::FieldLoad(tag, tag_ptr));
                tag
            };
            for vi in 0..n - 1 {
                let idx_val = self.fresh_value(tag_ty);
                self.push_instr(Instr::Const(idx_val, vi as i64));
                let c = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(c, CmpOp::Eq, tag, idx_val));
                let false_target = if vi == n - 2 {
                    variant_ids[n - 1]
                } else {
                    self.fresh_block()
                };
                self.seal_block(Terminator::Jnz(c, variant_ids[vi], false_target));
                if vi < n - 2 {
                    self.start_block(false_target);
                }
            }
        }
        variant_ids
    }

    /// R5/R12/R16: the universal disposal primitive. On a linear value (a
    /// struct/enum whose `is_linear` is set, or an owning cell) this is a
    /// plain `Call` to the (builtin or synthesized) destructor; a `Copy`
    /// value is discarded with no runtime effect. Shared by `drop`, `S>fi`'s
    /// drop-the-rest, `S<fi`'s drop-on-overwrite, and the synthesized
    /// struct/enum destructors themselves, so "how a value is disposed" lives
    /// in one place.
    pub(super) fn emit_drop(&mut self, v: Value) {
        match self.value_type(v) {
            // A cell always frees on drop, regardless of its payload's own
            // linearity: the synthesized destructor drops a linear payload
            // first.
            IrType::OwnedCell(id) => {
                let symbol = cell_drop_symbol(id, self.cells.drop_generations[id.index()]);
                self.push_instr(Instr::Call(None, symbol, vec![v]));
            }
            IrType::Struct(id) if self.structs.layouts[id.index()].is_linear => {
                let symbol =
                    struct_drop_symbol(id, self.structs.layouts[id.index()].drop_generation);
                self.push_instr(Instr::Call(None, symbol, vec![v]));
            }
            IrType::Enum(id) if self.enums.layouts[id.index()].is_linear => {
                let symbol = enum_drop_symbol(id, self.enums.layouts[id.index()].drop_generation);
                self.push_instr(Instr::Call(None, symbol, vec![v]));
            }
            IrType::Array(id) if self.arrays.layouts[id.index()].is_linear => unreachable!(
                "checked: a linear array element is rejected wherever an array type is named"
            ),
            _ => {}
        }
    }

    /// R10, callee side: pop the top `n` stack values into a fresh bundle of
    /// `id` (deepest output first, matching the field order the checker
    /// interned) and yield it as the word's single returned value. Literally
    /// the struct constructor, which is the point: the bundle is the struct
    /// users hand-wrote before this ABI existed.
    pub(super) fn pack_bundle(&mut self, id: StructId) -> Value {
        self.lower_struct_word(StructWord::Construct(id));
        self.stack.pop().expect("pack: the bundle just constructed")
    }

    /// R11, caller side: replace the returned bundle on the stack with its
    /// fields, deepest first — the exact reverse of `pack_bundle`, through the
    /// same destructure a generated `S>` uses, so a linear field is moved out
    /// of the shell exactly as `S>` moves one.
    fn unpack_bundle(&mut self, id: StructId) {
        self.lower_struct_word(StructWord::Destructure(id));
    }

    /// R14/R11: lower a call to a polymorphic word through its per-call-site
    /// `CallInst`. The mangled symbol (not `(self.resolve)(name)`), the
    /// per-θ output arity, and the bundle come straight from the table, so
    /// two instantiations of one word reach two distinct symbols and two
    /// distinct return shapes even though `env`/`resolve` are name-keyed. The
    /// input arity is name-constant across θ and read from `poly_arities`; the
    /// row prefix, if any, stays on the stack below the popped args (S2). The
    /// bundle unpack is the same pack/unpack path a monomorphic multi-output
    /// call takes (R10/R11), so a row-variable-expanded count lowers
    /// identically to a fixed multi-output word — D4's one mechanism.
    fn lower_poly_call(&mut self, inst: &CallInst) {
        let in_arity = self.poly_arities[&inst.callee];
        let split = self.stack.len() - in_arity;
        let args = self.stack.split_off(split);
        let ret = if inst.out_arity == 1 || inst.bundle.is_some() {
            let ret_ty = match inst.bundle {
                Some(id) => IrType::Struct(id),
                None => ir_type_of(
                    *inst
                        .output_types
                        .first()
                        .expect("out_arity == 1 guarantees a single output type"),
                ),
            };
            Some(self.fresh_value(ret_ty))
        } else {
            None
        };
        self.push_instr(Instr::Call(ret, inst.symbol.clone(), args));
        if let Some(v) = ret {
            self.stack.push(v);
        }
        if let Some(id) = inst.bundle {
            self.unpack_bundle(id);
        }
    }

    /// Lower a generated struct word inline, first field deepest.
    fn lower_struct_word(&mut self, sw: StructWord) {
        match sw {
            StructWord::Construct(id) => {
                let n = self.structs.layouts[id.index()].fields.len();
                let split = self.stack.len() - n;
                let args = self.stack.split_off(split);
                let dst = self.alloc_struct(id);
                for (fi, arg) in args.into_iter().enumerate() {
                    let field = self.structs.layouts[id.index()].fields[fi];
                    let fptr = self.field_ptr(dst, field.offset);
                    self.store_field(fptr, arg, field);
                }
                self.stack.push(dst);
            }
            StructWord::Get(id, fi) => {
                let s = self.stack.pop().expect("getter: struct operand");
                let fields = self.structs.layouts[id.index()].fields.clone();
                self.load_field_onto_stack(s, fields[fi]);
                // R9: on a linear receiver, `S>fi` still consumes the whole
                // aggregate, so every non-extracted linear field is dropped
                // here (a no-op drop-the-rest when every other field is
                // Copy, unchanged from before this slice).
                for (j, field) in fields.iter().enumerate() {
                    if j != fi && field_is_linear(field.ty, self.structs, self.enums, self.arrays) {
                        let v = self.field_value(s, *field);
                        self.emit_drop(v);
                    }
                }
            }
            StructWord::Set(id, fi) => {
                let newval = self.stack.pop().expect("setter: new field value");
                let s = self.stack.pop().expect("setter: struct operand");
                let dst = self.alloc_struct(id);
                let size = self.structs.layouts[id.index()].size;
                if size > 0 {
                    self.push_instr(Instr::Blit(s, dst, size));
                }
                let field = self.structs.layouts[id.index()].fields[fi];
                // R11: the old shell's other fields transfer via the blit
                // above (consumed, never dropped); only the field being
                // overwritten is read back out and dropped, before the store,
                // so the order is deterministic.
                if field_is_linear(field.ty, self.structs, self.enums, self.arrays) {
                    let old = self.field_value(dst, field);
                    self.emit_drop(old);
                }
                let fptr = self.field_ptr(dst, field.offset);
                self.store_field(fptr, newval, field);
                self.stack.push(dst);
            }
            StructWord::Destructure(id) => {
                let s = self.stack.pop().expect("destructure: struct operand");
                let n = self.structs.layouts[id.index()].fields.len();
                for fi in 0..n {
                    let field = self.structs.layouts[id.index()].fields[fi];
                    self.load_field_onto_stack(s, field);
                }
            }
            StructWord::Peek(id, fi) => {
                // R10: non-consuming, so the aggregate stays on the stack;
                // only the field's value is pushed on top of it. The checker
                // already rejected a linear field, so there is no drop glue
                // to consider here (unlike `Get`).
                let s = *self.stack.last().expect("peek: struct operand");
                let field = self.structs.layouts[id.index()].fields[fi];
                self.load_field_onto_stack(s, field);
            }
        }
    }

    /// Lower a variant constructor inline (R15): alloc the enum's tagged
    /// aggregate, store the discriminant (the variant's declaration index) as
    /// an `i32` at `tag_offset`, then store each field at `payload_offset +
    /// field.offset` (first field deepest, reusing `store_field`).
    fn lower_enum_word(&mut self, ew: EnumWord) {
        match ew {
            EnumWord::Construct(id, variant_idx) => {
                // Slice 9 (D-A/R1/R2): a zero-payload enum's constructor
                // needs no memory at all -- the discriminant is the value --
                // so `True`/`False` (and any other all-unit-variant enum's
                // constructors) lower to a bare `Const`, register-resident,
                // exactly the retired `TermKind::BoolLit`'s shape.
                if self.enums.layouts[id.index()].is_scalar {
                    let v = self.fresh_value(IrType::Enum(id));
                    self.push_instr(Instr::Const(v, variant_idx as i64));
                    self.stack.push(v);
                    return;
                }
                let (tag_ty, tag_offset, payload_offset, fields) = {
                    let layout = &self.enums.layouts[id.index()];
                    (
                        layout.tag_ty,
                        layout.tag_offset,
                        layout.payload_offset,
                        layout.variants[variant_idx].fields.clone(),
                    )
                };
                let split = self.stack.len() - fields.len();
                let args = self.stack.split_off(split);
                let dst = self.alloc_enum(id);
                let tag = self.fresh_value(tag_ty);
                self.push_instr(Instr::Const(tag, variant_idx as i64));
                let tag_ptr = self.field_ptr(dst, tag_offset);
                self.push_instr(Instr::FieldStore(tag_ptr, tag));
                for (arg, field) in args.into_iter().zip(fields) {
                    let fptr = self.field_ptr(dst, payload_offset + field.offset);
                    self.store_field(fptr, arg, field);
                }
                self.stack.push(dst);
            }
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
