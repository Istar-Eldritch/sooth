//! Aggregate destructor synthesis: recursive disposal path search plus the
//! struct/enum/owned-cell destructor synthesizers, driven by
//! `synthesize_aggregate_destructors`. Depends on `layout` and `func_builder`
//! (the shared `lower_word_parts` entry point, for drop-override body lowering).
use super::*;

/// Every linear struct's and enum's synthesized destructor, one `IrFunc` per
/// type. The REPL redefines these per line; safe because type redefinition is
/// rejected, so every generation's glue is identical. If type redefinition is
/// ever allowed, add a generation suffix, matching word symbols.
///
/// R11 (slice 8b): a *user* `drop` override is where that premise fails --
/// redefining one at the REPL puts a different body under the same symbol.
/// R11.2 additionally suffixes every *other* linear struct's/enum's/cell's
/// destructor too, once the session holds any override at all: any of them
/// may `Call` an overridden struct's destructor (directly, or transitively
/// through a further composed aggregate), so their own body's callee changes
/// across an override event exactly as the overridden struct's own body does.
/// All three symbol kinds carry the *same* session-wide override epoch
/// (`StructLayout`/`EnumLayout::drop_generation`, `Cells::drop_generations`,
/// set by the session), so every destructor in a session that has ever seen
/// an override mints a fresh, never-before-loaded symbol per override event
/// -- the cheap alternative to computing exactly which aggregates reach the
/// override. Before any override, epoch is `None` everywhere and every
/// symbol stays unsuffixed, unchanged from the build path.
///
/// R2 (slice 8b): a struct in `overrides` gets the user's own `drop` body under
/// that same symbol instead of the synthesized field glue. Every caller of the
/// destructor already goes through `struct_drop_symbol` (`emit_drop`, and
/// `drop_level_fields` through it), so substituting the body here is the whole
/// of dispatch: no call site resolves a `drop` overload by name.
///
/// R11.3: an `AlreadyLoaded` entry gets no destructor emitted at all — the
/// REPL marks every override but the one being declared that way, since each
/// override's symbol is pinned to its defining epoch and its body must be
/// lowered once, against the env it was checked against.
pub fn synthesize_aggregate_destructors(
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    overrides: &DropOverrides,
    combinators: &HashMap<String, Vec<Term>>,
) -> Vec<IrFunc> {
    let Registries {
        structs,
        enums,
        cells,
        ..
    } = regs;
    // R9: an override's body is lowered by `lower_word_parts`, so it can carry
    // materialized quotations of its own (element 0 is the destructor itself);
    // the glue-only cases stay single-func, wrapped so the `flatten` is uniform.
    let struct_destructors: Vec<IrFunc> = structs
        .layouts
        .iter()
        .enumerate()
        // R10/R11: a linear *bundle* gets no glue. Its fields are the caller's
        // outputs, moved out by the unpack the instant the call returns, so a
        // destructor for the shell would free a linear one a second time.
        .filter(|(_, layout)| layout.is_linear && !layout.bundle)
        .filter_map(|(idx, _)| {
            let id = StructId::from_index(idx);
            match overrides.get(&id) {
                Some(DropOverride::Body(word)) => Some(synthesize_struct_destructor_override(
                    id,
                    word,
                    env,
                    resolve,
                    regs,
                    combinators,
                )),
                Some(DropOverride::AlreadyLoaded) => None,
                None => Some(vec![synthesize_struct_destructor(id, env, resolve, regs)]),
            }
        })
        .flatten()
        .collect();
    let enum_destructors = enums
        .layouts
        .iter()
        .enumerate()
        .filter(|(_, layout)| layout.is_linear)
        .map(|(idx, _)| synthesize_enum_destructor(EnumId::from_index(idx), env, resolve, regs));
    // Every cell gets a destructor, not just those whose filter would
    // require a linear payload: `drop` on any cell must free it.
    let cell_destructors = cells.payload.iter().enumerate().map(|(idx, _)| {
        synthesize_cell_destructor(OwnedCellId::from_index(idx), env, resolve, regs)
    });
    struct_destructors
        .into_iter()
        .chain(enum_destructors)
        .chain(cell_destructors)
        .collect()
}

