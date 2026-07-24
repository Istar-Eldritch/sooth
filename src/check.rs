//! Stack-effect checker. Simulates a compile-time virtual stack of concrete
//! `Type`s through each word body and verifies the net effect matches the
//! declared signature.
//!
//! Every operand is checked against the type its consumer expects, so a
//! `bool` where `+` wants an `i64` is a located compile error (Forth's silent
//! coercion failure mode becomes a diagnostic here). Branch join points unify
//! on both depth and per-slot type: the `then` and `else` arms must leave the
//! same stack shape.

use std::collections::HashMap;

use crate::ast::{
    Clause, EnumDecl, EnumId, Module, Span, StackEffect, StructDecl, StructId, Term, TermKind,
    Type, VariantDecl, WordBody, WordDef,
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
}

impl Slot {
    /// A slot holding a computed (non-literal) value of `ty`: every path but
    /// a bare `IntLit` push produces one of these.
    fn computed(ty: Type) -> Slot {
        Slot { ty, literal: false }
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
/// Every builtin word is structural and handled directly in `check_term`
/// (`check_shuffle`/`check_operator`): the stack shuffles, the numeric-tower
/// operators, and `.` (type-directed over any printable scalar, not a fixed
/// `( i64 -- )`) all dispatch on the concrete operand type rather than a
/// fixed signature, so this table is currently empty.
pub fn builtin_table() -> HashMap<String, Sig> {
    HashMap::new()
}

/// Error context for the shared stack simulation: a full word (with its
/// declared effect and typed locals) or a bare REPL line (no signature to cite).
enum Ctx<'a> {
    Word {
        name: &'a str,
        effect: &'a StackEffect,
        locals: &'a HashMap<String, Type>,
    },
    Line,
}

impl Ctx<'_> {
    fn local_type(&self, name: &str) -> Option<Type> {
        match self {
            Ctx::Word { locals, .. } => locals.get(name).copied(),
            Ctx::Line => None,
        }
    }
}

pub fn check(module: &Module) -> Result<(), String> {
    check_types(&module.structs, &module.enums)?;

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

    for word in &module.words {
        check_word(word, &module.enums, &env)?;
    }
    Ok(())
}

/// Type-level checks that must pass before any generated-word signature or
/// word body is type-checked: no two `type:` declarations share a name across
/// the combined struct+enum registries, and no struct or enum contains itself
/// by value, directly or transitively, through the combined type graph (D9,
/// D10, R8, R10).
pub fn check_types(structs: &[StructDecl], enums: &[EnumDecl]) -> Result<(), String> {
    check_duplicate_type_names(structs, enums)?;
    check_recursion(structs, enums)?;
    Ok(())
}

/// The struct-only projection of `check_types` (no enums), for callers that
/// don't yet declare enums.
pub fn check_structs(structs: &[StructDecl]) -> Result<(), String> {
    check_types(structs, &[])
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
}

/// Detect a struct or enum that contains itself by value, directly or
/// transitively, via cycle detection over the *combined* type graph (D9): a
/// struct's field types and an enum's variant field types are edges, so a
/// struct-of-enum-of-struct cycle is caught the same as a pure-struct one.
fn check_recursion(structs: &[StructDecl], enums: &[EnumDecl]) -> Result<(), String> {
    let mut sstate = vec![VisitState::Unvisited; structs.len()];
    let mut estate = vec![VisitState::Unvisited; enums.len()];
    let mut path = Vec::new();
    for start in 0..structs.len() {
        if sstate[start] == VisitState::Unvisited {
            visit_recursion(
                TypeNode::Struct(start),
                structs,
                enums,
                &mut sstate,
                &mut estate,
                &mut path,
            )?;
        }
    }
    for start in 0..enums.len() {
        if estate[start] == VisitState::Unvisited {
            visit_recursion(
                TypeNode::Enum(start),
                structs,
                enums,
                &mut sstate,
                &mut estate,
                &mut path,
            )?;
        }
    }
    Ok(())
}

/// The frontend `Type` of a field, mapped to a graph node (a scalar has no
/// edge).
fn type_node(ty: &Type) -> Option<TypeNode> {
    match ty {
        Type::Struct(id, _) => Some(TypeNode::Struct(id.index())),
        Type::Enum(id, _) => Some(TypeNode::Enum(id.index())),
        _ => None,
    }
}

/// The value-containment edges out of a node: a struct's field types, or every
/// variant field type of an enum.
fn node_edges(node: TypeNode, structs: &[StructDecl], enums: &[EnumDecl]) -> Vec<TypeNode> {
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
    }
}

