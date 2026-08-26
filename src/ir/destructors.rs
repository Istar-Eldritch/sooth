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
    resolved_fields: &HashMap<Span, (StructId, usize)>,
    resolved_variant_fields: &HashMap<Span, (EnumId, usize, usize)>,
    combinators: &crate::check::CombinatorIndex,
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
                    resolved_fields,
                    resolved_variant_fields,
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
    /// (`slot_value`, no free).
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
/// `slot_value`/`emit_drop` the `drop` builtin uses, so "how a field is
/// disposed" stays in one place.
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
    resolved_fields: &HashMap<Span, (StructId, usize)>,
    resolved_variant_fields: &HashMap<Span, (EnumId, usize, usize)>,
    combinators: &crate::check::CombinatorIndex,
) -> Vec<IrFunc> {
    // R9: element 0 is the override body itself, renamed to the destructor
    // symbol every call site already targets; any materialized quotations it
    // produced follow, lowered under their own symbols.
    let mut funcs = lower_word_parts(
        &word.name,
        &word.effect,
        &word.body,
        crate::check::has_self_tail_call(word, combinators),
        None,
        env,
        resolve,
        regs,
        empty_instantiations(),
        empty_builtin_overloads(),
        empty_trait_calls(),
        empty_poly_calls(),
        resolved_fields,
        resolved_variant_fields,
        empty_poly_arities(),
        combinators,
        EnvPlan::None,
        empty_splice_records(),
        empty_splice_trait_calls(),
    );
    funcs[0].name = struct_drop_symbol(id, regs.structs.layouts[id.index()].drop_generation);
    funcs
}