/// One step of the route a fused destructor loop walks from a type back to
/// itself. A tree, not a flat list: an enum's variants are mutually
/// exclusive at runtime, so each may independently continue toward `Self`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PathStep {
    /// Project a `Struct`/`Enum` field of the current aggregate byval
    /// (`field_value`, no free).
    Project { field: usize },
    /// Materialize a `^T` field's payload and free the cell
    /// (`load_owned_payload` + `free`). `field` is `None` when the current
    /// type *is* the cell (the inner step of `^^Self`) rather than an
    /// aggregate holding it; a struct can hold two fields of the same cell
    /// type, so the index is not derivable from `cell`.
    Unwrap {
        field: Option<usize>,
        cell: OwnedCellId,
    },
    /// The path reached an enum, at the entry type or any intermediate point
    /// alike: dispatch on its tag. `None` for a variant that does not
    /// continue toward `Self` (drop its fields, leave the loop); `Some` for
    /// one that does, via its own further steps. More than one variant may
    /// continue: a tagged value is only ever one variant, so this is not the
    /// simultaneously-live multi-edge case a struct's own field choice must
    /// narrow. Always the last step of the sequence containing it, since what
    /// follows a dispatch lives inside each variant's own continuation.
    Branch {
        enum_id: EnumId,
        variants: Vec<Option<Vec<PathStep>>>,
    },
}

/// The route a fused destructor loop walks from `self_ty` back to `self_ty`,
/// or `None` for a type on no cycle. A fresh pass over `Registries`, not a
/// reuse of the checker's cycle graph: that graph cuts `^` edges entirely,
/// but this needs to see exactly them.
///
/// The search starts at `expand_path`, never `find_path`: `self_ty` is seeded
/// into `visited`, and `find_path`'s trivial `current == target` match would
/// otherwise succeed before `self_ty`'s own fields were ever examined. Only a
/// *subsequent* arrival back at `self_ty`, via at least one step, is a cycle.
pub(super) fn recursive_disposal_path(self_ty: IrType, regs: Registries) -> Option<Vec<PathStep>> {
    expand_path(self_ty, self_ty, &mut vec![self_ty], regs)
}

/// One recursive hop of the walk: the trivial-match and cycle-prune checks
/// that must fire for every hop but not for the outermost one (hence the
/// split from `expand_path`). The target check precedes the prune check
/// because the entry type is itself in `visited`.
fn find_path(
    current: IrType,
    target: IrType,
    visited: &mut Vec<IrType>,
    regs: Registries,
) -> Option<Vec<PathStep>> {
    if current == target {
        return Some(Vec::new());
    }
    if visited.contains(&current) {
        return None;
    }
    visited.push(current);
    let found = expand_path(current, target, visited, regs);
    visited.pop();
    found
}

/// Search `current`'s own structure for a continuation toward `target`. A
/// cell counts as a type in its own right, so `^^Self` steps through the
/// inner cell rather than treating it as a dead end.
fn expand_path(
    current: IrType,
    target: IrType,
    visited: &mut Vec<IrType>,
    regs: Registries,
) -> Option<Vec<PathStep>> {
    match current {
        // R7 (slice 8b): a struct with a user `drop` overload is a dead end for
        // *another* type's search, exactly as a `Copy` scalar field is. The
        // fused loop inlines every intermediate type's field projection instead
        // of calling its destructor, so routing a cycle through an overridden
        // struct would bypass the user's body and leak its resource silently.
        // The `current != target` carve-out is for the search's own root: an
        // overridden struct's own destructor is its override regardless (R2), so
        // whether a path back to itself exists is moot there.
        IrType::Struct(id)
            if current != target && regs.structs.layouts[id.index()].has_drop_overload =>
        {
            None
        }
        IrType::Struct(id) => {
            let fields = &regs.structs.layouts[id.index()].fields;
            expand_fields(fields, target, visited, regs)
        }
        IrType::Enum(id) => {
            let variants: Vec<Option<Vec<PathStep>>> = regs.enums.layouts[id.index()]
                .variants
                .iter()
                .map(|v| {
                    // A copy of `visited` per variant: one variant's
                    // abandoned attempt must not poison a sibling's search.
                    let mut seen = visited.clone();
                    expand_fields(&v.fields, target, &mut seen, regs)
                })
                .collect();
            variants.iter().any(Option::is_some).then(|| {
                vec![PathStep::Branch {
                    enum_id: id,
                    variants,
                }]
            })
        }
        // `cells.payload[c] == target` needs no case of its own: `find_path`
        // matches it and returns an empty tail, which this prepend turns into
        // a lone `Unwrap`.
        IrType::OwnedCell(c) => find_path(regs.cells.payload[c.index()], target, visited, regs)
            .map(|rest| {
                prepend(
                    PathStep::Unwrap {
                        field: None,
                        cell: c,
                    },
                    rest,
                )
            }),
        _ => None,
    }
}

