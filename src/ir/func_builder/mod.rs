//! `FuncBuilder`: the compile-time virtual-stack lowering machine. Holds the
//! per-function block/value/loop bookkeeping and the term-lowering methods that
//! turn a word body into SSA-shaped blocks, plus the shared `lower_word_parts`
//! entry point that drives a `FuncBuilder` through a word body. Both the
//! lowering driver (`driver`) and the destructor synthesizer (`destructors`)
//! call `lower_word_parts` here, so it lives at this shared dependency root
//! rather than in either caller (Q2). Depends on `types`, `layout`, and
//! `destructors`; the lowering driver (parent module) drives it.

mod calls;
mod control_flow;
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
}

/// 7b/R16/R17: turn the env `word` holding capture `cap` into the value the
/// lowered body binds. A reference capture *is* the pointer (carry its
/// referent shape across); a scalar snapshot reinterprets the word back to
/// the scalar's own type (`Ptr` is neither arithmetic nor printable) via a
/// one-word scratch slot: `FieldStore` the received `Ptr`-typed word (its full
/// width, matching the env slot `build_env` wrote the capture's own bytes
/// into) then `FieldLoad` it back at `cap.ty`'s own width/class. A typed
/// add-of-zero previously stood in for this -- correct only when `cap.ty`
/// shared `Ptr`'s width and register class (an integer), wrong for `bool`
/// (narrower, silently read garbage upper bytes) and any float (a mismatched-
/// class `add` the backend rejects outright). The memory round-trip is
/// class-agnostic and needs no assumption about `Ptr`'s concrete width (NF1).
fn bind_env_capture(b: &mut FuncBuilder, cap: &EnvCapture, word: Value) -> Value {
    match cap.referent {
        Some(referent) => {
            b.ref_inner.insert(word, referent);
            word
        }
        None => {
            let slot = b.fresh_value(IrType::Ptr);
            b.push_alloc(Instr::Alloc(slot, WORD_WIDTH, WORD_WIDTH));
            b.push_instr(Instr::FieldStore(slot, word));
            let v = b.fresh_value(cap.ty);
            b.push_instr(Instr::FieldLoad(v, slot));
            v
        }
    }
}

