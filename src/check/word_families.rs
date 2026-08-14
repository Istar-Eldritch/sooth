use super::*;

/// Every `&`-led word — the two prefix borrow operators and the
/// reference-mode accessor family. Returns `None` if `name` is not `&`-led
/// (the caller falls through to the ordinary lookup chain).
///
/// One spelling per shape *and* per mutability: the mutability is in the
/// token, never inherited from the receiver, so a reader gets reference-ness,
/// mutability and arity from the word alone. Every accessor consumes its
/// reference argument the way any word consumes its arguments.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_reference_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    scope: &Scope,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    live: &Liveness,
    at: usize,
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
            if let Some(id) = live_deriv(stack, scope, prov, live, at, |d| {
                d.owned_root.as_deref() == Some(rest) && (mutable || d.mutable)
            }) {
                // R24: if the conflicting borrow is live *only* because an
                // erased closure's surviving set keeps its holder alive past
                // its last syntactic use (R20), this borrow reads a captured
                // reference past that last use -> past-last-use, naming the
                // captured reference. A still-`Known` closure or a genuinely
                // live borrow keeps the conflicting-borrow wording.
                if let Some(captured) = past_last_use_capture(stack, scope, prov, live, at, id) {
                    return Err(past_last_use_error(ctx, span, &captured));
                }
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
                if let Some(origin) = aliasing_origin(stack, scope, prov, live, at, rest) {
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
#[allow(clippy::too_many_arguments)]
pub(super) fn check_access_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    refs: &[RefDecl],
    scope: &Scope,
    prov: &Provenance,
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
            // Review fix: `@` reads an *element* of an aggregate the
            // reference roots into (an array slot, a struct field), not the
            // whole named place, so it never sees a `surviving` set that
            // rides on a `Slot` directly (only a store onto the root binding,
            // R20, records one). Look the root binding up by the reference's
            // own provenance (`owned_root`, generic over array/struct/cell
            // chains) and forward its surviving set (if any) onto the
            // fetched value -- the same fetch-side half of the store-side
            // union `!`/`+!` already performs.
            let surviving = stack[n - 1]
                .deriv
                .and_then(|id| prov.deriv(id).owned_root.clone())
                .and_then(|root| scope.local(&root))
                .and_then(|b| b.surviving);
            stack.truncate(n - 1);
            stack.push(Slot {
                surviving,
                ..Slot::computed(referent)
            });
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
pub(super) fn check_str_word(
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

pub(super) fn check_array_word(
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
            // sweep never sees this shape. D2's shared gate owns this check
            // (no zero-safety: `fill` replicates a real seed, D4).
            check_array_element_gate(
                ctx,
                span,
                "fill",
                element.ty,
                ctx.structs(),
                ctx.enums(),
                arrays,
                false,
            )?;
            let array_ty = intern_array_type(arrays, element.ty, count_val as u32);
            // Review fix: forward the element's surviving set (R19) onto the
            // array -- `fill` replicates one closure-carrying element N
            // times, so the array as a whole is that closure's carrier
            // exactly as a struct/enum constructor's output is.
            let surviving = element.surviving;
            stack.truncate(n - 2);
            stack.push(Slot {
                surviving,
                ..Slot::computed(array_ty)
            });
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
pub(super) fn check_owned_cell_word(
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
            // Review fix: forward the payload's surviving set (R19) onto the
            // cell -- `^` allocating a closure-carrying value must keep it
            // visible to R22's return guard exactly as a struct/enum
            // constructor does.
            let surviving = stack[n - 1].surviving;
            let cell_ty = intern_owned_cell_type(cells, payload);
            stack.truncate(n - 1);
            stack.push(Slot {
                surviving,
                ..Slot::computed(cell_ty)
            });
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
            // Review fix: forward the cell's own surviving set onto the
            // extracted payload -- the inverse of `^`'s forward above.
            let surviving = stack[n - 1].surviving;
            let payload = cells[id.index()].payload;
            stack.truncate(n - 1);
            stack.push(Slot {
                surviving,
                ..Slot::computed(payload)
            });
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
            // Review fix: forward the cell's surviving set (R19) onto the
            // peeked copy too, same as `^>`'s consuming fetch.
            stack.push(Slot {
                surviving: stack[n - 1].surviving,
                ..Slot::computed(payload)
            });
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
pub(super) fn check_struct_peek_word(
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
    // Review fix: forward the struct operand's surviving set (R19) onto the
    // peeked field -- a closure the struct carries stays visible through a
    // peek exactly as it would through the consuming getter below.
    stack.push(Slot {
        alias,
        surviving: top.surviving,
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
pub(super) fn check_struct_get_word(
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
    // Review fix: forward the struct operand's surviving set (R19) onto the
    // extracted field -- an aggregate field carrying a closure (a nested
    // struct/array/cell) must keep that closure visible to R22's return
    // guard past this getter.
    stack.push(Slot {
        alias,
        surviving: top.surviving,
        ..Slot::computed(field_ty)
    });
    Ok(Some(std::mem::take(stack)))
}

/// Apply a stack shuffle if `name` is one, returning `Some(stack)`; `None` if
/// the name is not a shuffle (the caller then looks it up in the env). Shuffles
/// move concrete slot types with no fixed signature: `dup` of a `bool` yields
/// two `bool`s, `swap` reorders whatever two types are on top, etc.
/// R1 (slice 8b, D2): the sole authority on whether a scoped name is visible to
/// a module. A name owned by `defining` is visible to `caller` iff `defining` is
/// the caller's own module, or the caller selectively imported that bare name
/// from that module. A qualified-only import (`import: lib "lib.sth"`) makes
/// nothing visible by bare name, so it is not a route here. Consumed by D1's
/// `drop` gate and (phase 3) 8a's operator fix; neither invents its own rule.
pub(super) fn is_name_visible_to_module(
    modules: &[ModuleInfo],
    caller: u32,
    defining: u32,
    name: &str,
) -> bool {
    defining == caller || modules[caller as usize].selective.get(name) == Some(&defining)
}

/// R12 (slice 8b, 8a): the operator overloads of `name` visible to the calling
/// module. `None` means "module scoping does not apply -- use the flat
/// `env.get(name)`": the REPL path (`ctx.modules()` is `None`) and a
/// single-module build, where `resolve_modules` leaves an operator decl bare
/// (`+`, not `+__m0`) so the flat lookup already finds the own overload. In a
/// multi-module build every operator decl is mangled per module, so a bare
/// lookup of `+` is `None`; assemble the caller's own overload (under
/// `mangle(name, M)`) plus one it selectively imported, membership decided by
/// `is_name_visible_to_module` (R1), never re-derived.
pub(super) fn scoped_operator_overloads(
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    name: &str,
) -> Option<Vec<Overload>> {
    // Only a builtin operator name is left bare by `resolve_modules` and so
    // scoped here; every other bare call was already rewritten to its mangled
    // spelling, and re-mangling that would only miss (`foo__m0__m0`). This is
    // also what keeps the fall-through env-call path (which reads this result)
    // from corrupting an ordinary word's candidate lookup.
    if !BUILTIN_TABLE.contains_key(name) {
        return None;
    }
    let modules = ctx.modules()?;
    if modules.len() < 2 {
        return None;
    }
    let caller = ctx.module();
    let mut defining = vec![caller];
    if let Some(&k) = modules[caller as usize].selective.get(name) {
        defining.push(k);
    }
    let mut out: Vec<Overload> = Vec::new();
    for d in defining {
        if is_name_visible_to_module(modules, caller, d, name) {
            if let Some(cands) = env.get(&crate::resolve::mangle(name, d)) {
                out.extend(cands.iter().cloned());
            }
        }
    }
    Some(out)
}

/// R4 (slice 8b, D1): reject a bare `drop` of an imported resource type whose
/// `drop` override is not visible to the calling module. The name checked is the
/// struct's demangled source spelling, since `ModuleInfo::selective` is keyed by
/// source names while `decl.name` is mangled (`Res__m1`) in a >=2-module build.
///
/// The visible-from module is `span.module`, the module the term was *written*
/// in, not `ctx.module()`, the module it is being checked in. The two differ
/// only under splicing, where `ctx.module()` is the splice destination (a
/// library's own module for a combinator body), which would judge the caller's
/// `drop` against the library's imports.
pub(super) fn check_drop_import_visibility(
    ctx: &Ctx,
    span: Span,
    m: &[ModuleInfo],
    decl: &StructDecl,
) -> Result<(), String> {
    let source = crate::resolve::demangle_word(&decl.name);
    if is_name_visible_to_module(m, span.module, decl.module, source) {
        Ok(())
    } else {
        Err(drop_import_visibility_error(ctx, span, m, decl, source))
    }
}

/// R5 (slice 8b): the located diagnostic for a `drop` whose destructor lives in
/// a module the caller imported qualified-only. Names the demangled type under
/// the qualifier the caller binds it (the qualifier whose import maps to the
/// declaring module) and the remedy: import the type by name. The `Ctx::Line`
/// arm drops the enclosing-word clause, though the REPL path never reaches the
/// gate (`ctx.modules()` is `None` there, R8).
fn drop_import_visibility_error(
    ctx: &Ctx,
    span: Span,
    m: &[ModuleInfo],
    decl: &StructDecl,
    source: &str,
) -> String {
    // `span.module`, matching the gate above: deriving the qualifier from
    // `ctx.module()` would look it up in the splice destination's import map
    // and describe an import the authoring module never wrote.
    //
    // The gate above already decided accept/reject as a pure function of
    // `span.module`, so `ctx` cannot change whether this function runs at
    // all, only what it prints once it does: a quotation literal is
    // re-validated against each nesting level's own live row
    // (`check_poly_combinator_args`, row = `stack[..base]`), so `ctx` here can
    // be a splice destination several levels removed from `span.module`, and
    // that divergence is routine, not a corner case, including in programs
    // that pass. `drop_visibility_error_is_worded_from_the_authoring_module_
    // under_nested_splicing` (`tests/phase4_slice10b.rs`) pins the print side:
    // `m[ctx.module()]` would name a different qualifier than `m[caller]`
    // there, and the wrong one. `m[ctx.module()]` is always a valid index
    // either way, so there is no panic risk from the choice.
    let caller = span.module as usize;
    let qualifier = m[caller]
        .imports
        .iter()
        .find(|(_, &target)| target == decl.module)
        .map(|(q, _)| q.as_str());
    // `ModuleInfo` carries no name or path of its own (only its import map,
    // exports, and selective names), so a struct reachable only
    // *transitively* -- the caller imports some module that imports the
    // declaring one, but never imports the declaring one itself -- has no
    // qualifier to name here. Naming the struct's own bare name as if it
    // were a module qualifier (the prior behavior) reads as a valid import
    // spelling that silently fails; say plainly that the path is transitive
    // instead of fabricating one.
    let (ty_name, note) = match qualifier {
        Some(qualifier) => (
            format!("{qualifier}::{source}"),
            format!(
                "disposing it runs a `drop` destructor declared in module `{qualifier}`, which this module has not imported by name\n  note: add `{source}` to the import (`import: {qualifier} | {source} | \"...\"`), or dispose it in a module that declares `{source}`"
            ),
        ),
        None => (
            source.to_string(),
            format!(
                "disposing it runs a `drop` destructor declared in a module this module never imports directly -- it is only reachable transitively, through another module's import\n  note: import the module that declares `{source}` directly, then add `{source}` to that import"
            ),
        ),
    };
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: cannot `drop` a value of type `{ty_name}` in `{name}` (line {})\n  {note}",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: cannot `drop` a value of type `{ty_name}` (line {})\n  {note}",
            span.line
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
            &HashMap::new(),
        )
        .map(|(stack, _insts, _overloads)| stack)
    }
    // --- Slice 8b, D2/D1: the module-visibility primitive and `drop` gate. ---

    /// R1: the primitive is a pure function of `(modules, caller, defining,
    /// name)`; construct `ModuleInfo` directly rather than route through a build.
    #[test]
    fn visibility_own_module_is_visible() {
        let modules = vec![ModuleInfo::default(), ModuleInfo::default()];
        assert!(is_name_visible_to_module(&modules, 1, 1, "Res"));
    }
    #[test]
    fn visibility_selectively_imported_is_visible() {
        let mut caller = ModuleInfo::default();
        caller.selective.insert("Res".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        assert!(is_name_visible_to_module(&modules, 1, 0, "Res"));
    }
    #[test]
    fn visibility_qualified_only_import_is_not_visible() {
        // A qualified-only import binds the qualifier but no bare name.
        let mut caller = ModuleInfo::default();
        caller.imports.insert("lib".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        assert!(!is_name_visible_to_module(&modules, 1, 0, "Res"));
    }
    #[test]
    fn visibility_unrelated_module_is_not_visible() {
        let modules = vec![
            ModuleInfo::default(),
            ModuleInfo::default(),
            ModuleInfo::default(),
        ];
        assert!(!is_name_visible_to_module(&modules, 1, 2, "Res"));
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
                "only `call` accepts one",
            ),
            w(
                ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n: main ( -- ) [ + ] dupit drop drop ;\n",
                "passed to `dupit`",
                "only `call` accepts one",
            ),
            // check_outputs (R10).
            w(
                ": f ( -- i64 ) [ + ] ;\n",
                "declared output",
                "leaves a quotation on the stack",
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
    fn check_len_on_str_types_as_usize() {
        // R8: `check_str_word` claims `len` on a `str` operand before the
        // array path ever sees it, consuming the `str` and typing the result
        // `usize` (not the array `len`'s non-consuming signature).
        check_src(": w ( -- usize ) \"hi\" len ;").unwrap();
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
}