fn node_state<'a>(
    node: TypeNode,
    sstate: &'a mut [VisitState],
    estate: &'a mut [VisitState],
) -> &'a mut VisitState {
    match node {
        TypeNode::Struct(i) => &mut sstate[i],
        TypeNode::Enum(i) => &mut estate[i],
    }
}

fn node_name<'a>(node: TypeNode, structs: &'a [StructDecl], enums: &'a [EnumDecl]) -> &'a str {
    match node {
        TypeNode::Struct(i) => structs[i].name.as_str(),
        TypeNode::Enum(i) => enums[i].name.as_str(),
    }
}

fn visit_recursion(
    node: TypeNode,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    sstate: &mut [VisitState],
    estate: &mut [VisitState],
    path: &mut Vec<TypeNode>,
) -> Result<(), String> {
    *node_state(node, sstate, estate) = VisitState::InProgress;
    path.push(node);
    for child in node_edges(node, structs, enums) {
        match *node_state(child, sstate, estate) {
            VisitState::Unvisited => visit_recursion(child, structs, enums, sstate, estate, path)?,
            VisitState::InProgress => {
                let cycle_start = path.iter().position(|&x| x == child).unwrap();
                let mut names: Vec<&str> = path[cycle_start..]
                    .iter()
                    .map(|&n| node_name(n, structs, enums))
                    .collect();
                names.push(node_name(child, structs, enums));
                // Key the wording on the repeated node's kind so a pure-struct
                // cycle keeps its Slice 3 message and an enum cycle names an
                // enum (X3).
                let kind = match child {
                    TypeNode::Struct(_) => "struct",
                    TypeNode::Enum(_) => "enum",
                };
                return Err(format!(
                    "error: recursive {kind} definition (infinite size): {}",
                    names.join(" -> ")
                ));
            }
            VisitState::Done => {}
        }
    }
    path.pop();
    *node_state(node, sstate, estate) = VisitState::Done;
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
) -> Result<(), String> {
    let mut env = env.clone();
    env.insert(word.name.clone(), sig_of(&word.effect));
    check_word(word, enums, &env)
}

/// Infer the net effect of a bare line: simulate the typed stack from
/// `entry_stack` (the carried slot types) and return the resulting typed stack.
/// A type mismatch or underflow against the carried stack is a reported error.
pub fn infer_line(
    terms: &[Term],
    entry_stack: &[Type],
    env: &HashMap<String, Sig>,
) -> Result<Vec<Type>, String> {
    let initial: Vec<Slot> = entry_stack.iter().map(|ty| Slot::computed(*ty)).collect();
    let final_stack = check_terms(terms, initial, &Ctx::Line, env)?;
    Ok(final_stack.into_iter().map(|s| s.ty).collect())
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

/// The output-count / output-type mismatch check shared by a term body and a
/// clause body (M6, X8): `final_stack` must match the declared outputs.
/// Honors D8's literal coercion (a bare integer literal satisfies a declared
/// `usize` output) and reports the X10 diagnostic for a computed one.
fn check_outputs(
    word: &WordDef,
    final_stack: &[Slot],
    declared: &[Type],
    line: u32,
) -> Result<(), String> {
    if final_stack.len() != declared.len() {
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

fn check_word(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Sig>,
) -> Result<(), String> {
    // A parameter name equal to a registered variant name is rejected (X12)
    // regardless of body form.
    for slot in &word.effect.inputs {
        if let Some(name) = &slot.name {
            reject_variant_local(&word.name, name, "parameter", enums)?;
        }
    }
    match &word.body {
        WordBody::Terms { locals, terms } => check_terms_word(word, enums, locals, terms, env),
        WordBody::Clauses(clauses) => check_clause_word(word, enums, clauses, env),
    }
}

fn check_terms_word(
    word: &WordDef,
    enums: &[EnumDecl],
    locals: &[String],
    terms: &[Term],
    env: &HashMap<String, Sig>,
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
    for name in locals {
        reject_variant_local(&word.name, name, "local", enums)?;
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
    };
    let final_stack = check_terms(terms, initial, &ctx, env)?;

    let declared: Vec<Type> = word.effect.outputs.iter().map(|s| s.ty).collect();
    let line = terms.last().map(|t| t.span.line).unwrap_or(0);
    check_outputs(word, &final_stack, &declared, line)
}

/// Check a clause-style word (D4, D5, D7, M6, R11): the top input must be an
/// enum (X7), the clauses must cover every variant exactly once (X4/X5/X6),
/// and every clause body must leave the word's single declared output effect
/// (X8).
fn check_clause_word(
    word: &WordDef,
    enums: &[EnumDecl],
    clauses: &[Clause],
    env: &HashMap<String, Sig>,
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

fn check_clause_body(
    word: &WordDef,
    enums: &[EnumDecl],
    clause: &Clause,
    below: &[Type],
    variant: &VariantDecl,
    declared: &[Type],
    env: &HashMap<String, Sig>,
) -> Result<(), String> {
    for name in &clause.locals {
        reject_variant_local(&word.name, name, "local", enums)?;
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
    };
    let final_stack = check_terms(&clause.body, stack_after_bind, &ctx, env)?;
    let line = clause
        .body
        .last()
        .map(|t| t.span.line)
        .unwrap_or(clause.span.line);
    check_outputs(word, &final_stack, declared, line)
}

fn unknown_word_error(ctx: &Ctx, span: Span, name: &str) -> String {
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: unknown word `{}` in `{}` (line {})",
            name, wname, span.line
        ),
        Ctx::Line => format!("error: unknown word `{name}`"),
    }
}

fn underflow_error(ctx: &Ctx, span: Span, op: &str, needs: usize, holds: usize) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `{}` needs {} values, but the stack holds {}\n  note: declared {}",
            name, span.line, op, needs, holds, effect_str(effect),
        ),
        Ctx::Line => format!("error: stack underflow: needs {needs} values, but the stack holds {holds}"),
    }
}