/// Try one struct's fields, or one enum variant's, in reverse declaration
/// order; the first candidate whose own sub-walk reaches `target` wins.
/// Backtracking, not a syntactic guess: a field is only chosen once a
/// complete path through it is known to exist.
///
/// A direct `^target` field is tried, in reverse order, before any other
/// field: this is today's fusable shape, and it must keep winning even when
/// a *later*-declared field also reaches `target`, only indirectly. Without
/// this tier, declaring an indirect-but-successful field after a direct one
/// flips which edge the reverse scan below picks, silently lengthening the
/// fused loop's path with an extra unwrap step and hoisted slot per level of
/// indirection, for a shape that a direct edge would have reached in one.
///
/// Reverse order generalizes the old direct-edge rule's last-field tie-break
/// to every struct level of the walk. This is the one restriction on a
/// struct with two fields that could each reach `target`: it picks exactly
/// one, since both may be live in one node instance at once (Phase 6's
/// worklist case). The non-chosen fields are dropped like any other field, not
/// marked. Looping the last child rather than the first is what makes a
/// right-leaning shape constant-stack and a left-leaning one still O(depth)
/// (documented, not fixed). Arrays are absent deliberately: `[^T N]` is
/// rejected outright, so an array can never launder an edge.
fn expand_fields(
    fields: &[FieldLayout],
    target: IrType,
    visited: &mut Vec<IrType>,
    regs: Registries,
) -> Option<Vec<PathStep>> {
    for (fi, field) in fields.iter().enumerate().rev() {
        if let IrType::OwnedCell(c) = field.ty {
            if regs.cells.payload[c.index()] == target {
                return Some(vec![PathStep::Unwrap {
                    field: Some(fi),
                    cell: c,
                }]);
            }
        }
    }

    for (fi, field) in fields.iter().enumerate().rev() {
        match field.ty {
            IrType::OwnedCell(c) => {
                let payload = regs.cells.payload[c.index()];
                if let Some(rest) = find_path(payload, target, visited, regs) {
                    return Some(prepend(
                        PathStep::Unwrap {
                            field: Some(fi),
                            cell: c,
                        },
                        rest,
                    ));
                }
            }
            IrType::Struct(_) | IrType::Enum(_) => {
                if let Some(rest) = find_path(field.ty, target, visited, regs) {
                    return Some(prepend(PathStep::Project { field: fi }, rest));
                }
            }
            _ => {}
        }
    }
    None
}

fn prepend(step: PathStep, rest: Vec<PathStep>) -> Vec<PathStep> {
    let mut path = vec![step];
    path.extend(rest);
    path
}

/// R12: synthesize struct `id`'s destructor, called by `drop` on any value of
/// that type: drop each linear field, in declaration order. Built via a
/// bare `FuncBuilder` (no locals, no tail-call machinery) reusing the same
/// `field_value`/`emit_drop` a `drop`, `S>fi`, and `S<fi` use, so "how a field
/// is disposed" stays in one place.
///
/// A struct on a disposal cycle (a `^Self` field, or a longer route back to
/// itself through other types) is disposed by one fused loop that walks the
/// whole route, instead of a mutually recursive `cell_drop`/`struct_drop`
/// chain. An all-struct cycle has no base case, so its loop is exit-less and
/// the trailing `Ret` is skipped; such a shape is uninhabited, so that is
/// about not crashing the emitter rather than about a program that runs.
pub(super) fn synthesize_struct_destructor(
    id: StructId,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
) -> IrFunc {
    let structs = regs.structs;
    let self_ty = IrType::Struct(id);
    let mut b = FuncBuilder::new(env, resolve, regs, String::new());
    let param = b.fresh_value(self_ty);
    let fields = structs.layouts[id.index()].fields.clone();
    match recursive_disposal_path(self_ty, regs) {
        // A struct's own path always starts at one of its own fields: only an
        // enum expands into a `Branch`, and this level is not one.
        Some(path) => {
            // R1a: the aggregate-staging transform is gated OFF here; the fused
            // destructor loop is correct by its own ordered hoisted-slot reuse
            // and must stay byte-for-byte.
            let node = b.begin_loop(&[param], false)[0];
            b.emit_field_level(node, &fields, &path);
            b.finalize_loop();
        }
        None => b.drop_level_fields(param, &fields, None),
    }
    // A back-edge or a dispatch arm already sealed the final block, and a
    // second seal would emit a duplicate `BlockId`.
    if !b.terminated {
        b.seal_block(Terminator::Ret(None));
    }
    IrFunc {
        name: struct_drop_symbol(id, structs.layouts[id.index()].drop_generation),
        params: vec![self_ty],
        ret: None,
        blocks: b.blocks,
        value_types: b.value_types,
    }
}

