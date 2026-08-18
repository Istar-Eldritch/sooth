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
    resolved_fields: &mut HashMap<Span, (StructId, usize)>,
    resolved_variant_fields: &mut HashMap<Span, (EnumId, usize, usize)>,
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
            // Review fix (P7 slice 1): forward the receiver's region
            // unchanged onto the narrowed reference -- an over-approximation
            // (every index looks like it aliases the whole array), but
            // without it a receiver whose only live reference chains through
            // this word would look unborrowed to the consume-time check
            // below.
            let alias = stack[n - 2].alias;
            stack.truncate(n - 2);
            stack.push(Slot {
                alias,
                ..Slot::derived(out, deriv)
            });
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
            // Review fix (P7 slice 1): forward the receiver's region, same
            // reasoning as `&>` above.
            let alias = stack[n - 1].alias;
            stack.truncate(n - 1);
            stack.push(Slot {
                alias,
                ..Slot::derived(out, deriv)
            });
        }
        _ => {
            // P7 slice 1 (D1/R1): a receiver-directed projection. `&hp` names
            // no type, so the field resolves against the type already on the
            // stack; the resolution is recorded per call site for lowering,
            // which has no checker stack to re-derive it from (R2). Tried
            // ahead of the local/static borrow below: the receiver wins (R3).
            if let Some(out) = check_field_projection(
                name,
                rest,
                mutable,
                span,
                stack,
                ctx,
                scope,
                refs,
                prov,
                live,
                at,
                resolved_fields,
                resolved_variant_fields,
            )? {
                return Ok(Some(out));
            }
            // Everything else is a prefix borrow of a place: a bound local,
            // or (R1) a module static.
            if rest.is_empty() {
                return Err(borrow_of_non_place_error(
                    ctx,
                    span,
                    name,
                    "it names nothing (a bare sigil cannot borrow whatever happens to be on the stack)",
                ));
            }
            // R1's resolution order: a bound local first, then a static of
            // this module, then nothing. A local shadowing a static therefore
            // wins, exactly as it does for every other name.
            let mut static_root = false;
            let referent_ty = if let Some(local_ty) = scope.local_type(rest) {
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
                // there. A local consumed earlier holds nothing, and borrowing
                // it would read (and project through) storage its owner has
                // already freed.
                if let Some(site) = scope.moves.moved_site(rest) {
                    return Err(use_after_move_error(ctx, span, rest, local_ty, site));
                }
                local_ty
            } else if let Some(static_ty) = ctx.static_type(rest) {
                // R1: a *scalar* static is borrowable though a scalar local is
                // not -- a static has a data-symbol address to hand out, a
                // scalar local has none. Nothing else about the borrow
                // differs: the exclusivity scans below run verbatim, keyed on
                // the same `owned_root` an owned local's borrow carries (R3).
                // A static is never owned, moved or dropped, so the move and
                // aggregate-only gates above have nothing to say about it.
                static_root = true;
                static_ty
            } else {
                // `rest` reaches here mangled whenever it named a type
                // (`&!Point__m0>z`, an accessor whose field does not exist),
                // so it is demangled like every other rendered name.
                let shown = crate::resolve::demangle_call(rest);
                let found = if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    format!("`{shown}` is a literal, not a local")
                } else {
                    format!("`{shown}` is not a local in scope")
                };
                return Err(borrow_of_non_place_error(ctx, span, name, &found));
            };
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
            let out = intern_ref_type(refs, referent_ty, mutable);
            let deriv = prov.borrow(rest, mutable, static_root, span);
            stack.push(Slot::derived(out, Some(deriv)));
        }
    }
    Ok(Some(std::mem::take(stack)))
}