fn type_mismatch_error(ctx: &Ctx, span: Span, op: &str, expected: Type, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` expected `{}`, found `{}`\n  note: declared {}",
            name, span.line, op, expected, found, effect_str(effect),
        ),
        Ctx::Line => {
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
        Ctx::Line => format!(
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
        Ctx::Line => format!(
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
        Ctx::Line => format!(
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
        Ctx::Line => format!(
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
        Ctx::Line => format!(
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
        Ctx::Line => format!(
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
        Ctx::Line => format!(
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
        Ctx::Line => {
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
        Ctx::Line => {
            format!("error: type mismatch: `.` requires a printable scalar, found `{found}`")
        }
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
        Ctx::Line => format!(
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
        Ctx::Line => format!("error: unknown type `{name}`"),
    }
}

fn branch_mismatch_error(ctx: &Ctx, span: Span, d_then: usize, d_else: usize) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `if` branches leave different stack depths (then: {}, else: {})\n  note: declared {}",
            name, span.line, d_then, d_else, effect_str(effect),
        ),
        Ctx::Line => format!(
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
        Ctx::Line => format!(
            "error: `if` branches leave different types (then: `{t_then}`, else: `{t_else}`)"
        ),
    }
}

fn check_terms(
    terms: &[Term],
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
) -> Result<Vec<Slot>, String> {
    for term in terms {
        stack = check_term(term, stack, ctx, env)?;
    }
    Ok(stack)
}

fn check_term(
    term: &Term,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
) -> Result<Vec<Slot>, String> {
    let span = term.span;
    match &term.kind {
        TermKind::IntLit(_) => {
            // A bare integer literal is the one D8 source: fresh off the
            // term, it may still silently fill a `usize` position.
            stack.push(Slot {
                ty: Type::I64,
                literal: true,
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
                stack.push(Slot::computed(ty));
                return Ok(stack);
            }
            if let Some(stack) = check_shuffle(name, span, &mut stack, ctx)? {
                return Ok(stack);
            }
            if let Some(stack) = check_operator(name, span, &mut stack, ctx)? {
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
            let then_stack = check_terms(then_branch, stack.clone(), ctx, env)?;
            let else_stack = check_terms(else_branch, stack, ctx, env)?;
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

/// Apply a stack shuffle if `name` is one, returning `Some(stack)`; `None` if
/// the name is not a shuffle (the caller then looks it up in the env). Shuffles
/// move concrete slot types with no fixed signature: `dup` of a `bool` yields
/// two `bool`s, `swap` reorders whatever two types are on top, etc.
fn check_shuffle(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
) -> Result<Option<Vec<Slot>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "dup" => {
            let top = *stack.last().ok_or_else(|| need("dup", 1, stack.len()))?;
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
        let module = parse(&tokens).unwrap();
        check(&module)
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
        infer_line(&terms, entry, &builtin_table())
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
}
