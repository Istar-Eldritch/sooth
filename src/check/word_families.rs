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
pub(super) fn check_drop_import_visibility(
    ctx: &Ctx,
    span: Span,
    m: &[ModuleInfo],
    decl: &StructDecl,
) -> Result<(), String> {
    let source = crate::resolve::demangle_word(&decl.name);
    if is_name_visible_to_module(m, ctx.module(), decl.module, source) {
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
    let caller = ctx.module() as usize;
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