/// The shared word-body lowering, parameterized by name/effect/body so a
/// monomorphized instantiation (R9) can lower a polymorphic word's body under
/// its mangled symbol against a `θ`-substituted concrete effect. The
/// instantiation table and poly-arity map thread through so a call to a
/// polymorphic word inside this body resolves to its per-site symbol (R14).
/// Lives here rather than in `driver` so `destructors` can call it without a
/// `destructors` → `driver` back-edge (Q2: shared code at the shared root).
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_word_parts(
    name: &str,
    effect: &StackEffect,
    body: &WordBody,
    self_tail: bool,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    instantiations: &HashMap<Span, CallInst>,
    builtin_overloads: &HashMap<Span, String>,
    poly_arities: &HashMap<String, usize>,
    combinators: &HashMap<String, Vec<Term>>,
    env_plan: EnvPlan,
) -> Vec<IrFunc> {
    let mut params: Vec<IrType> = effect.inputs.iter().map(|s| ir_type_of(s.ty)).collect();
    // 7b/R17: a materialized quotation body takes one trailing `Ptr` env
    // parameter after its declared inputs (even when it captures nothing, so
    // `lower_indirect_call` can pass the env slot uniformly).
    let n_declared = params.len();
    if matches!(env_plan, EnvPlan::Env(_)) {
        params.push(IrType::Ptr);
    }
    let bundle = bundle_of(&effect.outputs, regs.structs);
    let ret = word_ret_ty(&effect.outputs, regs.structs);

    let mut b = FuncBuilder::new(env, resolve, regs, name.to_string());
    b.instantiations = instantiations;
    b.builtin_overloads = builtin_overloads;
    b.poly_arities = poly_arities;
    b.combinators = combinators;
    // R11: the declared output row's `IrType`s, so a tail branch join can find
    // the target quotation type for the slot it materializes.
    b.cur_outputs = effect.outputs.iter().map(|s| ir_type_of(s.ty)).collect();

    // Params occupy the first N value ids; leftmost input is deepest.
    // (b.cur_word_name is set above for R7's self-tail-call detection.)
    let params_values: Vec<Value> = params.iter().map(|ty| b.fresh_value(*ty)).collect();

    // R6: a self-tail-recursive word lowers to a loop. The entry block binds
    // the params and jumps to a header carrying one phi per loop-carried slot;
    // the body reads the phi outputs so each iteration rebinds them. A word
    // with no tail self-call lowers exactly as before (no header, no phi).
    let entry_values = if self_tail {
        // R1a: aggregate staging gated ON for the user self-tail-call loop. A
        // materialized body is never self-tail, so its env param is never here.
        b.begin_loop(&params_values, true)
    } else {
        params_values
    };

    // 7b/R17: the trailing env param is not a stack input; split it off. Its
    // value binds the captured local (if any); the declared inputs alone seed
    // the stack.
    let env_value = if matches!(env_plan, EnvPlan::Env(_)) {
        Some(entry_values[n_declared])
    } else {
        None
    };
    let stack_inputs: Vec<Value> = entry_values[..n_declared].to_vec();

    // A reference parameter arrives as an opaque `Ptr`, so the referent
    // shape every projection and access needs comes from the declared type,
    // not from the value. Seeded against `stack_inputs` so a loop reads it off
    // the header phi output the body actually uses.
    for (slot, value) in effect.inputs.iter().zip(&stack_inputs) {
        if let Type::Ref(id, _, _) = slot.ty {
            b.ref_inner.insert(*value, regs.refs.referent[id.index()]);
        }
    }

    // 7b/R16/R17: bind each captured local to a read of the env before the
    // body runs, so its `Call` references resolve. With one capture the env
    // word *is* the capture (inline); with two or more the env word is a
    // pointer to a stack bundle, each capture read from its word offset.
    if let (Some(env), EnvPlan::Env(caps)) = (env_value, &env_plan) {
        match caps.as_slice() {
            [] => {}
            [cap] => {
                let bound = bind_env_capture(&mut b, cap, env);
                b.locals.push((cap.name.clone(), bound));
            }
            many => {
                for (i, cap) in many.iter().enumerate() {
                    let slot = b.field_ptr(env, i as u32 * WORD_WIDTH);
                    let word = b.fresh_value(IrType::Ptr);
                    b.push_instr(Instr::FieldLoad(word, slot));
                    let bound = bind_env_capture(&mut b, cap, word);
                    b.locals.push((cap.name.clone(), bound));
                }
            }
        }
    }

    match body {
        WordBody::Terms { terms } => {
            // Every input starts on the stack (D6: the header phi outputs when
            // looping); an entry `| ... |` binding pops from it like any other
            // binding term.
            b.stack = stack_inputs;
            b.lower_terms(terms, self_tail);
        }
        WordBody::Clauses(clauses) => {
            let scrutinee_ty = effect
                .inputs
                .last()
                .expect("clause word has a scrutinee input")
                .ty;
            b.lower_clauses(clauses, &stack_inputs, scrutinee_ty)
        }
    }

    // R8: back-patch the header phis with the collected back-edge operands.
    if self_tail {
        b.finalize_loop();
    }

    // The fall-through (base-case) block returns; a body that ended entirely in
    // back-edges is already terminated and needs no Ret.
    if !b.terminated {
        // R10: two or more outputs leave the frame packed into the bundle,
        // deepest output in the first field; one or none is the single value
        // (or nothing) it always was.
        let result = match bundle {
            Some(id) => Some(b.pack_bundle(id)),
            // R7/R9: a word declaring a `Type::Quotation` output is a
            // materialization boundary; a phantom the body leaves there becomes
            // a real `(code, env)` value before it is returned.
            None if ret.is_some() => {
                let v = b
                    .stack
                    .pop()
                    .expect("a word with a declared output leaves one");
                Some(b.materialize_if_phantom(v, ret.expect("ret.is_some()")))
            }
            None => None,
        };
        b.seal_block(Terminator::Ret(result));
    }

    // R9: this word is done; any quotation literal a materialization boundary
    // turned into a value is lowered into its own `IrFunc` here (recursively:
    // a materialized body may itself materialize a nested quotation). The main
    // func is element 0; every caller flattens the returned vec into the
    // module's function list.
    let mats = std::mem::take(&mut b.materialized);
    let mut out = vec![IrFunc {
        name: name.to_string(),
        params,
        ret,
        blocks: b.blocks,
        value_types: b.value_types,
    }];
    out.extend(lower_materialized(
        mats,
        env,
        resolve,
        regs,
        instantiations,
        builtin_overloads,
        poly_arities,
        combinators,
    ));
    out
}