/// P7 slice 1 (D2/R1): `&f` / `&!f`, a field projection resolved against the
/// receiver on top of the stack rather than against a type name baked into the
/// word. `None` when the stack top is not a struct or variant (or a reference
/// to one) with a field `field`, so the caller falls through to the
/// prefix-borrow chain and an unrelated `&x` keeps its existing meaning.
///
/// Two effects, by receiver:
///
/// - `( &S -- &A )` / `( &!S -- &!A )`, **consuming**. A reference left on the
///   stack is a surplus value, so a non-consuming chain would strand an
///   intermediate at every step of `u &stats &hp @`.
/// - owned `S`: `( S -- S &A )`, **non-consuming**. Consuming the receiver
///   would oblige this word to dispose the fields it did not project, which is
///   exactly the implicit disposal this slice deletes.
///
/// R4/Phase 6 slice 3 (R6): a variant receiver (`Type::Variant`) is resolved
/// the same way, and records its resolution into `resolved_variant_fields`
/// (`EnumId`-keyed, mirroring `resolved_fields`), which `lower_reference_word`
/// reads back.
#[allow(clippy::too_many_arguments)]
fn check_field_projection(
    name: &str,
    field: &str,
    mutable: bool,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    scope: &Scope,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    live: &Liveness,
    at: usize,
    resolved_fields: &mut HashMap<Span, (StructId, usize)>,
    resolved_variant_fields: &mut HashMap<Span, (EnumId, usize, usize)>,
) -> Result<Option<Vec<Slot>>, String> {
    // D1's grammar makes a receiver-directed field an ordinary identifier,
    // never an accessor, and `>` is not a lexer delimiter -- so a leftover
    // `&!Type>field` arrives here as one token whose whole text would be read
    // as a field name, answering "`Point` declares no field named `Point>z`"
    // and implying that spelling could name a field. It cannot: the fused
    // accessors are retired, so the honest answer is the borrow chain's
    // "not a place" / "not a local in scope" below.
    if field.contains('>') {
        return Ok(None);
    }
    let n = stack.len();
    if n < 1 {
        return Ok(None);
    }
    let top = stack[n - 1];
    if top.quot.is_some() {
        return Ok(None);
    }
    let (referent, recv_mut) = match ref_parts(top.ty, refs) {
        Some((referent, recv_mut)) => (referent, Some(recv_mut)),
        None => (top.ty, None),
    };
    // R4: a variant receiver is resolved by the same rule as a struct one --
    // the fields and display name just come from a different declaration
    // table. Its resolution lands in `resolved_variant_fields`
    // (`EnumId`-keyed) below, not `resolved_fields` (`StructId`-keyed).
    let (fields, receiver_name): (&[(String, Type)], &str) = match referent {
        Type::Struct(id, _) => (
            &ctx.structs()[id.index()].fields,
            ctx.structs()[id.index()].name_static,
        ),
        Type::Variant(id, vi, _) => {
            let variant = &ctx.enums()[id.index()].variants[vi];
            (&variant.fields, variant.display_static)
        }
        _ => return Ok(None),
    };
    // `field` arrives as `resolve` left it, which mangles it whenever it
    // matches a static of this module (R2) -- struct field names are never
    // mangled, so the lookup below has to compare against the demangled
    // spelling or a static's mangled name can never be seen to collide with
    // a field of the same source name.
    let field_name = crate::resolve::demangle_call(field);
    let field_pos = fields.iter().position(|(f, _)| f == field_name.as_ref());
    // R3: the receiver is tried first, but a local/static of the same name
    // still has a say -- present on both sides it is a shadow error, present
    // only there (the receiver lacking the field) it is the fallback that
    // keeps `&n` meaning what it always meant, and present on neither side
    // the diagnostic can finally name the receiver type.
    let existing_place = scope.local_type(field).is_some() || ctx.static_type(field).is_some();
    let fi = match field_pos {
        Some(_fi) if existing_place => {
            return Err(projection_field_shadowed_by_local_error(
                ctx,
                span,
                name,
                receiver_name,
                field_name.as_ref(),
            ));
        }
        Some(fi) => fi,
        None if existing_place => return Ok(None),
        None => {
            return Err(projection_unknown_field_error(
                ctx,
                span,
                name,
                receiver_name,
                field_name.as_ref(),
            ));
        }
    };
    let field_ty = fields[fi].1;
    match recv_mut {
        Some(recv_mut) => {
            // Full-type equality: a wrong mutability is a type mismatch
            // between the two interned reference shapes.
            if recv_mut != mutable {
                let want = intern_ref_type(refs, referent, mutable);
                return Err(type_mismatch_error(ctx, span, name, want, top.ty));
            }
            let out = intern_ref_type(refs, field_ty, mutable);
            let deriv = prov.project(top.deriv);
            // Review fix (P7 slice 1): forward the receiver's region, same
            // reasoning as `&>` above -- a receiver reached only through a
            // chain of projections (`&!s &!v`) would otherwise look
            // unborrowed to the consume-time check.
            let alias = top.alias;
            stack.truncate(n - 1);
            stack.push(Slot {
                alias,
                ..Slot::derived(out, deriv)
            });
        }
        None => {
            // The receiver stays, so this arm needs region machinery: the
            // projection is interned as a child region of the receiver's own,
            // which is what makes two overlapping projections off one
            // *anonymous* receiver (`p &!hp swap &!hp`) a checked conflict
            // rather than an unchecked pair of aliases. Nothing is gated on
            // `is_copy`: a projection borrows the field rather than
            // duplicating its value, so a linear field is not special here
            // (`@`/`!` still refuse to move one through it).
            let alias = projected_region(&mut stack[n - 1], field, span, prov);
            if let Some(origin) =
                overlapping_projection(&stack[..n - 1], scope, prov, live, at, alias.set, mutable)
            {
                return Err(conflicting_projection_error(
                    ctx, span, name, mutable, origin,
                ));
            }
            let out = intern_ref_type(refs, field_ty, mutable);
            stack.push(Slot {
                alias: Some(alias),
                ..Slot::computed(out)
            });
        }
    }
    match referent {
        Type::Struct(id, _) => {
            resolved_fields.insert(span, (id, fi));
        }
        Type::Variant(id, vi, _) => {
            resolved_variant_fields.insert(span, (id, vi, fi));
        }
        _ => {}
    }
    Ok(Some(std::mem::take(stack)))
}

/// Where a live reference denoting a region overlapping a new projection's is,
/// when one exists and the pair cannot coexist (a new `&!` conflicts with any
/// live projection, a new `&` only with a live mutable one). Only
/// *reference*-typed values are candidates: the receiver and its copies denote
/// the same regions but hold no borrow, and consuming one is what ends it.
pub(super) fn overlapping_projection<'a>(
    below: &[Slot],
    scope: &'a Scope,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
    set: AliasSetId,
    mutable: bool,
) -> Option<AliasOrigin<'a>> {
    let conflicts = |ty: Type, alias: Option<AliasSetId>| {
        let Type::Ref(_, other_mutable, _) = ty else {
            return false;
        };
        (mutable || other_mutable) && alias.is_some_and(|other| prov.alias_sets_overlap(set, other))
    };
    if let Some(b) = scope
        .bound
        .iter()
        .find(|b| !live.dead(&b.name, at) && conflicts(b.ty, b.aliases))
    {
        return Some(AliasOrigin::Name(&b.name));
    }
    below
        .iter()
        .find(|slot| conflicts(slot.ty, slot.alias.map(|a| a.set)))
        .and_then(|slot| slot.alias)
        .map(|alias| AliasOrigin::Stack(alias.span))
}

/// P7 slice 1 review fix: the live reference that consuming `consumed` would
/// strand, when one exists. The guard at every point a place with a region --
/// named or, via the owned-receiver projection arm, anonymous -- is consumed
/// for good (`drop`, a moving word call, `^`, a moving field getter). That arm
/// produces a reference with a region (`Slot.alias`) but no `Deriv` (there is
/// no place name to root one on), so it is invisible to the named-place
/// consume checks (`consume_of_borrowed_place_error`) and needs this
/// region-keyed sibling instead. `mutable: true` so *any* live reference
/// (shared or mutable) counts, not only a mutable one.
///
/// A *reference* being consumed is not a place ending: consuming one of two
/// shared projections of a field is how a borrow ends, and the storage it
/// pointed into outlives it either way.
pub(super) fn consumed_place_conflict<'a>(
    consumed: Slot,
    others: &[Slot],
    scope: &'a Scope,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
) -> Option<AliasOrigin<'a>> {
    if matches!(consumed.ty, Type::Ref(..)) {
        return None;
    }
    overlapping_projection(others, scope, prov, live, at, consumed.alias?.set, true)
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
/// Slice 10c (R-P3-2): `tag ( E -- u32 )`, one of the three machine-level
/// primitives, reads a scalar enum's discriminant as a condition flag. Its
/// domain is deliberately the `is_scalar` enums only — every variant
/// payload-free — where the value already *is* its discriminant, so the
/// lowering is a relabel rather than a field read. A payload-carrying enum is
/// a located error here, at check time, rather than at lowering: reading a
/// real tag field out of tagged storage is a larger feature this slice does
/// not need. The predicate is computed from the enum *declaration* (all
/// variants field-free), not from `ir::layout`, which is why the error is
/// reachable before any lowering runs.
pub(super) fn check_tag_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
) -> Result<Option<Vec<Slot>>, String> {
    if name != "tag" {
        return Ok(None);
    }
    if stack.last().is_some_and(|s| s.quot.is_some()) {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    let Some(top) = stack.last().copied() else {
        return Err(underflow_error(ctx, span, "tag", 1, 0));
    };
    let Type::Enum(id, _) = top.ty else {
        return Err(tag_operand_error(ctx, span, top.ty));
    };
    if !ctx.enums()[id.index()]
        .variants
        .iter()
        .all(|v| v.fields.is_empty())
    {
        return Err(tag_payload_enum_error(ctx, span, top.ty));
    }
    stack.pop();
    stack.push(Slot::computed(Type::U32));
    Ok(Some(std::mem::take(stack)))
}

/// `tag` applied to something that is not an enum at all.
fn tag_operand_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `tag` requires an enum operand, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `tag` requires an enum operand, found `{found}`")
        }
    }
}

