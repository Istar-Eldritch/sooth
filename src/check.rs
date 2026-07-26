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
    intern_array_type, intern_owned_cell_type, ArrayDecl, Clause, EnumDecl, EnumId, Module,
    OwnedCellDecl, Span, StackEffect, StructDecl, StructId, Term, TermKind, Type, VariantDecl,
    WordBody, WordDef, SPY_NAME,
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
}

impl Slot {
    /// A slot holding a computed (non-literal) value of `ty`: every path but
    /// a bare `IntLit` push produces one of these.
    fn computed(ty: Type) -> Slot {
        Slot {
            ty,
            literal: false,
            int_val: None,
        }
    }
}

/// The outcome of matching one `Slot` against a single expected `Type`
/// (a word-call argument, a declared output slot, or a binary operator's
/// second operand once the first has picked a target type): exact, D8's
/// literal coercion into a `usize` position, the specific "needs `>usize`"
/// diagnostic (X10) for a *computed* value in that position, or a plain
/// mismatch.
enum SlotMatch {
    Exact,
    LiteralUsize,
    NeedsUsizeConversion,
    Mismatch,
}

fn match_slot(found: Slot, want: Type) -> SlotMatch {
    if found.ty == want {
        return SlotMatch::Exact;
    }
    if want == Type::Usize && found.ty == Type::I64 {
        return if found.literal {
            SlotMatch::LiteralUsize
        } else {
            SlotMatch::NeedsUsizeConversion
        };
    }
    SlotMatch::Mismatch
}

/// The result of unifying two `Slot`s for a homogeneous binary operator
/// (`+ - * = < > <= >= <> mod and or xor`): the operands' common `Type` once
/// D8's literal coercion is applied (a `usize` paired with a bare integer
/// literal unifies to `usize`), the X10 diagnostic's target for a `usize`
/// paired with a *computed* `i64` instead, or a plain mismatch.
enum PairMatch {
    Ok(Type),
    NeedsUsizeConversion,
    Mismatch,
}