/// R12 (Phase 4): synthesize enum `id`'s destructor, called by `drop` on any
/// value of that type. Unlike the struct case (a fixed field list), an enum's
/// active variant is a runtime fact, so the destructor tag-dispatches (its
/// own `Jnz` chain, the same compare-chain shape `lower_clauses` uses for an
/// eliminator's scrutinee) and then drops only the dispatched variant's
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::test_helpers::*;
    use crate::lexer::lex;

    #[test]
    fn two_drop_overloads_for_different_structs_do_not_collide() {
        // Criterion 16: neither override lands in the generic per-word
        // lowering pass (which would emit two QBE functions literally named
        // `drop`, the second colliding with the first), and each instead fills
        // its own struct's destructor symbol with its own body.
        let module = lower_src(
            "type: A x i64 ; type: B y i64 ; \
             : drop ( A -- ) | a | a A> . ; : drop ( B -- ) | b | b B> drop ; \
             : main ( -- ) 1 A drop 2 B drop ;",
        );
        assert!(
            module.funcs.iter().all(|f| f.name != "drop"),
            "an emitted IrFunc was literally named `drop`: {:?}",
            module.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let a = func(&module, &struct_drop_symbol(StructId::from_index(0), None));
        let b = func(&module, &struct_drop_symbol(StructId::from_index(1), None));
        // `A`'s body prints its field, `B`'s discards it: two distinct bodies
        // under two distinct symbols, not one shared or one clobbered.
        assert_eq!(count(a, |i| matches!(i, Instr::Print(_))), 1);
        assert_eq!(count(b, |i| matches!(i, Instr::Print(_))), 0);
    }

    #[test]
    fn lower_forces_drop_overload_linearity_even_when_check_never_ran() {
        // R1/R2 code-review fix: `lower` used to trust
        // `StructDecl::has_drop_overload`, a bit only `check::check` sets. A
        // module that reaches `lower` without having gone through `check`
        // (this test skips it, unlike `lower_src`) must still layout `File`
        // as linear and substitute the override, not silently emit nothing.
        let src = format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;");
        let tokens = lex(&src).unwrap();
        let module = crate::test_support::parse_with_core(&tokens).unwrap();
        let ir_module = lower(&module).unwrap();
        let file = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(call_symbols(func(&ir_module, "main")), vec![file.as_str()]);
        let dtor = func(&ir_module, &file);
        assert_eq!(count(dtor, |i| matches!(i, Instr::Print(_))), 1);
    }

    #[test]
    fn drop_of_an_overridden_struct_calls_its_destructor_symbol() {
        // R2: the whole of dispatch. `lower_call`'s `"drop"` arm is unchanged
        // and still symbol-based; forcing `is_linear` is what makes
        // `emit_drop`'s guard pass, and the substituted body is what the
        // symbol now resolves to.
        let module = lower_src(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let file = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(call_symbols(func(&module, "main")), vec![file.as_str()]);
        // The destructor is the user's body (one `.` of the field), not the
        // generic glue (which for an all-`Copy` struct emits nothing at all).
        let dtor = func(&module, &file);
        assert_eq!(count(dtor, |i| matches!(i, Instr::Print(_))), 1);
    }

    #[test]
    fn synthesize_destructor_of_resource_with_a_linear_field_uses_user_body_not_field_glue() {
        // Criterion 15/R5: the override runs *instead of* the field glue, not
        // before or alongside it. `Res`'s only field is linear, so the glue
        // would call `Inner`'s destructor symbol directly; the body hands the
        // field to `dispose` instead, so that call is the only one emitted.
        let module = lower_src(&format!(
            "{SPY_DEF}type: Inner s Spy ; type: Res i Inner ; \
             : dispose ( Inner -- ) drop ; \
             : drop ( Res -- ) | r | r Res> dispose ; \
             : main ( -- ) 1 Spy Inner Res drop ;"
        ));
        let inner = struct_drop_symbol(StructId::from_index(1), None);
        let res = struct_drop_symbol(StructId::from_index(2), None);
        assert_eq!(call_symbols(func(&module, &res)), vec!["dispose"]);
        // The glue that would have run is still emitted for `Inner` itself,
        // which has no override: `dispose`'s own `drop` calls it.
        assert_eq!(call_symbols(func(&module, "dispose")), vec![inner.as_str()]);
    }

    #[test]
    fn resource_field_disposed_via_its_own_drop_symbol() {
        // Criterion 13/R7 (ordinary composition): an enclosing struct's
        // per-field disposal calls each linear field's destructor rather than
        // inlining its fields, so a resource field is disposed through the
        // user's body with no new mechanism -- `Holder`'s glue prints nothing
        // itself, it calls `File`'s destructor, which prints.
        let module = lower_src(&format!(
            "{FILE_RESOURCE} type: Holder h File n i64 ; \
             : main ( -- ) 1 File 2 Holder drop ;"
        ));
        let file = struct_drop_symbol(StructId::from_index(0), None);
        let holder = func(&module, &struct_drop_symbol(StructId::from_index(1), None));
        assert_eq!(call_symbols(holder), vec![file.as_str()]);
        assert_eq!(count(holder, |i| matches!(i, Instr::Print(_))), 0);
    }

    #[test]
    fn synthesize_destructor_excludes_override_structs_from_a_fused_disposal_path() {
        // Criterion 14/R7 (the disposal-cycle case): `Chain`'s cycle runs back
        // to itself *through* `Res`. The fused loop inlines every intermediate
        // type's field projection instead of calling its destructor, so
        // fusing this cycle would bypass `Res`'s override and leak its
        // resource silently. With `Res` overridden the search stops there, so
        // `Chain` falls back to per-field disposal and reaches the override
        // through its own symbol.
        let src = "type: Res fd i64 next ^Chain ; type: Chain r Res ; : main ( -- ) ;";
        let plain = Probe::new(src);
        assert!(
            plain.path(plain.struct_ty("Chain")).is_some(),
            "without an override, `Chain` fuses its cycle into one loop"
        );

        let p = Probe::with_overrides(src, &["Res"]);
        assert_eq!(p.path(p.struct_ty("Chain")), None);
        // The search's own root is unaffected: whether `Res` is on a cycle is
        // moot, since its destructor is its override either way (R2).
        assert!(p.path(p.struct_ty("Res")).is_some());

        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let chain = synthesize_struct_destructor(p.struct_id("Chain"), &env, &resolve, p.regs());
        assert_eq!(
            call_symbols(&chain),
            vec![struct_drop_symbol(p.struct_id("Res"), None).as_str()]
        );
    }

    #[test]
    fn recursive_type_destructor_is_not_transformed() {
        // R1a: the fused iterative destructor's `begin_loop` is gated OFF, so a
        // recursive type's synthesized destructor keeps its one header phi for
        // the carried node (R2 would drop it to zero) and gains no entry-block
        // init Blit (R3's blit is the only Blit the transform routes to the
        // entry block; the destructor's own copy-out lands in a body block).
        // This is the check that is red when the gate is missing.
        let ir = lower_src(
            "type: Res n i64 ;\n\
             : drop ( Res -- ) | r | r Res> 5000 add . ;\n\
             : mkres ( i64 -- Res ) | n | n Res ;\n\
             type: List | Nil | Cons v Res next ^List ;\n\
             : w ( -- ) ;",
        );
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_enum_drop_0")
            .expect("a fused destructor was synthesized for the recursive enum");
        let header = loop_header(dtor);
        let phis = header_phis(header_block(dtor, header));
        assert_eq!(
            phis.len(),
            1,
            "the ungated-off destructor keeps its one carried-node header phi"
        );
        let entry = &dtor.blocks[0];
        assert!(
            !entry.instrs.iter().any(|i| matches!(i, Instr::Blit(..))),
            "the destructor gains no entry-block init blit (R1a gate holds)"
        );
    }

    #[test]
    fn lower_appends_one_destructor_func_per_linear_struct_only() {
        // R12: a synthesized destructor exists for every linear struct type,
        // and only those (a Copy struct needs no glue, so gets no function).
        // `Plain` (index 1, Copy) gets no destructor; `Holds` (index 2,
        // linear) does.
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Plain x i64 y i64 ; \
             type: Holds a Spy b i64 ; \
             : w ( -- ) ;"
        ));
        assert!(ir.funcs.iter().any(|f| f.name == "sooth_struct_drop_2"));
        assert!(!ir.funcs.iter().any(|f| f.name == "sooth_struct_drop_1"));
    }

    #[test]
    fn lower_drop_of_whole_linear_struct_calls_its_synthesized_destructor() {
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Holds a Spy b i64 ; : w ( -- ) 1 Spy 2 Holds drop ;"
        ));
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let calls: Vec<&String> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, args) if args.len() == 1 => Some(sym),
                _ => None,
            })
            .collect();
        let holds_drop = struct_drop_symbol(StructId::from_index(1), None);
        assert_eq!(calls, vec![holds_drop.as_str()]);
    }

    #[test]
    fn synthesized_struct_destructor_drops_linear_fields_in_declaration_order() {
        // R12: struct -> drop its linear fields in declaration order. `Holds`
        // has a linear field (`a`) then a Copy one (`b`), so the destructor
        // calls `Spy`'s destructor exactly once, for `a`.
        let ir = lower_src(&format!("{SPY_DEF}type: Holds a Spy b i64 ; : w ( -- ) ;"));
        let holds_drop = struct_drop_symbol(StructId::from_index(1), None);
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == holds_drop)
            .expect("a destructor was synthesized for the linear struct");
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(calls, vec![spy_drop.as_str()]);
    }

    #[test]
    fn lower_appends_a_destructor_func_for_every_cell_even_a_copy_payload() {
        // R8: unlike the struct/enum filters above, *every* cell gets a
        // destructor, because `drop` on a cell must free it whatever its
        // payload is. `^i64`'s payload is Copy and it still gets one.
        let ir = lower_src(": w ( -- ) 5 ^ drop ;");
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_cell_drop_0")
            .expect("a Copy-payload cell still gets a destructor");
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            vec![FREE_SYMBOL],
            "a Copy payload frees and nothing else"
        );
    }

    #[test]
    fn synthesized_cell_destructor_frees_before_dropping_a_linear_aggregate_payload() {
        // An aggregate payload is copied out of the cell (a Blit), then
        // the block is freed, and only then does the copy's own drop
        // glue run. The `^Spy` golden covers the scalar payload at
        // runtime; this pins the aggregate path, where the copy-out must
        // still complete before anything else touches the block or the copy.
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Holds a Spy b i64 ; : w ( -- ) 1 Spy 2 Holds ^ drop ;"
        ));
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_cell_drop_0")
            .expect("a destructor was synthesized for the cell");
        let is = instrs(dtor);
        let blit_at = is
            .iter()
            .position(|i| matches!(i, Instr::Blit(..)))
            .expect("a copy-out Blit");
        let calls: Vec<(usize, &String)> = is
            .iter()
            .enumerate()
            .filter_map(|(at, i)| match i {
                Instr::Call(None, sym, _) => Some((at, sym)),
                _ => None,
            })
            .collect();
        let holds_drop = struct_drop_symbol(StructId::from_index(1), None);
        assert_eq!(
            calls
                .iter()
                .map(|(_, sym)| sym.as_str())
                .collect::<Vec<_>>(),
            vec![FREE_SYMBOL, holds_drop.as_str()],
            "the cell frees, then the payload's own destructor runs"
        );
        assert!(
            blit_at < calls[0].0,
            "the payload must be copied out before the block is freed: blit at {blit_at}, free at {}",
            calls[0].0
        );
    }

    // Phase 3 Slice 1, Phase 4: the synthesized enum destructor's own tag
    // dispatch (structural, not full-stdout: `tests/phase0.rs` covers the
    // 2-variant runtime behavior; these pin the shapes it doesn't reach).

    #[test]
    fn synthesized_enum_destructor_newtype_skips_the_tag_compare() {
        // R7/R12: a single-variant enum (n == 1) has nothing to dispatch on,
        // so the destructor jumps straight to the one variant block instead
        // of loading a tag and comparing it (the `n == 1` branch of
        // `dispatch_on_tag`, otherwise unreached by the 2-variant goldens).
        let ir = lower_src(&format!("{SPY_DEF}type: Box | Full v Spy ; : w ( -- ) ;"));
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_enum_drop_0")
            .expect("a destructor was synthesized for the linear enum");
        assert_eq!(count(dtor, |i| matches!(i, Instr::Cmp(..))), 0);
        assert_eq!(
            dtor.blocks.len(),
            2,
            "a bare `Jmp` to the one variant block, no compare block"
        );
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(calls, vec![spy_drop.as_str()]);
    }

    #[test]
    fn synthesized_enum_destructor_three_variants_chains_through_a_middle_block() {
        // R7/R12: with 3 variants the compare chain has an intermediate block
        // between the first and last compare (`vi < n - 2` in
        // `dispatch_on_tag`), never built by the 2-variant goldens. Each of
        // the 3 variants gets its own block; only `Full`'s carries a drop.
        let ir = lower_src(&format!(
            "{SPY_DEF}type: Item | Empty | Full v Spy | Named n i64 ; : w ( -- ) ;"
        ));
        let dtor = ir
            .funcs
            .iter()
            .find(|f| f.name == "sooth_enum_drop_0")
            .expect("a destructor was synthesized for the linear enum");
        assert_eq!(dtor.blocks.len(), 5, "2 compares + 3 variant blocks");
        assert_eq!(count(dtor, |i| matches!(i, Instr::Cmp(..))), 2);
        let calls: Vec<&String> = instrs(dtor)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, _) => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(calls, vec![spy_drop.as_str()]);
    }

    // Unit-level coverage of `recursive_disposal_path`'s path-finding: which
    // steps it finds for a shape, distinct from the runtime goldens in
    // tests/phase0.rs that prove those shapes actually dispose correctly.

    #[test]
    fn recursive_disposal_path_finds_indirect_nested_mutual_and_composed_cycles() {
        // The wrapper-struct list: the cell is one byval struct hop away from
        // the enum that owns it, so the path is a tag dispatch, a projection
        // into `Wrap`, then the unwrap.
        let p = Probe::new(
            "type: Wrap v i64 next ^List ;\n\
             type: List | Nil | Cons w Wrap ;\n\
             : main ( -- ) ;",
        );
        let list = p.enum_ty("List");
        assert_eq!(
            p.path(list),
            Some(vec![PathStep::Branch {
                enum_id: p.enum_id("List"),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Project { field: 0 },
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(list),
                        },
                    ]),
                ],
            }])
        );
        // The same cycle rooted at `Wrap` instead: one rotation of it, the
        // dispatch now mid-path (every type on the cycle gets its own
        // loop, entered from its own shape).
        assert_eq!(
            p.path(p.struct_ty("Wrap")),
            Some(vec![
                PathStep::Unwrap {
                    field: Some(1),
                    cell: p.cell(list),
                },
                PathStep::Branch {
                    enum_id: p.enum_id("List"),
                    variants: vec![None, Some(vec![PathStep::Project { field: 0 }])],
                },
            ])
        );

        // `^^Self`: the outer unwrap names the field, the inner one cannot
        // (the current type *is* the cell at that point).
        let p = Probe::new(
            "type: L | Nil | Cons n i64 next ^^L ;\n\
             : main ( -- ) ;",
        );
        let l = p.enum_ty("L");
        let inner = p.cell(l);
        assert_eq!(
            p.path(l),
            Some(vec![PathStep::Branch {
                enum_id: p.enum_id("L"),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(IrType::OwnedCell(inner)),
                        },
                        PathStep::Unwrap {
                            field: None,
                            cell: inner,
                        },
                    ]),
                ],
            }])
        );

        // The mutual A/B chain, from both directions: `A` dispatches at entry,
        // `B` (a plain struct, no tag of its own) dispatches mid-path.
        let p = Probe::new(
            "type: A | ANil | ACons x i64 next ^B ;\n\
             type: B y i64 z ^A ;\n\
             : main ( -- ) ;",
        );
        let (a, b) = (p.enum_ty("A"), p.struct_ty("B"));
        assert_eq!(
            p.path(a),
            Some(vec![PathStep::Branch {
                enum_id: p.enum_id("A"),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(b),
                        },
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(a),
                        },
                    ]),
                ],
            }])
        );
        assert_eq!(
            p.path(b),
            Some(vec![
                PathStep::Unwrap {
                    field: Some(1),
                    cell: p.cell(a),
                },
                PathStep::Branch {
                    enum_id: p.enum_id("A"),
                    variants: vec![
                        None,
                        Some(vec![PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(b),
                        }]),
                    ],
                },
            ])
        );

        // Composition: a wrapper struct sitting inside a two-type cycle, so
        // one path threads three unwraps through three distinct types.
        let p = Probe::new(
            "type: P q ^W ;\n\
             type: W m i64 next ^Q ;\n\
             type: Q r ^P ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(
            p.path(p.struct_ty("P")),
            Some(vec![
                PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(p.struct_ty("W")),
                },
                PathStep::Unwrap {
                    field: Some(1),
                    cell: p.cell(p.struct_ty("Q")),
                },
                PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(p.struct_ty("P")),
                },
            ])
        );
    }

    #[test]
    fn recursive_disposal_path_finds_multi_variant_and_enum_enum_mutual_cycles() {
        // Two independently recursive variants: both continue, because an
        // enum's variants are mutually exclusive at runtime and so are not
        // the simultaneously-live branching case a struct's own field choice
        // must narrow. Collapsing to one would regress a program that
        // already disposes in constant stack.
        let p = Probe::new(
            "type: T | Nil | X n i64 next ^T | Y m i64 next ^T ;\n\
             : main ( -- ) ;",
        );
        let t = p.enum_ty("T");
        let step = vec![PathStep::Unwrap {
            field: Some(1),
            cell: p.cell(t),
        }];
        assert_eq!(
            p.path(t),
            Some(vec![PathStep::Branch {
                enum_id: p.enum_id("T"),
                variants: vec![None, Some(step.clone()), Some(step)],
            }])
        );

        // The enum/enum mutual pair: two nested `Branch` steps, the inner one
        // dispatched partway along the path rather than at the entry.
        let p = Probe::new(
            "type: A | ANil | ACons x i64 next ^B ;\n\
             type: B | BNil | BCons y i64 next ^A ;\n\
             : main ( -- ) ;",
        );
        let (a, b) = (p.enum_ty("A"), p.enum_ty("B"));
        assert_eq!(
            p.path(a),
            Some(vec![PathStep::Branch {
                enum_id: p.enum_id("A"),
                variants: vec![
                    None,
                    Some(vec![
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(b),
                        },
                        PathStep::Branch {
                            enum_id: p.enum_id("B"),
                            variants: vec![
                                None,
                                Some(vec![PathStep::Unwrap {
                                    field: Some(1),
                                    cell: p.cell(a),
                                }]),
                            ],
                        },
                    ]),
                ],
            }])
        );
    }

    #[test]
    fn recursive_disposal_path_rejects_non_cyclic_and_misleading_shapes() {
        // No cell at all: nothing to walk.
        let p = Probe::new(&format!(
            "{SPY_DEF}type: Plain x i64 y Spy ;\n: main ( -- ) ;"
        ));
        assert_eq!(p.path(p.struct_ty("Plain")), None);

        // The bait is the *last* field, which is where the reverse-order scan
        // starts, and the genuine edge is indirect, so the direct-field tier
        // cannot short-circuit past it: the scan must try `bait`, walk into
        // `Bait` and `Leafy`, fail, and back up to `good`. A greedy search
        // that committed to the first cell field it saw would return `None`.
        let p = Probe::new(
            "type: Leafy v i64 ;\n\
             type: Bait c ^Leafy ;\n\
             type: Hop n ^Node ;\n\
             type: Node good Hop bait ^Bait ;\n\
             : main ( -- ) ;",
        );
        let node = p.struct_ty("Node");
        assert_eq!(
            p.path(node),
            Some(vec![
                PathStep::Project { field: 0 },
                PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(node),
                },
            ])
        );

        // `^^Other`: the walk does step through the inner cell (that is how
        // `^^Self` is found at all), and still bottoms out in a dead end.
        let p = Probe::new(
            "type: Other v i64 ;\n\
             type: Twice c ^^Other ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(p.path(p.struct_ty("Twice")), None);

        // Two unrelated self-recursive types: each finds its own edge and
        // neither path wanders into the other type.
        let p = Probe::new(
            "type: R1 n ^R1 ;\n\
             type: R2 n ^R2 ;\n\
             : main ( -- ) ;",
        );
        for name in ["R1", "R2"] {
            let ty = p.struct_ty(name);
            assert_eq!(
                p.path(ty),
                Some(vec![PathStep::Unwrap {
                    field: Some(0),
                    cell: p.cell(ty),
                }])
            );
        }
    }

    #[test]
    fn recursive_disposal_path_prefers_direct_field_over_later_indirect_one() {
        // `a` is a direct `^Self` field; `b` is declared after it and also
        // reaches `Self`, but only by way of `Wrap`'s own cell field. Without
        // a preferred tier for direct fields, the reverse scan tries `b`
        // first and finds it succeeds, silently swapping in the longer route
        // and lengthening every iteration of the fused loop.
        let p = Probe::new(
            "type: Wrap v i64 n ^List ;\n\
             type: List a ^List b Wrap ;\n\
             : main ( -- ) ;",
        );
        let list = p.struct_ty("List");
        assert_eq!(
            p.path(list),
            Some(vec![PathStep::Unwrap {
                field: Some(0),
                cell: p.cell(list),
            }])
        );

        // The same trap one level up, between an enum's variants: each
        // variant picks its own edge independently, `Direct`'s direct one and
        // `Indirect`'s route through `Wrap`.
        let p = Probe::new(
            "type: Wrap v i64 n ^E ;\n\
             type: E | Nil | Direct d ^E | Indirect w Wrap ;\n\
             : main ( -- ) ;",
        );
        let e = p.enum_ty("E");
        assert_eq!(
            p.path(e),
            Some(vec![PathStep::Branch {
                enum_id: p.enum_id("E"),
                variants: vec![
                    None,
                    Some(vec![PathStep::Unwrap {
                        field: Some(0),
                        cell: p.cell(e),
                    }]),
                    Some(vec![
                        PathStep::Project { field: 0 },
                        PathStep::Unwrap {
                            field: Some(1),
                            cell: p.cell(e),
                        },
                    ]),
                ],
            }])
        );
    }
}