/// R-P3-2/OQ4: `tag` outside its domain — an enum at least one of whose
/// variants carries a payload, where the discriminant is a field in tagged
/// storage rather than the value itself.
fn tag_payload_enum_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `tag` requires an enum whose variants all carry no payload, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `tag` requires an enum whose variants all carry no payload, found `{found}`"
        ),
    }
}

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
#[allow(clippy::too_many_arguments)]
pub(super) fn check_owned_cell_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    cells: &mut Vec<OwnedCellDecl>,
    prov: &Provenance,
    scope: &Scope,
    live: &Liveness,
    at: usize,
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
            // Review fix (P7 slice 1): `^` consumes its payload just as
            // `drop` does, so a payload a live projection still reaches
            // cannot be moved into the cell out from under that reference.
            if let Some(origin) =
                consumed_place_conflict(stack[n - 1], &stack[..n - 1], scope, prov, live, at)
            {
                return Err(consuming_borrowed_value_error(ctx, span, "^", origin));
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
pub(super) fn array_index_out_of_range_error(
    ctx: &Ctx,
    span: Span,
    count: u32,
    index: i64,
) -> String {
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
            "error: invalid array length in `{}` (line {})\n  `fill` count {} is invalid (an array length must be gte 1 and lte {})\n  note: declared {}",
            name, span.line, count, u32::MAX, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `fill` count {count} is invalid (an array length must be gte 1 and lte {})",
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
    let spelled = crate::resolve::demangle_call(spelled);
    format!(
        "error: `{spelled}` does not borrow a place{} (line {}, col {})\n  {found}\n  a place is a local name or a module `static:`; bind the value with `| name |` first, then borrow that name",
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
/// P7 slice 1 (R1/OQ3): two projections into one region off an owned receiver
/// that has no name to key the place-based `conflicting_borrow_error` on. The
/// remedy is the same one that error gives, phrased for a projection.
fn conflicting_projection_error(
    ctx: &Ctx,
    span: Span,
    name: &str,
    mutable: bool,
    origin: AliasOrigin<'_>,
) -> String {
    let held = match origin {
        AliasOrigin::Name(name) => format!("the projection bound as `{name}`"),
        AliasOrigin::Stack(pushed) => format!(
            "the projection taken at line {}, col {}",
            pushed.line, pushed.col
        ),
    };
    let taken = if mutable { "mutable" } else { "shared" };
    format!(
        "error: `{name}` conflicts with a live projection of the same field{} (line {}, col {})\n  {held} is still live, and this one is {taken}\n  at most one `&!` into a field, and never a `&` alongside a `&!`; consume the earlier projection first",
        in_word(ctx),
        span.line,
        span.col,
    )
}

/// R3: the receiver resolved to a struct, but it has no field of that name,
/// and no local or static shares the name either -- there is no fallback
/// left to try. Named at check-time, where the receiver type is known, so
/// the diagnostic points at that type rather than saying `unknown word`.
fn projection_unknown_field_error(
    ctx: &Ctx,
    span: Span,
    name: &str,
    receiver: &str,
    field: &str,
) -> String {
    format!(
        "error: `{receiver}` has no field `{field}`{} (line {}, col {})\n  `{name}` projects a field of the receiver on top of the stack; `{receiver}` declares no field named `{field}`",
        in_word(ctx),
        span.line,
        span.col,
    )
}

/// R3: the receiver has a field of this name *and* a local (or static) of the
/// same name is in scope. `&`-led resolution tries the receiver first, so a
/// collision here is silent precedence unless it is a located error naming
/// both candidates -- field and local names are short in this corpus and
/// collide easily (`arr`, `acc`, `key`, `n`).
fn projection_field_shadowed_by_local_error(
    ctx: &Ctx,
    span: Span,
    name: &str,
    receiver: &str,
    field: &str,
) -> String {
    let name = crate::resolve::demangle_call(name);
    format!(
        "error: `{name}` is ambiguous{} (line {}, col {})\n  `{receiver}` has a field `{field}`, and a local (or static) named `{field}` is also in scope\n  rename one of them",
        in_word(ctx),
        span.line,
        span.col,
    )
}

/// P7 slice 1 review fix: `drop`, a moving word call, or a moving field
/// getter discards a place -- named or anonymous -- while a reference the
/// owned-receiver projection arm took from it is still live. That arm
/// produces a reference with a region (`Slot.alias`) but no `Deriv` (there is
/// no place name to root one on), so it is invisible to the named-place
/// consume checks (`consume_of_borrowed_place_error`) and needs this region-
/// keyed sibling instead.
pub(super) fn consuming_borrowed_value_error(
    ctx: &Ctx,
    span: Span,
    name: &str,
    origin: AliasOrigin<'_>,
) -> String {
    let name = crate::resolve::demangle_call(name);
    let held = match origin {
        AliasOrigin::Name(n) => format!("the projection bound as `{n}`"),
        AliasOrigin::Stack(pushed) => format!(
            "the projection taken at line {}, col {}",
            pushed.line, pushed.col
        ),
    };
    format!(
        "error: `{name}` consumes a value while a reference derived from it is still live{} (line {}, col {})\n  {held} is still live\n  a place stays borrowed until every reference derived from it is consumed",
        in_word(ctx),
        span.line,
        span.col,
    )
}

fn aliased_place_borrow_error(
    ctx: &Ctx,
    span: Span,
    place: &str,
    origin: &AliasOrigin<'_>,
) -> String {
    let place = crate::resolve::demangle_word(place);
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
    /// `check_src` skips `resolve_modules` entirely, so it never mangles a
    /// name and cannot catch a diagnostic that forgot to demangle one. Every
    /// real build mangles (`assemble_module`'s `always_mangle`, `driver.rs`)
    /// even for a single file, so this helper runs that same pass first.
    fn check_src_mangled(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        crate::resolve::resolve_modules(&mut module, true).unwrap();
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
            &CombinatorEnv::default(),
        )
        .map(|(stack, _insts, _overloads, _fields, _variant_fields)| stack)
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
            op(": main ( -- ) 1 [ add ] add ;\n", "`add`"),
            op(": main ( -- ) [ add ] 1 sub . ;\n", "`sub`"),
            op(": main ( -- ) [ add ] . ;\n", "`.`"),
            // Slice 10c: the branch condition, before the flag mismatch.
            // `if` is a `lib/` word now, so the audited site is the primitive
            // it wraps -- the one builtin exempt from the quotation-operand
            // default-deny for its *branch* operands, but not for its
            // condition.
            op(": main ( -- ) [ add ] [ 1 . ] [ 2 . ] branch ;\n", "`branch`"),
            // check_str_word (`len`/`cstr`).
            op(": main ( -- ) [ add ] len ;\n", "`len`"),
            op(": main ( -- ) [ add ] cstr ;\n", "`cstr`"),
            // check_array_word: the `fill` count operand and the stored element.
            op(": main ( -- ) 5 [ add ] fill ;\n", "`fill`"),
            w(
                ": main ( -- ) [ add ] 8 fill drop ;\n",
                "a quotation cannot be stored",
                "escaping quotations are slice 7",
            ),
            // check_array_index, reached through the `&>` reference word.
            op(
                "type: V x i64 ;\n: main ( -- ) 1 2 V | v | &v &x [ add ] &> drop drop ;\n",
                "`&>`",
            ),
            // check_owned_cell_word.
            op(": main ( -- ) [ add ] ^ ;\n", "`^`"),
            // check_reference_word's `&q` prefix-borrow-of-a-local form.
            op(": main ( -- ) [ add ] | q | &q drop ;\n", "`&q`"),
            // check_access_word's store paths: the value and the receiver.
            w(
                "type: Box s cstr ;\n: main ( -- ) \"hi\" cstr Box | b | &!b &!s [ add ] ! b drop ;\n",
                "a quotation cannot be stored",
                "escaping quotations are slice 7",
            ),
            op(": main ( -- ) [ add ] 1 ! ;\n", "`!`"),
            // the env argument loop and check_poly_call's input loop (R9/R9p).
            w(
                ": foo ( i64 -- i64 ) ;\n: main ( -- ) [ add ] foo drop ;\n",
                "passed to `foo`",
                "only `call` accepts one",
            ),
            w(
                ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n: main ( -- ) [ add ] dupit drop drop ;\n",
                "passed to `dupit`",
                "only `call` accepts one",
            ),
            // check_outputs (R10).
            w(
                ": f ( -- i64 ) [ add ] ;\n",
                "declared output",
                "leaves a quotation on the stack",
            ),
            // the REPL residual (R19), checked through `infer_line`.
            Row {
                source: "1 [ add ]",
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
    /// Slice 10c (R-P3-2/OQ4): `tag`'s domain is the `is_scalar` enums only,
    /// and the predicate is computed from the *declaration* (every variant
    /// payload-free), not from `ir::layout` -- which is what makes the
    /// out-of-domain rejection a located **check-time** error rather than a
    /// lowering panic. Happy path, both error shapes.
    #[test]
    fn check_tag_word_accepts_a_scalar_enum_and_locates_both_rejections() {
        check_src(": w ( bool -- u32 ) tag ;\n: main ( -- ) true w drop ;\n")
            .expect("`bool` is payload-free, so its value is its discriminant");
        check_src(
            "type: Dir | N | S | E | W ;\n: w ( Dir -- u32 ) tag ;\n\
             : main ( -- ) N w drop ;\n",
        )
        .expect("`bool` is not a carve-out: any all-unit-variant enum works");

        let payload = check_src(
            "type: E | None | Some v i64 ;\n: w ( E -- u32 ) tag ;\n\
             : main ( -- ) None w drop ;\n",
        )
        .unwrap_err();
        assert!(
            payload.contains("`tag` requires an enum whose variants all carry no payload"),
            "unexpected message: {payload}"
        );
        assert!(payload.contains("`E`"), "names the enum: {payload}");
        assert!(payload.contains("line 2"), "is located: {payload}");

        let not_enum =
            check_src(": w ( i64 -- u32 ) tag ;\n: main ( -- ) 1 w drop ;\n").unwrap_err();
        assert!(
            not_enum.contains("`tag` requires an enum operand"),
            "unexpected message: {not_enum}"
        );
        assert!(not_enum.contains("line 1"), "is located: {not_enum}");
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

    // --- Phase 6 slice 2 (R10): the variant accessor family, per mechanism.
    //
    // No surface syntax mints a `Type::Variant` operand until slice 3's
    // eliminator, so every case here seeds one onto a hand-built stack. Each
    // names the mechanism it guards and asserts a discriminating shape: a
    // scalar field driven through the projection path would return `Ok(None)`
    // and pass vacuously, so scalar cases go through env dispatch instead, and
    // only the R12 fall-through case asserts on `Ok(None)`.

    /// `Circle` carries one scalar (`r`) and one aggregate (`p`) field, `Rect`
    /// an aggregate one, `Dot` none. Parsed and checked from source so each
    /// `display_static` carries its real `Shape.Circle` spelling; `bool`
    /// occupies enum 0, so `Shape` is enum 1.
    const SHAPE_SRC: &str = "type: P a i64 b i64 ;\ntype: Shape | Circle r i64 p P | Rect q P | Dot ;\n: main ( -- ) ;\n";

    fn shape_module() -> Module {
        let tokens = lex(SHAPE_SRC).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        module
    }

    fn shape_variant(module: &Module, vi: usize) -> Type {
        variant_type(&module.enums, EnumId::from_index(1), vi)
    }

    fn struct_ty(module: &Module, name: &str) -> Type {
        let idx = module.structs.iter().position(|d| d.name == name).unwrap();
        Type::Struct(StructId::from_index(idx), module.structs[idx].name_static)
    }

    /// Mechanism 1: the ordinary env-call path, with only the variant-generated
    /// sigs registered, over an entry stack the caller seeds. Returns the
    /// residual stack.
    fn infer_variant_line(module: &Module, entry: &[Type], src: &str) -> Result<Vec<Type>, String> {
        let tokens = lex(src).unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        let mut env: HashMap<String, Vec<Overload>> = HashMap::new();
        for (name, symbol, sig) in variant_generated_sigs(&module.enums) {
            env.entry(name).or_default().push(Overload { sig, symbol });
        }
        infer_line(
            &terms,
            entry,
            &env,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &module.structs,
            &module.enums,
            &HashMap::new(),
            &CombinatorEnv::default(),
        )
        .map(|(stack, _insts, _overloads, _fields, _variant_fields)| stack)
    }

    #[test]
    fn variant_whole_destructure_types_by_sig_dispatch() {
        // Mechanism 1 (R6): the whole destructure projects every field in
        // declared order, first field deepest, and has no check function at
        // all (structs have none either).
        let module = shape_module();
        let stack = infer_variant_line(&module, &[shape_variant(&module, 0)], "Circle>").unwrap();
        assert_eq!(stack, vec![Type::I64, struct_ty(&module, "P")]);
    }

    #[test]
    fn zero_field_variant_destructures_to_nothing_and_mints_no_getter() {
        // R7: `Dot>` is a no-op destructure and no `Dot>anything` exists.
        let module = shape_module();
        let stack = infer_variant_line(&module, &[shape_variant(&module, 2)], "Dot>").unwrap();
        assert_eq!(stack, vec![]);
        let minted: Vec<String> = variant_generated_sigs(&module.enums)
            .into_iter()
            .map(|(key, _, _)| key)
            .filter(|key| key.starts_with("Dot>"))
            .collect();
        assert_eq!(minted, vec!["Dot>".to_string()]);
    }

    // --- P7 slice 2, R1/R3: borrowing a module static. ---

    /// R1: a *scalar* static is borrowable and types as `&!T` for its declared
    /// `T`. The callee's declared `&!i64` parameter is what pins the type: a
    /// borrow that produced anything else (or a shared `&i64`) fails to match
    /// it.
    #[test]
    fn borrow_of_scalar_static_is_ref_typed() {
        check_src(
            "static: LIMIT i64 = 10 ;\n\
             : peek ( &!i64 -- i64 ) @ ;\n\
             : main ( -- ) &!LIMIT peek . ;",
        )
        .unwrap();
    }

    /// The twin, and the discriminating half: a scalar *local* still has no
    /// address to hand out, so the static branch must not have made every
    /// scalar borrowable.
    #[test]
    fn borrow_of_scalar_local_still_error() {
        let err = check_src(": main ( -- ) 1 | x | &!x @ . ;").unwrap_err();
        assert!(
            err.contains("cannot borrow the scalar local `x` of type `i64`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("a scalar has no address"),
            "unexpected message: {err}"
        );
    }

    /// R3: the exclusivity scan is `owned_root`-keyed and a static-rooted
    /// borrow carries a real `owned_root`, so two live `&!` to one static
    /// conflict exactly as two live `&!` to one local aggregate do. Giving a
    /// static root `owned_root: None` would silently disable this scan for
    /// permanent shared state.
    #[test]
    fn two_live_mutable_static_borrows_conflict() {
        let err = check_src(
            "static: COUNT i64 = 0 ;\n\
             : main ( -- ) &!COUNT &!COUNT @ . @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&!COUNT` conflicts with a live borrow of `COUNT`"),
            "unexpected message: {err}"
        );
    }

    /// The shared/mutable half of the same scan.
    #[test]
    fn shared_static_borrow_beside_a_live_mutable_one_conflicts() {
        let err = check_src(
            "static: COUNT i64 = 0 ;\n\
             : main ( -- ) &!COUNT &COUNT @ . @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&COUNT` conflicts with a live borrow of `COUNT`"),
            "unexpected message: {err}"
        );
    }

    /// R1's resolution order: a bound local first, then a static. The local
    /// here is a scalar, which is *not* borrowable, so the scalar-local
    /// rejection proves the local won -- resolving to the static would have
    /// type-checked clean.
    #[test]
    fn local_shadowing_a_static_resolves_to_the_local() {
        let err = check_src(
            "static: COUNT i64 = 0 ;\n\
             : main ( -- ) 1 | COUNT | &COUNT @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("cannot borrow the scalar local `COUNT`"),
            "unexpected message: {err}"
        );
    }

    /// Mechanism 3 (R11): `check_reference_word` raw, since no source line can
    /// produce a `&Variant` operand this slice. Returns the pushed slot plus
    /// the operand's own derivation, so the caller can assert the projection.
    fn call_variant_ref_word(
        module: &Module,
        word: &str,
        operand_mutable: bool,
    ) -> Result<(Slot, Provenance, DerivId, Vec<RefDecl>), String> {
        let ctx = Ctx::Line {
            structs: &module.structs,
            enums: &module.enums,
        };
        let mut refs = Vec::new();
        let mut prov = Provenance::default();
        let operand = prov.borrow("s", operand_mutable, false, Span::default());
        let ref_ty = intern_ref_type(&mut refs, shape_variant(module, 0), operand_mutable);
        let mut stack = vec![Slot::derived(ref_ty, Some(operand))];
        let out = check_reference_word(
            word,
            Span::default(),
            &mut stack,
            &ctx,
            &Scope::default(),
            &[],
            &[],
            &mut refs,
            &mut prov,
            &Liveness::scan(&[], &HashSet::new(), false),
            0,
            &mut HashMap::new(),
            &mut HashMap::new(),
        )?
        .expect("the variant reference arm claims this word");
        assert_eq!(out.len(), 1);
        Ok((out[0], prov, operand, refs))
    }

    /// R3: the no-stored-reference rule is type-keyed (`contains_reference`),
    /// so it applies to a static-rooted reference unchanged -- the static
    /// branch must not route around it.
    #[test]
    fn storing_a_static_ref_in_a_cell_is_error() {
        let err = check_src(
            "static: COUNT i64 = 0 ;\n\
             : main ( -- ) &!COUNT ^ ;",
        )
        .unwrap_err();
        assert!(
            err.contains("a reference cannot be stored"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("has type `&!i64`"),
            "unexpected message: {err}"
        );
    }

    /// A sigilled name that is neither a local nor a static of this module
    /// still reaches the unchanged non-place rejection.
    #[test]
    fn borrow_of_neither_local_nor_static_is_error() {
        let err = check_src(
            "static: COUNT i64 = 0 ;\n\
             : main ( -- ) &NOPE @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&NOPE` does not borrow a place"),
            "unexpected message: {err}"
        );
    }

    /// A real build always mangles (`always_mangle`, `driver.rs`), so a
    /// static reaches this diagnostic as `COUNT__m0`, never as the source
    /// spelling. `conflicting_borrow_error` must demangle the place before
    /// rendering it, or the user sees a name they never wrote. Run through
    /// `check_src_mangled`, not `check_src`: the latter never mangles
    /// anything and cannot see this class of bug (P7 slice 2 review).
    #[test]
    fn conflicting_borrow_error_names_the_source_spelling_not_the_mangled_one() {
        let err = check_src_mangled(
            "static: COUNT i64 = 0 ;\n\
             : main ( -- ) &!COUNT &!COUNT @ . @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&!COUNT` conflicts with a live borrow of `COUNT`"),
            "unexpected message: {err}"
        );
        assert!(!err.contains("__m"), "leaked a mangled name: {err}");
    }

    /// R3: the back-edge carve-out belongs to a borrow that actually rooted in
    /// a static, not to every borrow whose root *name* answers the static
    /// table. A local shadowing a static (legal, R1) wins the borrow, so its
    /// reference must still be refused at the back-edge: the local's slot is
    /// rebound each iteration whatever it is called.
    #[test]
    fn local_shadowing_a_static_keeps_the_back_edge_check() {
        let err = check_src(
            "static: COUNT i64 = 0 ;\n\
             type: V x i64 ;\n\
             : spin ( &!V i64 -- )\n  | r n |\n  n 0 eq ~[\n  ] ~[\n    \
             0 V | COUNT |\n    &!COUNT n 1 sub spin\n  ] if ;\n\
             : main ( -- )\n  0 V | v |\n  &!v 3 spin\n  v drop ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("a reference to a local cannot cross a loop")
                && err.contains("`COUNT`, a local of this frame"),
            "unexpected message: {err}"
        );
    }

    /// The non-place rejection renders a name too. It is reached with a
    /// *mangled* one whenever the borrow named a type: `&!Point>z` resolves
    /// `Point` and mangles it, then falls through here because `z` is not a
    /// field.
    #[test]
    fn borrow_of_unknown_field_names_the_source_spelling_not_the_mangled_one() {
        let err = check_src_mangled(
            "type: Point x i64 y i64 ;\n\
             : main ( -- ) 1 2 Point | p | &!Point>z drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&!Point>z` does not borrow a place")
                && err.contains("`Point>z` is not a local in scope"),
            "unexpected message: {err}"
        );
        assert!(!err.contains("__m"), "leaked a mangled name: {err}");
    }

    /// Review fix (P7 slice 1, D1): the sibling of the test above with the
    /// receiver *on the stack* rather than bound to a local first --
    /// `check_field_projection` (D2's receiver-directed arm) sees this same
    /// `Point>z` token before the prefix-borrow chain does, and used to treat
    /// the whole mangled `Point>z` string as a plain field name instead of
    /// falling through, leaking it into `projection_unknown_field_error`.
    #[test]
    fn borrow_of_unknown_field_names_the_source_spelling_with_receiver_on_stack() {
        let err = check_src_mangled(
            "type: Point x i64 y i64 ;\n\
             : main ( -- ) 1 2 Point &!Point>z drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&!Point>z` does not borrow a place")
                && err.contains("`Point>z` is not a local in scope"),
            "unexpected message: {err}"
        );
        assert!(!err.contains("__m"), "leaked a mangled name: {err}");
    }

    // --- P7 slice 1, D2/R1: receiver-directed field projections. ---

    /// R1: the field resolves against the type on the stack, not against a
    /// type name in the word. Both modes, and the chain D2 exists for.
    #[test]
    fn projection_resolves_field_against_receiver_type() {
        assert!(check_src(
            "type: Stats hp i64 ;\n\
             type: Unit stats Stats ;\n\
             : main ( -- ) 1 Stats Unit | u | &u &stats &hp @ . &!u &!stats &!hp 2 ! u drop ;",
        )
        .is_ok());
    }

    /// A receiver with no field of that name at all is not a projection, so
    /// the borrow chain still gets it and a non-place still says so.
    #[test]
    fn projection_on_non_struct_receiver_is_error() {
        let err = check_src("type: Point x i64 ;\n: main ( -- ) 7 &x @ . ;").unwrap_err();
        assert!(
            err.contains("`&x` does not borrow a place"),
            "unexpected message: {err}"
        );
    }

    /// The sigil carries the mode; it is never inherited from the receiver.
    #[test]
    fn projection_mode_mismatch_is_error() {
        let err = check_src("type: Point x i64 ;\n: main ( -- ) 1 Point | p | &p &!x 2 ! p drop ;")
            .unwrap_err();
        assert!(
            err.contains("`&!x` expected `&!Point`, found `&Point`"),
            "unexpected message: {err}"
        );
    }

    /// R3: a receiver lacking the field leaves `&n` meaning what it always
    /// meant. Guards the fall-through, which a projection arm that claimed
    /// every `&`-led name would break.
    #[test]
    fn projection_falls_back_to_local_borrow_when_receiver_lacks_field() {
        assert!(check_src(
            "type: Point x i64 ;\n\
             type: Box w i64 ;\n\
             : main ( -- ) 1 Point | p | 2 Box &p &x @ . drop ;",
        )
        .is_ok());
    }

    /// R3: the receiver has no field of that name, and there is no local or
    /// static to fall back to either -- this is check-time's own error,
    /// naming the receiver type rather than saying `unknown word`.
    #[test]
    fn projection_unknown_field_names_the_receiver_type() {
        let err = check_src(
            "type: Buf len usize ;\n\
             : main ( -- ) 0 >usize Buf &lenn @ . drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`Buf` has no field `lenn`"),
            "unexpected message: {err}"
        );
    }

    /// R3: the receiver has field `hp` *and* a local `hp` is in scope --
    /// a located error naming both, not silent precedence either way.
    #[test]
    fn projection_field_shadowed_by_local_is_error() {
        let err = check_src(
            "type: Stats hp i64 ;\n\
             : main ( -- ) 9 | hp | 1 Stats &hp @ . drop hp drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&hp` is ambiguous")
                && err.contains("`Stats` has a field `hp`")
                && err.contains("local"),
            "unexpected message: {err}"
        );
    }

    /// Review fix (P7 slice 1, D2): the same shadow, but with a *static*
    /// rather than a local. `resolve` mangles `&n` to `&n__m0` before the
    /// checker runs (R2's static mangle is unconditional), so the field
    /// lookup here has to demangle before comparing against the struct's own
    /// field names -- otherwise the static wins silently, with no ambiguity
    /// error at all, and the field becomes unreachable through `&n`.
    #[test]
    fn projection_field_shadowed_by_static_is_error() {
        let err = check_src_mangled(
            "static: n i64 = 0 ;\n\
             type: Cnt n i64 ;\n\
             : main ( -- ) 1 Cnt &n @ . drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&n` is ambiguous")
                && err.contains("`Cnt` has a field `n`")
                && err.contains("local (or static)"),
            "unexpected message: {err}"
        );
        assert!(!err.contains("__m"), "leaked a mangled name: {err}");
    }

    /// The point of R1: one spelling, two receivers, two different fields.
    /// A resolution keyed on the name alone could not tell these apart.
    #[test]
    fn projection_same_field_name_on_two_structs_resolves_by_receiver() {
        assert!(check_src(
            "type: A n i64 ;\n\
             type: B tag i64 n bool ;\n\
             : main ( -- ) 1 A &n @ . drop 2 true B &n @ . drop ;",
        )
        .is_ok());
        // ... and the field's *type* is the receiver's, not the other's:
        // `B`'s `n` is a `bool`, so printing it as an `i64` must not resolve.
        let err = check_src(
            "type: A n i64 ;\n\
             type: B tag i64 n bool ;\n\
             : sum ( i64 i64 -- i64 ) add ;\n\
             : main ( -- ) 2 true B &n @ 1 sum . drop ;",
        )
        .unwrap_err();
        assert!(err.contains("`sum`"), "unexpected message: {err}");
    }

    /// OQ3's actual hazard: two live mutable projections of one field off an
    /// *anonymous* owned receiver, which has no place name for the ordinary
    /// borrow-conflict scan to key on. Rejected through the region overlap
    /// the projection interns off the receiver.
    #[test]
    fn two_mutable_projections_off_one_anonymous_receiver_is_error() {
        let err = check_src(
            "type: Point x i64 y i64 ;\n\
             : main ( -- ) 1 2 Point &!x swap &!x 3 ! swap 4 ! drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&!x` conflicts with a live projection of the same field")
                && err.contains("the projection taken at line 2, col 25 is still live"),
            "unexpected message: {err}"
        );
        // A shared projection alongside a mutable one is the same conflict,
        // and a projection bound to a local is found by name.
        let bound = check_src(
            "type: Point x i64 y i64 ;\n\
             : main ( -- ) 1 2 Point &x | a | &!x 3 ! a @ . drop ;",
        )
        .unwrap_err();
        assert!(
            bound.contains("the projection bound as `a` is still live"),
            "unexpected message: {bound}"
        );
        // Two *disjoint* fields off one receiver stay legal: the conflict is
        // region overlap, not "any second projection".
        assert!(check_src(
            "type: Point x i64 y i64 ;\n\
             : main ( -- ) 1 2 Point &!x swap &!y 3 ! swap 4 ! drop ;",
        )
        .is_ok());
    }

    /// R4: `&r` resolves against an owned `Type::Variant` receiver exactly
    /// like a struct receiver -- the receiver-directed lookup does not care
    /// which declaration table the field comes from. Phase 6 slice 3 (R6):
    /// this is the canary asserting the resolution lands in
    /// `resolved_variant_fields` (`EnumId`-keyed) and never in `resolved_fields`
    /// (`StructId`-keyed) -- a future misroute of a variant into the struct
    /// table would flip this assertion.
    #[test]
    fn projection_on_variant_receiver_ok() {
        let module = shape_module();
        let ctx = Ctx::Line {
            structs: &module.structs,
            enums: &module.enums,
        };
        let mut refs = Vec::new();
        let mut prov = Provenance::default();
        let mut stack = vec![Slot::computed(shape_variant(&module, 0))];
        let mut resolved_fields = HashMap::new();
        let mut resolved_variant_fields = HashMap::new();
        let span = Span::default();
        let out = check_reference_word(
            "&r",
            span,
            &mut stack,
            &ctx,
            &Scope::default(),
            &[],
            &[],
            &mut refs,
            &mut prov,
            &Liveness::scan(&[], &HashSet::new(), false),
            0,
            &mut resolved_fields,
            &mut resolved_variant_fields,
        )
        .unwrap()
        .expect("the projection arm claims a bare field on a variant receiver");
        // D2: owned receiver, non-consuming -- the receiver stays and the
        // projected reference joins it.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].ty, shape_variant(&module, 0));
        assert_eq!(out[1].ty, intern_ref_type(&mut refs, Type::I64, false));
        // R6: a variant projection must land in `resolved_variant_fields`
        // (`EnumId`-keyed), never `resolved_fields` (`StructId`-keyed) -- that
        // would hand lowering an index into the wrong declaration table.
        assert_eq!(
            resolved_variant_fields.get(&span),
            Some(&(EnumId::from_index(1), 0, 0)),
            "a variant projection must record (EnumId, variant, field) in resolved_variant_fields"
        );
        assert!(
            resolved_fields.is_empty(),
            "a variant projection must not be routed through `resolved_fields`"
        );

        // A reference receiver is consuming, the same as the struct case.
        let (pushed, _prov, _operand, mut refs) =
            call_variant_ref_word(&module, "&r", false).unwrap();
        assert_eq!(pushed.ty, intern_ref_type(&mut refs, Type::I64, false));
    }

    /// Review fix: the owned-receiver arm's output has a region
    /// (`Slot.alias`) but no `Deriv`, so it is invisible to the named-place
    /// consume checks. Without a region-keyed guard at the point the
    /// receiver is actually discarded, `drop`ping it while the projection is
    /// still live leaves that reference aimed at storage that no longer
    /// exists -- a use-after-free.
    #[test]
    fn drop_of_receiver_while_its_projection_is_live_is_error() {
        let err = check_src(
            "type: Point x i64 y i64 ;\n\
             : main ( -- ) 1 2 Point &x swap drop @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`drop` consumes a value while a reference derived from it is still live")
                && err.contains("the projection taken at line 2, col 25 is still live"),
            "unexpected message: {err}"
        );
        // Binding the receiver to a name first does not launder the hazard:
        // the alias set rides the binding (`Scope::bind`) exactly as it
        // rides an anonymous stack slot.
        let bound = check_src(
            "type: Point x i64 y i64 ;\n\
             : main ( -- ) 1 2 Point &x swap | p | p drop @ . ;",
        )
        .unwrap_err();
        assert!(
            bound.contains(
                "`drop` consumes a value while a reference derived from it is still live"
            ),
            "unexpected message: {bound}"
        );
        // Consuming the *projection* first, then the receiver, is the sound
        // order and stays legal.
        assert!(check_src(
            "type: Point x i64 y i64 ;\n\
             : main ( -- ) 1 2 Point &x @ . drop ;",
        )
        .is_ok());
    }

    /// The same hazard, reached through an ordinary word call rather than
    /// `drop`: any word consuming the receiver by value is an equally live
    /// route to the dangling reference, so the guard belongs on the generic
    /// dispatch path too, not only on the `drop` builtin.
    #[test]
    fn word_call_consuming_receiver_while_its_projection_is_live_is_error() {
        let err = check_src(
            "type: Point x i64 y i64 ;\n\
             : eat ( Point -- ) drop ;\n\
             : main ( -- ) 1 2 Point &x swap eat @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`eat` consumes a value while a reference derived from it is still live"),
            "unexpected message: {err}"
        );
    }

    /// Review fix: the stranded reference is as often another *operand of the
    /// same call* as a value left below it -- `mk &data &^ eat` passes the
    /// receiver and its projection together, and a scan that stops at the
    /// call's own base sees neither. Both operand orders, since the scan has
    /// to reach past the consumed operand in both directions.
    #[test]
    fn word_call_consuming_a_receiver_its_own_argument_projects_is_error() {
        let err = check_src(
            "type: Point x i64 y i64 ;\n\
             : eat ( Point &i64 -- ) swap drop @ . ;\n\
             : main ( -- ) 1 2 Point &x eat ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`eat` consumes a value while a reference derived from it is still live"),
            "unexpected message: {err}"
        );
        let swapped = check_src(
            "type: Point x i64 y i64 ;\n\
             : eat ( &i64 Point -- ) drop @ . ;\n\
             : main ( -- ) 1 2 Point &x swap eat ;",
        )
        .unwrap_err();
        assert!(
            swapped
                .contains("`eat` consumes a value while a reference derived from it is still live"),
            "unexpected message: {swapped}"
        );
    }

    /// Review fix: a polymorphic word is intercepted before the concrete `env`
    /// lookup and so misses the guard on that path entirely -- `'T` binds to
    /// the receiver's struct type as readily as a declared `Point` does, which
    /// makes any generic consumer (`drop`-alike, a container `push`) a route to
    /// the same dangling reference.
    #[test]
    fn poly_call_consuming_receiver_while_its_projection_is_live_is_error() {
        let err = check_src(
            "type: Point x i64 y i64 ;\n\
             : eat ( 'T -- ) drop ;\n\
             : main ( -- ) 1 2 Point &x swap eat @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`eat` consumes a value while a reference derived from it is still live"),
            "unexpected message: {err}"
        );
        // And with the receiver and its projection as operands of that same
        // generic call, the shape the concrete path also has to reach past.
        let same_call = check_src(
            "type: Point x i64 y i64 ;\n\
             : eat ( 'T &i64 -- ) drop drop ;\n\
             : main ( -- ) 1 2 Point &x eat ;",
        )
        .unwrap_err();
        assert!(
            same_call
                .contains("`eat` consumes a value while a reference derived from it is still live"),
            "unexpected message: {same_call}"
        );
    }

    /// Review fix: consuming a *reference* is how a borrow ends, not a place
    /// ending. Two shared projections of one field coexist by design (a new
    /// `&` conflicts only with a live `&!`), so discarding one while the other
    /// is live has to stay legal -- the consume guard keys on the value that
    /// owns the storage, never on a reference into it.
    #[test]
    fn dropping_one_of_two_shared_projections_is_allowed() {
        assert!(check_src(
            "type: Point x i64 y i64 ;\n\
             : main ( -- ) 1 2 Point &x swap &x drop swap @ . drop ;",
        )
        .is_ok());
    }

    /// Review fix: `&>` narrows a reference to one element, and the region it
    /// forwards is the only thing tying that element back to the receiver the
    /// storage belongs to. Without the forwarding the consume guard reads the
    /// receiver as unborrowed and this compiles into a read through a freed
    /// heap block.
    #[test]
    fn array_index_ref_forwards_its_receivers_region() {
        let err = check_src(
            "type: Buf data ^[u8 4] len usize ;\n\
             : mk ( -- Buf ) 0 >u8 4 fill ^ 0 >usize Buf ;\n\
             : main ( -- ) mk &data &^ 0 >usize &> swap drop @ >i64 . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`drop` consumes a value while a reference derived from it is still live"),
            "unexpected message: {err}"
        );
    }

    /// The same forwarding through a projection off an already-referenced
    /// struct (`&b &data`), the step a chain out of a nested receiver runs
    /// through.
    #[test]
    fn nested_field_ref_forwards_its_receivers_region() {
        let err = check_src(
            "type: Buf data ^[u8 4] len usize ;\n\
             type: Wrap b Buf ;\n\
             : mk ( -- Buf ) 0 >u8 4 fill ^ 0 >usize Buf ;\n\
             : main ( -- ) mk Wrap &b &data &^ swap drop 0 >usize &> @ >i64 . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`drop` consumes a value while a reference derived from it is still live"),
            "unexpected message: {err}"
        );
    }

    /// The projection's *reference*-receiver arm needs the same forwarding as
    /// its fused sibling above. A single hop is caught by the receiver's own
    /// region, so only a two-hop chain (`&!s &!v`) discriminates: without the
    /// forwarding the second hop drops the region and the store lands in
    /// storage `drop` already consumed.
    #[test]
    fn chained_projection_forwards_its_receivers_region() {
        let err = check_src(
            "type: Inner v i64 ;\n\
             type: Outer s Inner ;\n\
             : main ( -- ) 1 Inner Outer &!s &!v swap drop 7 ! ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`drop` consumes a value while a reference derived from it is still live"),
            "unexpected message: {err}"
        );
    }

    /// The forwarding is an over-approximation, so it needs a canary against
    /// over-rejection: a chained borrow stays legal across the consumption of
    /// an unrelated receiver of the same type, which a region coarse enough to
    /// conflate the two would refuse.
    #[test]
    fn chained_projection_does_not_borrow_an_unrelated_receiver() {
        assert!(check_src(
            "type: Inner v i64 ;\n\
             type: Outer s Inner ;\n\
             : main ( -- ) 2 Inner Outer &s &v @ . \
             &!s &!v 1 Inner Outer drop 9 ! &s &v @ . drop ;",
        )
        .is_ok());
    }

    /// A real build always mangles (`always_mangle`, `driver.rs`), so the
    /// consumer reaches this diagnostic as `eat__m0` / `Outer__m0>n` unless it
    /// demangles, exactly as every sibling error in this file does.
    #[test]
    fn consuming_borrowed_value_error_names_the_source_spelling_not_the_mangled_one() {
        let err = check_src_mangled(
            "type: Wrap n i64 ;\n\
             : eat ( Wrap -- ) drop ;\n\
             : main ( -- ) 1 Wrap &n swap eat @ . ;",
        )
        .unwrap_err();
        assert!(err.contains("`eat` consumes"), "unexpected message: {err}");
        assert!(!err.contains("__m"), "leaked a mangled name: {err}");
        // The generated destructure reaches it as a mangled *accessor*
        // spelling, the case a bare `demangle_word` would miss.
        let getter = check_src_mangled(
            "type: Inner v i64 ;\n\
             type: Outer tag i64 n Inner ;\n\
             : main ( -- ) 0 0 Inner Outer &tag swap Outer> drop drop drop @ . ;",
        )
        .unwrap_err();
        assert!(
            getter.contains("`Outer>` consumes"),
            "unexpected message: {getter}"
        );
        assert!(!getter.contains("__m"), "leaked a mangled name: {getter}");
    }

    /// And through `^`, which heap-allocates its top-of-stack operand and so
    /// pops the receiver exactly as `drop` does.
    #[test]
    fn owned_cell_alloc_consuming_receiver_while_its_projection_is_live_is_error() {
        let err = check_src(
            "type: Wrap n i64 ;\n\
             : main ( -- ) 1 Wrap &n swap ^ drop @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`^` consumes a value while a reference derived from it is still live"),
            "unexpected message: {err}"
        );
    }

    /// And through the whole-struct destructure (`Outer>`), which pops its
    /// struct receiver exactly as `drop` does.
    #[test]
    fn struct_get_consuming_receiver_while_its_projection_is_live_is_error() {
        let err = check_src(
            "type: Inner v i64 ;\n\
             type: Outer tag i64 n Inner ;\n\
             : main ( -- ) 0 0 Inner Outer &tag swap Outer> drop drop drop @ . ;",
        )
        .unwrap_err();
        assert!(
            err.contains(
                "`Outer>` consumes a value while a reference derived from it is still live"
            ),
            "unexpected message: {err}"
        );
    }
}