fn unify_pair(a: Slot, b: Slot) -> PairMatch {
    if a.ty == b.ty {
        return PairMatch::Ok(a.ty);
    }
    match (a.ty, b.ty) {
        (Type::Usize, Type::I64) if b.literal => PairMatch::Ok(Type::Usize),
        (Type::I64, Type::Usize) if a.literal => PairMatch::Ok(Type::Usize),
        (Type::Usize, Type::I64) | (Type::I64, Type::Usize) => PairMatch::NeedsUsizeConversion,
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
        // R4: always linear regardless of payload, with no payload lookup,
        // so this arm never recurses (unlike struct/enum/array above) and
        // `is_copy`'s arity stays unchanged.
        Type::OwnedCell(_, _) => false,
        _ => true,
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

/// The move-state of every *linear* local in the scope being checked, threaded
/// `&mut` through the walker (R14). A Copy local never appears: it carries no
/// ownership obligation, so mentioning it twice is ordinary reuse.
#[derive(Debug, Clone, Default)]
struct Moves {
    states: HashMap<String, MoveState>,
}

impl Moves {
    fn new(
        locals: &HashMap<String, Type>,
        structs: &[StructDecl],
        enums: &[EnumDecl],
        arrays: &[ArrayDecl],
    ) -> Moves {
        Moves {
            states: locals
                .iter()
                .filter(|(_, ty)| !is_copy(**ty, structs, enums, arrays))
                .map(|(name, _)| (name.clone(), MoveState::Live))
                .collect(),
        }
    }

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

/// Error context for the shared stack simulation: a full word (with its
/// declared effect and typed locals) or a bare REPL line (no signature to cite).
/// Both carry the struct/enum registries `is_copy` needs to resolve a
/// `Type::Struct`/`Type::Enum`'s linearity, so `dup`/`over`/back-edge checking
/// works identically whether the caller is a compiled word or a REPL line.
enum Ctx<'a> {
    Word {
        name: &'a str,
        effect: &'a StackEffect,
        locals: &'a HashMap<String, Type>,
        structs: &'a [StructDecl],
        enums: &'a [EnumDecl],
    },
    Line {
        structs: &'a [StructDecl],
        enums: &'a [EnumDecl],
    },
}

impl Ctx<'_> {
    fn local_type(&self, name: &str) -> Option<Type> {
        match self {
            Ctx::Word { locals, .. } => locals.get(name).copied(),
            Ctx::Line { .. } => None,
        }
    }

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
    check_types(&module.structs, &module.enums, &module.arrays)?;

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
    } = module;
    for word in words.iter() {
        check_word(word, enums, &env, arrays, owned_cells, structs)?;
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
) -> Result<(), String> {
    check_duplicate_type_names(structs, enums)?;
    check_recursion(structs, enums, arrays)?;
    check_no_linear_array_elements(structs, enums, arrays)?;
    Ok(())
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
    check_types(structs, &[], &[])
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
/// edge).
fn type_node(ty: &Type) -> Option<TypeNode> {
    match ty {
        Type::Struct(id, _) => Some(TypeNode::Struct(id.index())),
        Type::Enum(id, _) => Some(TypeNode::Enum(id.index())),
        Type::Array(id, _) => Some(TypeNode::Array(id.index())),
        _ => None,
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
pub fn check_def(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    structs: &[StructDecl],
) -> Result<(), String> {
    let mut env = env.clone();
    env.insert(word.name.clone(), sig_of(&word.effect));
    check_word(word, enums, &env, arrays, cells, structs)
}

/// Infer the net effect of a bare line: simulate the typed stack from
/// `entry_stack` (the carried slot types) and return the resulting typed stack.
/// A type mismatch or underflow against the carried stack is a reported error.
#[allow(clippy::too_many_arguments)] // a bare line's checking inputs; a bundle would obscure them
pub fn infer_line(
    terms: &[Term],
    entry_stack: &[Type],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
) -> Result<Vec<Type>, String> {
    let initial: Vec<Slot> = entry_stack.iter().map(|ty| Slot::computed(*ty)).collect();
    // A bare line binds no locals (so it has no move-state) and is not a word
    // body (so nothing in it is in tail position).
    let final_stack = check_terms(
        terms,
        initial,
        &Ctx::Line { structs, enums },
        env,
        arrays,
        cells,
        &mut Moves::default(),
        false,
    )?;
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
        .find(|ty| !is_copy(*ty, structs, enums, arrays));
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

/// A parameter / word-entry / clause-body binding name equal to a registered
/// variant name is a sharp error (D8 backstop, X12): it would make the
/// clause-vs-locals `|` disambiguation ambiguous.
fn reject_variant_local(
    word_name: &str,
    name: &str,
    kind: &str,
    enums: &[EnumDecl],
) -> Result<(), String> {
    if is_registered_variant(name, enums) {
        return Err(format!(
            "error: {kind} `{name}` in `{word_name}` collides with the variant name `{name}`"
        ));
    }
    Ok(())
}

/// A name repeated in a binding list (`| a a |`) collapses to last-wins when
/// zipped into the name -> type map, so the earlier binding (and any linear
/// value held in it) is tracked by nothing and never disposed. Reject
/// unconditionally, regardless of the bound type.
fn reject_duplicate_local<'a>(
    word_name: &str,
    name: &'a str,
    span: Span,
    seen: &mut HashSet<&'a str>,
) -> Result<(), String> {
    if !seen.insert(name) {
        return Err(format!(
            "error: duplicate local `{name}` in `{word_name}` (line {})\n  `{name}` is bound twice; the second binding shadows the first and silently drops it",
            span.line
        ));
    }
    Ok(())
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
            .find(|s| !is_copy(s.ty, structs, enums, arrays))
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
            SlotMatch::Exact | SlotMatch::LiteralUsize => {}
            SlotMatch::NeedsUsizeConversion => {
                return Err(format!(
                    "error: type mismatch in `{}` (line {})\n  body leaves a computed `i64` where the declaration requires `usize`: convert it explicitly with `>usize` first (a bare integer literal coerces automatically, a computed value does not)\n  note: declared {}",
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

#[allow(clippy::too_many_arguments)] // one word's checking inputs; a bundle would obscure them
fn check_word(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    structs: &[StructDecl],
) -> Result<(), String> {
    // A parameter name equal to a registered variant name is rejected (X12)
    // regardless of body form.
    for slot in &word.effect.inputs {
        if let Some(name) = &slot.name {
            reject_variant_local(&word.name, name, "parameter", enums)?;
        }
    }
    match &word.body {
        WordBody::Terms { locals, terms } => {
            check_terms_word(word, enums, locals, terms, env, arrays, cells, structs)
        }
        WordBody::Clauses(clauses) => {
            check_clause_word(word, enums, clauses, env, arrays, cells, structs)
        }
    }
}

#[allow(clippy::too_many_arguments)] // one word's checking inputs; a bundle would obscure them
fn check_terms_word(
    word: &WordDef,
    enums: &[EnumDecl],
    locals: &[String],
    terms: &[Term],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    structs: &[StructDecl],
) -> Result<(), String> {
    let inputs = word.effect.inputs.len();

    if locals.len() > inputs {
        return Err(format!(
            "error: stack effect mismatch in `{}`\n  locals bind {} value(s), but only {} input(s) are declared\n  note: declared {}",
            word.name,
            locals.len(),
            inputs,
            effect_str(&word.effect),
        ));
    }
    let dup_span = terms.first().map(|t| t.span).unwrap_or_default();
    let mut seen_locals = HashSet::new();
    for name in locals {
        reject_variant_local(&word.name, name, "local", enums)?;
        reject_duplicate_local(&word.name, name, dup_span, &mut seen_locals)?;
    }

    // Locals bind the topmost inputs; the remaining (deepest) inputs stay on the
    // simulated stack, deepest-first.
    let split = inputs - locals.len();
    let initial: Vec<Slot> = word.effect.inputs[..split]
        .iter()
        .map(|s| Slot::computed(s.ty))
        .collect();
    let mut local_types = HashMap::new();
    for (name, slot) in locals.iter().zip(&word.effect.inputs[split..]) {
        local_types.insert(name.clone(), slot.ty);
    }

    let ctx = Ctx::Word {
        name: &word.name,
        effect: &word.effect,
        locals: &local_types,
        structs,
        enums,
    };
    let mut moves = Moves::new(&local_types, structs, enums, arrays);
    let final_stack = check_terms(terms, initial, &ctx, env, arrays, cells, &mut moves, true)?;

    let declared: Vec<Type> = word.effect.outputs.iter().map(|s| s.ty).collect();
    let line = terms.last().map(|t| t.span.line).unwrap_or(0);
    check_outputs(word, &final_stack, &declared, line, structs, enums, arrays)?;
    check_linear_locals_consumed(word, &local_types, &moves, line)
}

/// Check a clause-style word (D4, D5, D7, M6, R11): the top input must be an
/// enum (X7), the clauses must cover every variant exactly once (X4/X5/X6),
/// and every clause body must leave the word's single declared output effect
/// (X8).
#[allow(clippy::too_many_arguments)] // one clause-style word's checking inputs; a bundle would obscure them
fn check_clause_word(
    word: &WordDef,
    enums: &[EnumDecl],
    clauses: &[Clause],
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    structs: &[StructDecl],
) -> Result<(), String> {
    let enum_id = match word.effect.inputs.last().map(|s| s.ty) {
        Some(Type::Enum(id, _)) => id,
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

    let mut seen: HashMap<&str, ()> = HashMap::new();
    for clause in clauses {
        let Some(vi) = enum_decl
            .variants
            .iter()
            .position(|v| v.name == clause.variant)
        else {
            return Err(format!(
                "error: unknown variant `{}` of enum `{}` in clause-style `{}` (line {})",
                clause.variant, enum_name, word.name, clause.span.line
            ));
        };
        if seen.insert(clause.variant.as_str(), ()).is_some() {
            return Err(format!(
                "error: duplicate clause for variant `{}` of enum `{}` in `{}` (line {})",
                clause.variant, enum_name, word.name, clause.span.line
            ));
        }
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
            structs,
        )?;
    }
    for variant in &enum_decl.variants {
        if !seen.contains_key(variant.name.as_str()) {
            return Err(format!(
                "error: non-exhaustive clause-style `{}`: missing variant `{}` of enum `{}`",
                word.name, variant.name, enum_name
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // one clause's checking inputs; a bundle would obscure them
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
    structs: &[StructDecl],
) -> Result<(), String> {
    let mut seen_locals = HashSet::new();
    for name in &clause.locals {
        reject_variant_local(&word.name, name, "local", enums)?;
        reject_duplicate_local(&word.name, name, clause.span, &mut seen_locals)?;
    }

    // The clause consumes the scrutinee and pushes the variant's fields
    // (first field deepest) atop any inputs below it.
    let mut initial = below.to_vec();
    for (_, ty) in &variant.fields {
        initial.push(*ty);
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
    let mut local_types = HashMap::new();
    for (name, ty) in clause.locals.iter().zip(&initial[split..]) {
        local_types.insert(name.clone(), *ty);
    }
    let stack_after_bind: Vec<Slot> = initial[..split]
        .iter()
        .map(|ty| Slot::computed(*ty))
        .collect();

    let ctx = Ctx::Word {
        name: &word.name,
        effect: &word.effect,
        locals: &local_types,
        structs,
        enums,
    };
    let mut moves = Moves::new(&local_types, structs, enums, arrays);
    let final_stack = check_terms(
        &clause.body,
        stack_after_bind,
        &ctx,
        env,
        arrays,
        cells,
        &mut moves,
        true,
    )?;
    let line = clause
        .body
        .last()
        .map(|t| t.span.line)
        .unwrap_or(clause.span.line);
    check_outputs(word, &final_stack, declared, line, structs, enums, arrays)?;
    check_linear_locals_consumed(word, &local_types, &moves, line)
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
fn linear_local_unconsumed_error(word: &WordDef, local: &str, ty: Type, line: u32) -> String {
    format!(
        "error: linear value `{}` is never consumed in `{}` (line {})\n  `{}` has type `{}`, which is linear: drop it or return it (nothing is dropped for you)\n  note: declared {}",
        local,
        word.name,
        line,
        local,
        ty,
        effect_str(&word.effect),
    )
}

/// R13/R14: a linear local consumed on one `if` arm but not the other. Unlike
/// `linear_local_unconsumed_error` (never touched at all), this local WAS
/// disposed on one path; the bug is the other arm forgetting it, so the
/// message points at the divergence rather than implying nothing happened.
fn linear_local_maybe_moved_error(word: &WordDef, local: &str, ty: Type, line: u32) -> String {
    format!(
        "error: linear value `{}` is not consumed on every path in `{}` (line {})\n  `{}` has type `{}`, which is linear: it is consumed on one `if` arm but not the other, so drop it (or return it) on every path\n  note: declared {}",
        local,
        word.name,
        line,
        local,
        ty,
        effect_str(&word.effect),
    )
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

/// R15: reject a linear value that would survive the back-edge of a
/// self-tail-call, either stranded on the stack below the call's arguments or
/// held by a local that was never consumed. A value *moved into* the call's
/// arguments is forwarded, not live across the edge, so it stays legal.
fn check_linear_across_back_edge(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    below_args: &[Slot],
    moves: &Moves,
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    if let Some(slot) = below_args
        .iter()
        .find(|s| !is_copy(s.ty, ctx.structs(), ctx.enums(), arrays))
    {
        return Err(linear_across_back_edge_error(ctx, span, callee, slot.ty));
    }
    if let Some(local) = moves.unconsumed().first() {
        let ty = ctx
            .local_type(local)
            .expect("a tracked local has a declared type");
        return Err(linear_across_back_edge_error(ctx, span, callee, ty));
    }
    Ok(())
}

/// R13/R14: every linear local must be consumed exactly once by the end of its
/// scope. A local still `Live` or `MaybeMoved` is the forgotten-disposal error.
fn check_linear_locals_consumed(
    word: &WordDef,
    locals: &HashMap<String, Type>,
    moves: &Moves,
    line: u32,
) -> Result<(), String> {
    match moves.unconsumed().first() {
        Some(local) => {
            let ty = locals[*local];
            match moves.states.get(*local) {
                Some(MoveState::MaybeMoved(_)) => {
                    Err(linear_local_maybe_moved_error(word, local, ty, line))
                }
                _ => Err(linear_local_unconsumed_error(word, local, ty, line)),
            }
        }
        None => Ok(()),
    }
}

/// A `usize` position (a binary operator's other operand, a word-call
/// argument, or a declared output) fed a *computed* (non-literal) `i64`
/// (X10): unlike a bare integer literal, a computed value doesn't
/// silently coerce, since Sooth has no comptime interpreter to fold it
/// and confirm it fits; names the missing `>usize` conversion explicitly.
fn usize_conversion_needed_error(ctx: &Ctx, span: Span, op: &str) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` mixes `usize` with a computed `i64`: convert it explicitly with `>usize` first (a bare integer literal coerces automatically, a computed value does not)\n  note: declared {}",
            name, span.line, op, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` mixes `usize` with a computed `i64`: convert it explicitly with `>usize` first"
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

/// Walk a term sequence. `moves` is the scope's linear-local move-state,
/// mutated in place as locals are mentioned; `tail` marks the sequence as
/// occupying its word's tail position, so its final term (and, recursively,
/// both arms of a final `if`) sits on the self-tail-call back-edge. The rule
/// mirrors `tail_position_calls`/`lower_terms`; all three must stay in
/// lockstep.
#[allow(clippy::too_many_arguments)] // the walker's threaded state; a bundle would obscure it
fn check_terms(
    terms: &[Term],
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    moves: &mut Moves,
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
            moves,
            tail && i == last,
        )?;
    }
    Ok(stack)
}

#[allow(clippy::too_many_arguments)] // the walker's threaded state; a bundle would obscure it
fn check_term(
    term: &Term,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    moves: &mut Moves,
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
        TermKind::Call(name) => {
            if let Some(ty) = ctx.local_type(name) {
                // R3 (D2): mentioning a linear local moves its value out; a
                // second mention names the site that already consumed it.
                if let Err(site) = moves.take(name, span) {
                    return Err(use_after_move_error(ctx, span, name, ty, site));
                }
                stack.push(Slot::computed(ty));
                return Ok(stack);
            }
            if let Some(stack) = check_shuffle(name, span, &mut stack, ctx, arrays)? {
                return Ok(stack);
            }
            if let Some(stack) = check_operator(name, span, &mut stack, ctx)? {
                return Ok(stack);
            }
            if let Some(stack) = check_array_word(name, span, &mut stack, ctx, arrays)? {
                return Ok(stack);
            }
            if let Some(stack) = check_owned_cell_word(name, span, &mut stack, ctx, arrays, cells)?
            {
                return Ok(stack);
            }
            if let Some(stack) = check_struct_peek_word(name, span, &mut stack, ctx, arrays)? {
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
                    SlotMatch::Exact | SlotMatch::LiteralUsize => {}
                    SlotMatch::NeedsUsizeConversion => {
                        return Err(usize_conversion_needed_error(ctx, span, name));
                    }
                    SlotMatch::Mismatch => {
                        return Err(type_mismatch_error(ctx, span, name, *want, found.ty));
                    }
                }
            }
            if tail && ctx.word_name() == Some(name.as_str()) {
                check_linear_across_back_edge(ctx, span, name, &stack[..base], moves, arrays)?;
            }
            stack.truncate(base);
            stack.extend(sig.outputs.iter().map(|ty| Slot::computed(*ty)));
            Ok(stack)
        }
        TermKind::If {
            then_branch,
            else_branch,
        } => {
            let cond = stack
                .pop()
                .ok_or_else(|| underflow_error(ctx, span, "if", 1, 0))?;
            if cond.ty != Type::Bool {
                return Err(type_mismatch_error(ctx, span, "if", Type::Bool, cond.ty));
            }
            // R14: each arm advances its own copy of the move-state; the join
            // reconciles them into `MaybeMoved` wherever they disagree.
            let mut then_moves = moves.clone();
            let mut else_moves = moves.clone();
            let then_stack = check_terms(
                then_branch,
                stack.clone(),
                ctx,
                env,
                arrays,
                cells,
                &mut then_moves,
                tail,
            )?;
            let else_stack = check_terms(
                else_branch,
                stack,
                ctx,
                env,
                arrays,
                cells,
                &mut else_moves,
                tail,
            )?;
            *moves = Moves::join(then_moves, else_moves);
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
                // A merged slot is a coercible literal only if *both* arms
                // leave a literal there: a value computed on either runtime
                // path is computed after the merge, so it can't silently fill
                // a `usize` position without `>usize` (D8/X10).
                merged.push(Slot {
                    ty: t_then.ty,
                    literal: t_then.literal && t_else.literal,
                    // A value merged from two branches is never a single
                    // known literal, so it can't feed a compile-time count.
                    int_val: None,
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
    // coercion (`Ok`); `Err(true)` is the `usize`/computed-`i64` X10 case,
    // `Err(false)` is a plain mismatch the caller reports with its own
    // op-specific diagnostic.
    let unify = |a: Slot, b: Slot| -> Result<Type, bool> {
        match unify_pair(a, b) {
            PairMatch::Ok(ty) => Ok(ty),
            PairMatch::NeedsUsizeConversion => Err(true),
            PairMatch::Mismatch => Err(false),
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
            let ty = unify(a, b).map_err(|needs_usize| {
                if needs_usize {
                    usize_conversion_needed_error(ctx, span, name)
                } else {
                    operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty)
                }
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
            let ty = unify(a, b).map_err(|needs_usize| {
                if needs_usize {
                    usize_conversion_needed_error(ctx, span, name)
                } else {
                    mod_requires_int_error(ctx, span, a.ty, b.ty)
                }
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
            let ty = unify(a, b).map_err(|needs_usize| {
                if needs_usize {
                    usize_conversion_needed_error(ctx, span, name)
                } else {
                    bitwise_pair_mismatch_error(ctx, span, name, a.ty, b.ty)
                }
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
            unify(a, b).map_err(|needs_usize| {
                if needs_usize {
                    usize_conversion_needed_error(ctx, span, name)
                } else {
                    operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty)
                }
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

/// `^|> ( ^T -- ^T T )` (R11/R14) applied to a linear payload: unlike `S|>fi`
/// (whose field is Copy-gated the same way), the cell stays live afterward, so
/// peeking would leave a second, unowned reference to a resource the cell
/// still owns; there is no reference machinery to make that legal. `^>`
/// (consuming unwrap) is the workaround.
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
        SlotMatch::LiteralUsize => {
            let idx = index.int_val.expect("a literal slot carries its value");
            if idx < 0 || idx >= i64::from(count) {
                return Err(array_index_out_of_range_error(ctx, span, count, idx));
            }
            Ok(())
        }
        SlotMatch::NeedsUsizeConversion => Err(usize_conversion_needed_error(ctx, span, op)),
        SlotMatch::Mismatch => Err(type_mismatch_error(ctx, span, op, Type::Usize, index.ty)),
    }
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
            stack.truncate(n - 1);
            stack.push(Slot::computed(elem));
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
                SlotMatch::Exact | SlotMatch::LiteralUsize => {}
                SlotMatch::NeedsUsizeConversion => {
                    return Err(usize_conversion_needed_error(ctx, span, "set"));
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

/// The three owning-cell access words (R11, R12, R12b): `^ ( T -- ^T )`
/// constructs a cell around whatever type sits on top of the stack (generic
/// over the payload shape, like `fill`, dispatching on the concrete operand
/// type rather than a fixed env signature); `^> ( ^T -- T )` consumes the
/// cell and yields the payload (frees the cell at lowering time, R13); `^|>
/// ( ^T -- ^T T )` is a non-consuming peek, restricted to a `Copy` payload
/// (R14) exactly like `S|>fi`. Matched by **exact name only** (R12b): `^>x`
/// and `^|>x` don't match any arm here and fall through to the ordinary
/// unknown-word error. Must run before `check_struct_peek_word`, whose
/// `"^|>".split_once("|>")` would otherwise probe a struct named `^` with an
/// empty field name.
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
    stack.push(Slot::computed(field_ty));
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
            stack.push(top);
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
    fn check_struct_direct_recursion_is_error_not_hang() {
        // X3/M5: a directly self-referential struct is a located error, and
        // this test itself is proof the checker terminated rather than hung.
        let err = check_src("type: Loop next Loop ;").unwrap_err();
        assert!(
            err.contains("recursive struct"),
            "unexpected message: {err}"
        );
        assert!(err.contains("Loop"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_mutual_recursion_is_error_not_hang() {
        // X3/M5: a mutually-recursive pair of structs, names both in the cycle.
        let err = check_src("type: A b B ; type: B a A ;").unwrap_err();
        assert!(
            err.contains("recursive struct"),
            "unexpected message: {err}"
        );
        assert!(err.contains('A'), "unexpected message: {err}");
        assert!(err.contains('B'), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_direct_recursion_is_error_not_hang() {
        // X3/M5: a directly self-referential enum (a variant field of its own
        // type) is a located error naming the cycle, and this test's return
        // is proof the DFS terminated rather than hung.
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
        assert!(err.contains("linear array elements are not supported yet"));
        assert!(err.contains("^i64"));
    }

    #[test]
    fn owned_of_linear_array_is_error() {
        let err = check_src(": w ( ^[__spy 2] -- ) drop ; : main ( -- ) 0 . ;").unwrap_err();
        assert!(err.contains("linear array elements are not supported yet"));
        assert!(err.contains("__spy"));
    }

    #[test]
    fn nested_array_of_owned_is_error() {
        let err = check_src(": w ( ^[^i64 4] -- ) drop ; : main ( -- ) 0 . ;").unwrap_err();
        assert!(err.contains("linear array elements are not supported yet"));
        assert!(err.contains("^i64"));
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
}