/// R2 (slice 8b): struct `id`'s destructor *is* the user's `drop` body. Lowered
/// by exactly the machinery any other word body gets, then renamed to the
/// destructor symbol every existing call site already calls — the override
/// replaces `synthesize_struct_destructor`'s field glue rather than running
/// before or alongside it (R5), so there is no glue left to compose with.
///
/// Calls `lower_word_parts` directly rather than the `lower_word` convenience
/// wrapper: `lower_word` hardcodes `empty_combinators()`, which is correct for
/// its one remaining caller (the REPL's `eval_def`, which cannot yet retain
/// any combinator to call) but was silently wrong here — a drop override's
/// body can call a combinator like any other word body, and a native build's
/// `combinator_bodies` map already exists by the time this runs.
#[allow(clippy::too_many_arguments)]
fn synthesize_struct_destructor_override(
    id: StructId,
    word: &WordDef,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
    combinators: &HashMap<String, Vec<Term>>,
) -> Vec<IrFunc> {
    // R9: element 0 is the override body itself, renamed to the destructor
    // symbol every call site already targets; any materialized quotations it
    // produced follow, lowered under their own symbols.
    let mut funcs = lower_word_parts(
        &word.name,
        &word.effect,
        &word.body,
        crate::check::has_self_tail_call(word),
        env,
        resolve,
        regs,
        empty_instantiations(),
        empty_builtin_overloads(),
        empty_poly_arities(),
        combinators,
        EnvPlan::None,
    );
    funcs[0].name = struct_drop_symbol(id, regs.structs.layouts[id.index()].drop_generation);
    funcs
}

/// R12 (Phase 4): synthesize enum `id`'s destructor, called by `drop` on any
/// value of that type. Unlike the struct case (a fixed field list), an enum's
/// active variant is a runtime fact, so the destructor tag-dispatches (its
/// own `Jnz` chain, the same compare-chain shape `lower_clauses` uses for a
/// clause-style word's scrutinee) and then drops only the dispatched variant's
/// linear payload fields. Every variant gets its own block even if none of its
/// fields are linear (an empty block that just returns), so the dispatch
/// shape stays uniform regardless of which variants happen to carry a linear
/// field.
///
/// If the enum is on a disposal cycle, the whole destructor becomes one fused
/// loop: the dispatch reads the loop-carried node instead of the param, a
/// variant that continues toward `Self` walks its own route and back-edges to
/// the header, and a variant that does not returns. That is the base case, so
/// an inhabited recursive enum (`Nil`/`Cons`) disposes in constant stack,
/// however long the route back to itself is.
fn synthesize_enum_destructor(
    id: EnumId,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
) -> IrFunc {
    let enums = regs.enums;
    let self_ty = IrType::Enum(id);
    let mut b = FuncBuilder::new(env, resolve, regs, String::new());
    let param = b.fresh_value(self_ty);
    match recursive_disposal_path(self_ty, regs) {
        // An enum's own path is always one top-level `Branch` (`expand_path`
        // builds no other shape for an enum), so the loop's whole body is
        // that dispatch.
        Some(path) => {
            // R1a: aggregate staging gated OFF (see `synthesize_struct_destructor`).
            let node = b.begin_loop(&[param], false)[0];
            b.emit_path_steps(node, &path);
            b.finalize_loop();
        }
        // No cycle: the same dispatch, every variant a base case.
        None => {
            let base_cases = vec![None; enums.layouts[id.index()].variants.len()];
            b.emit_branch(param, id, &base_cases);
        }
    }

    IrFunc {
        name: enum_drop_symbol(id, enums.layouts[id.index()].drop_generation),
        params: vec![self_ty],
        ret: None,
        blocks: b.blocks,
        value_types: b.value_types,
    }
}

/// Copy the payload out (if linear), free the cell, then drop the copied-out
/// payload. The block is freed before the payload's own destructor runs, so
/// the free must come after the copyout (`load_owned_payload` never touches
/// the block again) but before the drop.
fn synthesize_cell_destructor(
    id: OwnedCellId,
    env: &HashMap<String, Arity>,
    resolve: Resolver,
    regs: Registries,
) -> IrFunc {
    let Registries {
        structs,
        enums,
        arrays,
        cells,
        ..
    } = regs;
    let mut b = FuncBuilder::new(env, resolve, regs, String::new());
    let param = b.fresh_value(IrType::OwnedCell(id));
    let payload_ty = cells.payload[id.index()];
    let is_linear = field_is_linear(payload_ty, structs, enums, arrays);
    let payload = is_linear.then(|| b.load_owned_payload(param, payload_ty));
    let size = b.value_size(payload_ty);
    let size_v = b.fresh_value(IrType::I64);
    b.push_instr(Instr::Const(size_v, size as i64));
    b.push_instr(Instr::Call(
        None,
        FREE_SYMBOL.to_string(),
        vec![param, size_v],
    ));
    if let Some(payload) = payload {
        b.emit_drop(payload);
    }
    b.seal_block(Terminator::Ret(None));
    IrFunc {
        name: cell_drop_symbol(id, cells.drop_generations[id.index()]),
        params: vec![IrType::OwnedCell(id)],
        ret: None,
        blocks: b.blocks,
        value_types: b.value_types,
    }
}