/// Slice 7a (R9): lower a batch of materialized quotations into standalone
/// `IrFunc`s. Each is an ordinary term-bodied word under its minted symbol and
/// declared effect; `lower_word_parts` handles it (and any nested quotation it
/// materializes) exactly like a user word. Shared by `lower_word_parts` and
/// `lower_line`, the two lowering entry points that own a `FuncBuilder`.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_materialized(
    mats: Vec<MaterializedQuot>,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    instantiations: &HashMap<Span, CallInst>,
    builtin_overloads: &HashMap<Span, String>,
    poly_arities: &HashMap<String, usize>,
    combinators: &HashMap<String, Vec<Term>>,
) -> Vec<IrFunc> {
    let mut out = Vec::new();
    for m in mats {
        let effect = StackEffect {
            inputs: m
                .effect
                .inputs
                .iter()
                .map(|&ty| TypedSlot { name: None, ty })
                .collect(),
            outputs: m
                .effect
                .outputs
                .iter()
                .map(|&ty| TypedSlot { name: None, ty })
                .collect(),
        };
        let body = WordBody::Terms { terms: m.body };
        out.extend(lower_word_parts(
            &m.symbol,
            &effect,
            &body,
            false,
            env,
            resolve,
            regs,
            instantiations,
            builtin_overloads,
            poly_arities,
            combinators,
            EnvPlan::Env(m.captures),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::test_helpers::*;

    #[test]
    fn func_builder_new_threads_current_word_name() {
        // R5: FuncBuilder carries the word being lowered, set from `word.name`
        // in `lower_word`; the REPL path calls the same `lower_word` (no
        // REPL-specific plumbing), so this covers both callers.
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let b = FuncBuilder::new(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
            },
            "loop-word".to_string(),
        );
        assert_eq!(b.cur_word_name, "loop-word");
    }

    #[test]
    fn aggregate_carried_slot_gets_no_header_phi_but_scalar_does() {
        // R2: the aggregate (`Box`) slot contributes no header phi (it reads
        // its entry-hoisted stable slot); the scalar (i64) slot keeps one.
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        assert_eq!(
            phis.len(),
            1,
            "only the i64 scalar slot carries a header phi"
        );
        // `len() == 1` alone would also pass a transform that kept the `Box`
        // slot's phi and dropped the scalar's; pin that the survivor carries
        // the i64 counter, not a `Box` pointer, so "but scalar does" is checked.
        let (_, incoming) = phis[0][0];
        assert_eq!(
            f.value_types[incoming.0 as usize],
            IrType::I64,
            "the surviving header phi carries the scalar slot, not the aggregate"
        );
    }

    #[test]
    fn aggregate_stable_slot_and_temp_are_entry_hoisted_not_in_the_body() {
        // R1/R9: the stable slot and staging temp are `alloc`ed in the entry
        // block, not per-iteration in the body (which would bump the frame
        // every iteration and break the constant-stack guarantee). `instrs`
        // flattens across blocks, so this iterates `func.blocks` directly.
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let entry = &f.blocks[0];
        let entry_allocs = entry
            .instrs
            .iter()
            .filter(|i| matches!(i, Instr::Alloc(..)))
            .count();
        assert!(
            entry_allocs >= 2,
            "the stable slot and temp allocs should be hoisted into the entry block, saw {entry_allocs}"
        );
        let entry_id = entry.id;
        for block in &f.blocks {
            if block.id == entry_id || block.id == header {
                continue;
            }
            assert!(
                !block.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
                "block {:?} in the loop body must not alloc",
                block.id
            );
        }
    }

    #[test]
    fn aggregate_init_blit_lands_in_the_entry_block() {
        // R3: `begin_loop` seeds the stable slot with the incoming param once,
        // in the entry block, so iteration 1 reads an initialised value. It is
        // the only Blit routed to the entry block (the back-edge staging blits
        // go to predecessor blocks).
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let entry = &f.blocks[0];
        assert!(
            entry.instrs.iter().any(|i| matches!(i, Instr::Blit(..))),
            "the entry-arm init blit should land in the entry block"
        );
    }

    #[test]
    fn back_edge_stages_reads_before_writes() {
        // R4: on a staged back-edge, every read-phase blit (a snapshot into a
        // temp) precedes every write-phase blit (a store into the stable slot).
        // A blit is write-phase when its source is an earlier blit's dest in
        // the same predecessor block. `instrs` flattens across blocks, so this
        // inspects the predecessor block directly.
        let ir = lower_src(STAGED_LOOP);
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let pred = back_edge_pred(f, header);
        let mut written: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut seen_write = false;
        let mut blits = 0;
        for instr in &pred.instrs {
            if let Instr::Blit(src, dst, _) = instr {
                blits += 1;
                if written.contains(&src.0) {
                    seen_write = true;
                } else {
                    assert!(!seen_write, "a read-phase blit follows a write-phase blit");
                }
                written.insert(dst.0);
            }
        }
        assert!(
            blits >= 2,
            "the staged Box back-edge should emit a read and a write blit, saw {blits}"
        );
    }

    #[test]
    fn forwarded_in_place_aggregate_slot_emits_zero_back_edge_blits() {
        // R4: an aggregate carried unchanged (`prev`, its back-edge arg is
        // exactly its own stable slot) is forwarded in place and stages
        // nothing.
        let ir = lower_src(
            "type: Box n i64 ;\n\
             : mk ( i64 -- Box ) | n | n Box ;\n\
             : loop ( i64 Box -- Box ) | n prev | n 0 = if prev else n 1 - prev loop end ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "loop").unwrap();
        let header = loop_header(f);
        let pred = back_edge_pred(f, header);
        assert_eq!(
            pred.instrs
                .iter()
                .filter(|i| matches!(i, Instr::Blit(..)))
                .count(),
            0,
            "a forwarded-in-place slot emits zero back-edge blits"
        );
    }
}
