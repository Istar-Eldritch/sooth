//! `FuncBuilder` word_families method group: reference/array/owned-cell/
//! struct-field lowering primitives (`push_reference`..`load_field_onto_stack`).

use super::*;

impl<'a> FuncBuilder<'a> {
    /// Push a reference `Value` (always `IrType::Ptr`) and record what it
    /// points at, since the `IrType` deliberately no longer says.
    pub(super) fn push_reference(&mut self, ptr: Value, referent: IrType) {
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
    pub(super) fn lower_reference_word(&mut self, name: &str, span: Span) {
        let line = span.line;
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
                // P7 slice 1 (R6): a receiver-directed projection `&f`/`&!f`.
                // The name says nothing about which struct it projects out of,
                // so the receiver comes from the checker's per-call-site
                // record; an owned receiver keeps its place on the stack (D2).
                if let Some(&(id, fi)) = self.resolved_fields.get(&span) {
                    let base = *self.stack.last().expect("projection: receiver");
                    // D2: a reference receiver is consumed by the projection,
                    // an owned one stays. `ref_inner` is what tells them
                    // apart -- every reference `Value` records its referent,
                    // an owned aggregate's own value does not -- and either
                    // way the value *is* the address the field offsets from.
                    if self.ref_inner.contains_key(&base) {
                        self.stack.pop();
                    }
                    let field = self.structs.layouts[id.index()].fields[fi];
                    let addr = self.field_ptr(base, field.offset);
                    self.push_reference(addr, field.ty);
                    return;
                }
                // Phase 6 slice 3 (R6): the variant twin, keyed by `EnumId`
                // rather than `StructId` (the field lives inside the
                // variant's own payload region, at `payload_offset +
                // field.offset`, unlike a bare struct's field which starts at
                // offset 0).
                if let Some(&(id, vi, fi)) = self.resolved_variant_fields.get(&span) {
                    let base = *self.stack.last().expect("projection: receiver");
                    if self.ref_inner.contains_key(&base) {
                        self.stack.pop();
                    }
                    let payload_offset = self.enums.layouts[id.index()].payload_offset;
                    let field = self.enums.layouts[id.index()].variants[vi].fields[fi];
                    let addr = self.field_ptr(base, payload_offset + field.offset);
                    self.push_reference(addr, field.ty);
                    return;
                }
                let local = self.locals.iter().find(|(n, _)| n == rest).map(|(_, v)| *v);
                match local {
                    Some(value) => self.lower_borrow(value),
                    // R1: a module static, which unlike a local has a data
                    // symbol to hand out an address for -- so a *scalar*
                    // static is borrowable where a scalar local is not. Looked
                    // up second, so a local shadowing a static wins here the
                    // same way it does in the checker.
                    None => {
                        let ty =
                            *self.statics.referent.get(rest).expect(
                                "checked: a borrow's operand is a local or a module static",
                            );
                        let addr = self.fresh_value(IrType::Ptr);
                        self.push_instr(Instr::StaticAddr(addr, rest.to_string()));
                        self.push_reference(addr, ty);
                    }
                }
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
    pub(super) fn lower_access_word(&mut self, name: &str) {
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
    pub(in crate::ir) fn alloc_struct(&mut self, id: StructId) -> Value {
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
    pub(in crate::ir) fn alloc_enum(&mut self, id: EnumId) -> Value {
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
    pub(in crate::ir) fn alloc_array(&mut self, id: ArrayId) -> Value {
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
    pub(in crate::ir) fn value_size(&self, ty: IrType) -> u32 {
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
    pub(super) fn elem_addr(&mut self, base: Value, index: Value, stride: u32) -> Value {
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
    pub(super) fn lower_array_word(&mut self, name: &str) {
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
    pub(super) fn alloc_aggregate(&mut self, ty: IrType) -> Value {
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
    pub(in crate::ir) fn load_owned_payload(
        &mut self,
        cell_ptr: Value,
        payload_ty: IrType,
    ) -> Value {
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
    pub(in crate::ir) fn drop_level_fields(
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
    pub(in crate::ir) fn emit_path_steps(&mut self, cur: Value, steps: &[PathStep]) {
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
    pub(in crate::ir) fn emit_field_level(
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
    pub(in crate::ir) fn emit_branch(
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
    pub(super) fn lower_owned_cell_word(&mut self, name: &str) {
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
    pub(in crate::ir) fn field_ptr(&mut self, base: Value, offset: u32) -> Value {
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
    pub(super) fn store_field(&mut self, fptr: Value, val: Value, field: FieldLayout) {
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
    /// or the interior pointer as a nested struct/enum/quotation value (a
    /// destructure reads a stored quotation back out as a runtime value).
    pub(super) fn field_value(&mut self, base: Value, field: FieldLayout) -> Value {
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

    pub(super) fn load_field_onto_stack(&mut self, base: Value, field: FieldLayout) {
        let v = self.field_value(base, field);
        self.stack.push(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::test_helpers::*;

    /// Phase 6 slice 3 (R6): `Circle`'s field 0 is a scalar `i64`, `Rect`'s
    /// field 0 is an aggregate `P` -- so reading `variants[1]` (`Rect`) for a
    /// `Circle` (`vi` 0) projection yields a *differently-typed* address, not
    /// merely a missing one. Both field 0s sit at offset 0, so the address
    /// assertion cannot see that mutation on its own: the `ref_inner` type
    /// assertion is the only thing that catches a wrong variant index here,
    /// and deleting it would leave the wrong-variant case unguarded. No
    /// surface syntax reaches this mechanism
    /// yet (Phase 4's eliminator is what calls it), so the state is
    /// hand-built, mirroring how P7 slice 1 and Slice 2 each unit-tested a
    /// checker-only mechanism before its lowering arm existed.
    fn shape_enums() -> Enums {
        enums_of(
            "type: P a i64 b i64 ;\n\
             type: Shape | Circle r i64 p P | Rect q P ;\n\
             : main ( -- ) ;\n",
        )
    }

    #[test]
    fn variant_field_projection_reads_the_correct_variant_and_field() {
        let enums = shape_enums();
        // `bool` is injected as enum 0 ahead of any user enum (`BOOL_ENUM_ID`),
        // so `Shape` is enum 1.
        let id = EnumId::from_index(1);
        let payload_offset = enum_layout(&enums, "Shape").payload_offset;
        let field = enum_layout(&enums, "Shape").variants[0].fields[0];
        let structs = Structs::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let env: HashMap<String, Arity> = HashMap::new();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let mut b = empty_builder(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
                statics: empty_statics(),
            },
        );
        let span = Span::default();
        let mut variant_fields = HashMap::new();
        variant_fields.insert(span, (id, 0usize, 0usize));
        b.resolved_variant_fields = &variant_fields;
        let receiver = b.fresh_value(IrType::Enum(id));
        b.stack.push(receiver);
        b.lower_reference_word("&r", span);
        // D2: owned receiver, non-consuming -- the receiver stays and the
        // projected reference joins it.
        assert_eq!(b.stack.len(), 2);
        assert_eq!(b.stack[0], receiver);
        let addr = b.stack[1];
        assert!(
            b.cur_instrs.iter().any(|i| matches!(i,
                Instr::PtrOffset(dst, base, off)
                    if *dst == addr && *base == receiver && *off as u32 == payload_offset + field.offset
            )),
            "expected a PtrOffset at payload_offset add field.offset: {:?}",
            b.cur_instrs
        );
        assert_eq!(*b.ref_inner.get(&addr).unwrap(), field.ty);
    }

    #[test]
    fn variant_field_projection_reference_receiver_pops_it() {
        // The owned-vs-reference split (D2): a reference receiver is
        // consumed by the projection (`ref_inner` is what tells the two
        // apart), an owned one is not.
        let enums = shape_enums();
        let id = EnumId::from_index(1);
        let payload_offset = enum_layout(&enums, "Shape").payload_offset;
        let field = enum_layout(&enums, "Shape").variants[0].fields[0];
        let structs = Structs::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let env: HashMap<String, Arity> = HashMap::new();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let mut b = empty_builder(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
                statics: empty_statics(),
            },
        );
        let span = Span::default();
        let mut variant_fields = HashMap::new();
        variant_fields.insert(span, (id, 0usize, 0usize));
        b.resolved_variant_fields = &variant_fields;
        let receiver = b.fresh_value(IrType::Ptr);
        b.ref_inner.insert(receiver, IrType::Enum(id));
        b.stack.push(receiver);
        b.lower_reference_word("&r", span);
        // The reference receiver is popped; only the projected reference
        // remains.
        assert_eq!(b.stack.len(), 1);
        let addr = b.stack[0];
        assert!(
            b.cur_instrs.iter().any(|i| matches!(i,
                Instr::PtrOffset(dst, base, off)
                    if *dst == addr && *base == receiver && *off as u32 == payload_offset + field.offset
            )),
            "expected a PtrOffset at payload_offset add field.offset: {:?}",
            b.cur_instrs
        );
        assert_eq!(*b.ref_inner.get(&addr).unwrap(), field.ty);
    }

    #[test]
    fn lower_borrow_of_cell_local_gives_the_pointer_a_place() {
        // `&^`/`&!^` project by *loading* the cell pointer out of the
        // place holding it, but a cell local's value already *is* that pointer
        // (an SSA temporary with no address), so borrowing one has to give it a
        // slot first. The load then reads that slot back.
        let ir = lower_src(": w ( -- i64 ) 7 ^ | c | &c &^ @ c ^> drop ;");
        let w = &ir.funcs[0];
        let alloc = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Alloc(v, size, _) if *size == WORD_WIDTH => Some(*v),
                _ => None,
            })
            .expect("borrowing a cell local allocs a one-word place");
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::Store(dst, _) if *dst == alloc)),
            "the cell pointer is stored into its new place: {:?}",
            instrs(w)
        );
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::Load(_, src) if *src == alloc)),
            "the projection loads the pointer back out: {:?}",
            instrs(w)
        );
    }

    #[test]
    fn lower_reference_through_a_branch_join_keeps_its_referent() {
        // A merged reference is still the opaque `Ptr`, which says nothing
        // about what it points at, so the join has to carry the referent shape
        // across or the projection past it has no field offset to use.
        let ir = lower_src(
            "type: V x i64 y i64 ;\n             : w ( bool -- i64 ) | c | 1 2 V | v | c ~[ &v ] ~[ &v ] if &x @ ;",
        );
        let w = &ir.funcs[0];
        let phi = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Phi(v, _) => Some(*v),
                _ => None,
            })
            .expect("the two arms merge their references in a phi");
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::PtrOffset(_, base, _) if *base == phi)),
            "the projection past the join offsets from the merged value: {:?}",
            instrs(w)
        );
    }

    #[test]
    fn fill_lowering_instruction_count_is_independent_of_n() {
        // Slice 6h (D4): `fill`'s re-lowering is a counted loop, so its emitted
        // instruction count is identical at N 4 and 64 (the retired unrolled
        // lowering grew one FieldStore per element), and above a small floor so
        // an empty lowering cannot satisfy it. Replaces
        // `lower_fill_allocs_and_unrolls_n_stores`, whose name encoded the
        // removed unrolling.
        let n4 = count(&lower_src(": w ( -- ) 7 4 fill drop ;").funcs[0], |_| true);
        let n64 = count(&lower_src(": w ( -- ) 7 64 fill drop ;").funcs[0], |_| true);
        assert_eq!(n4, n64);
        assert!(n4 > 4, "not an empty lowering: {n4}");
    }

    #[test]
    fn fill_lowering_instruction_count_at_10000_equals_4() {
        // The compile-cost defect's durable proxy: the retired unrolled
        // lowering emitted one store per element, so N = 10000 was QBE-
        // quadratic on one straight-line block. The counted loop emits the
        // same instruction count at N = 10000 as at N = 4, so code size is O(1)
        // in the count (the re-measured wall-clock numbers are in the commit).
        let n4 = count(&lower_src(": w ( -- ) 7 4 fill drop ;").funcs[0], |_| true);
        let n10k = count(
            &lower_src(": w ( -- ) 7 10000 fill drop ;").funcs[0],
            |_| true,
        );
        assert_eq!(n4, n10k);
    }

    #[test]
    fn fill_lowering_uses_elem_addr_after_relowering() {
        // A real transition assertion: `fill` used `field_ptr`/`PtrOffset` with
        // a compile-time offset before slice 6h, so exactly one runtime
        // `ElemAddr` (the counted store loop's body) is the observable switch.
        // Its stride is the element stride (8 for `[i64]`), not the byte-
        // granular `1` the constructor's zero-init uses.
        let ir = lower_src(": w ( -- ) 7 4 fill drop ;");
        let w = &ir.funcs[0];
        let strides: Vec<i64> = w
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::ElemAddr(_, _, _, s) => Some(*s),
                _ => None,
            })
            .collect();
        assert_eq!(strides, vec![8], "one ElemAddr, element-strided");
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 1);
    }

    #[test]
    fn fill_lowering_result_reaches_a_reference_consumer() {
        // D4: the re-lowering must not disturb `fill`'s consumed operands nor
        // leave the array off the stack. A `fill` result that is then used
        // (indexed via a reference) lowers and reads back the seed, proving the
        // filled array survives the loop and reaches its consumer.
        //
        // This does NOT cover R19 surviving-capture-set forwarding
        // (`check.rs`'s `let surviving = element.surviving;` in
        // `check_array_word`'s "fill" arm): `Slot`/`surviving` is a check-time
        // concept the IR never sees (`lower_src` returns an `IrModule` with no
        // Slot-level information), so no IR-level assertion can exercise it.
        // The real regression test for that forwarding is
        // `check::tests::fill_forwards_surviving_set_so_a_returned_array_rejects_an_escaping_capture`
        // (an end-to-end located-error test, since deleting the forwarding
        // makes an unsound program wrongly build rather than change any IR
        // shape).
        let ir = lower_src(": w ( -- i64 ) 7 4 fill | a | &a 1 &> @ ;");
        let w = &ir.funcs[0];
        // One alloc for the array; the loop stores the seed; the consumer
        // reads it back through a reference projection.
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 1);
        assert!(count(w, |i| matches!(i, Instr::ElemAddr(..))) >= 2);
    }

    #[test]
    fn array_constructor_emits_exactly_one_alloc_of_correct_size() {
        // D3: the constructor allocs exactly one array slot, sized to the
        // layout (`[i64 10]` is 80 bytes / align 8), not one Alloc per element.
        let src = ": w ( -- ) [ i64 ; 10 ] drop ;";
        let ir = lower_src(src);
        let w = &ir.funcs[0];
        let (size, align) = {
            let a = arrays_of(src);
            (a.layouts[0].size, a.layouts[0].align)
        };
        assert_eq!((size, align), (80, 8));
        let allocs: Vec<(u32, u32)> = w
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::Alloc(_, s, al) => Some((*s, *al)),
                _ => None,
            })
            .collect();
        assert_eq!(allocs, vec![(size, align)]);
    }

    #[test]
    fn array_constructor_zero_init_uses_stride_one_and_bounds_by_layout_size() {
        // The zero-init loop is byte-granular: exactly one `ElemAddr` with
        // `stride == 1` (a stride of 8 would skip 7 of every 8 bytes), and its
        // loop bound is a `Const` equal to `ArrayLayout::size` (a bound of
        // `count` would zero only the first `count` bytes). An
        // instruction-*kind* assertion would catch neither mutation.
        let src = ": w ( -- ) [ i64 ; 10 ] drop ;";
        let ir = lower_src(src);
        let w = &ir.funcs[0];
        let size = arrays_of(src).layouts[0].size; // 80
        let strides: Vec<i64> = w
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::ElemAddr(_, _, _, s) => Some(*s),
                _ => None,
            })
            .collect();
        assert_eq!(strides, vec![1], "one byte-granular ElemAddr, stride 1");
        assert_eq!(
            count(w, |i| matches!(i, Instr::Const(_, v) if *v == size as i64)),
            1,
            "the loop bound is one Const equal to ArrayLayout::size"
        );
    }

    #[test]
    fn array_constructor_instruction_count_is_independent_of_count() {
        // A runtime zero-init loop is O(1) in Count: the emitted instruction
        // count is identical at 4 and 64 (an unrolled lowering would grow), and
        // above a small floor so an empty lowering cannot satisfy it.
        let n4 = count(&lower_src(": w ( -- ) [ i64 ; 4 ] drop ;").funcs[0], |_| {
            true
        });
        let n64 = count(
            &lower_src(": w ( -- ) [ i64 ; 64 ] drop ;").funcs[0],
            |_| true,
        );
        assert_eq!(n4, n64);
        assert!(n4 > 4, "not an empty lowering: {n4}");
    }

    #[test]
    fn lower_reference_element_read_is_elem_addr_and_load() {
        // `&>` addresses the element (`ElemAddr`); `@` loads it
        // (`FieldLoad`); neither allocs, since the array is never rebuilt.
        let ir = lower_src(": w ( [i64 4] -- i64 ) | a | &a 0 &> @ ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 0);
    }

    #[test]
    fn lower_reference_element_store_is_elem_addr_and_store_no_rebuild() {
        // `&!>` addresses the element; `!` stores directly, with no alloc and
        // no blit: replacing `set`'s whole-array rebuild is the point.
        let ir = lower_src(": w ( [i64 4] usize i64 -- ) | a i x | &!a i &!> x ! ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldStore(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::Blit(..))), 0);
    }

    #[test]
    fn lower_reference_element_runtime_index_emits_bounds_guard_and_trap_call() {
        // A runtime (non-literal) index guards the access with `index < N`
        // and jumps to a trap block that calls the OOB helper.
        let ir = lower_src(": w ( [i64 4] usize -- i64 ) | a i | &a i &> @ ;");
        let w = &ir.funcs[0];
        assert!(w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
        assert_eq!(
            count(
                w,
                |i| matches!(i, Instr::Call(None, sym, _) if sym == OOB_TRAP_SYMBOL)
            ),
            1
        );
    }

    #[test]
    fn lower_reference_element_constant_index_has_no_runtime_guard() {
        // A checked literal index is bounds-verified at compile time, so it
        // skips the runtime guard entirely — no branch, no trap call.
        let ir = lower_src(": w ( [i64 4] -- i64 ) | a | &a 0 &> @ ;");
        let w = &ir.funcs[0];
        assert!(!w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
        assert_eq!(
            count(
                w,
                |i| matches!(i, Instr::Call(None, sym, _) if sym == OOB_TRAP_SYMBOL)
            ),
            0
        );
    }

    #[test]
    fn lower_len_is_a_constant_with_no_memory_access() {
        // R18: `len` folds to a constant `usize` (the count) with no load and
        // no element addressing.
        let ir = lower_src(": w ( [i64 4] -- usize ) len swap drop ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).iter().any(|i| matches!(i, Instr::Const(_, 4))));
        assert_eq!(count(w, |i| matches!(i, Instr::ElemAddr(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::Load(..))), 0);
    }

    #[test]
    fn lower_owned_cell_unwrap_scalar_loads_before_freeing() {
        // R13: `^>` must materialise the payload before calling `sooth_free`,
        // so the freed pointer is never handed to the stack.
        let ir = lower_src(": w ( -- i64 ) 5 ^ ^> ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        let load_at = is
            .iter()
            .position(|i| matches!(i, Instr::FieldLoad(..)))
            .expect("a FieldLoad");
        let free_at = is
            .iter()
            .position(|i| matches!(i, Instr::Call(None, sym, _) if sym == FREE_SYMBOL))
            .expect("a free call");
        assert!(
            load_at < free_at,
            "scalar payload must load before the cell frees: load at {load_at}, free at {free_at}"
        );
    }

    #[test]
    fn lower_owned_cell_unwrap_aggregate_blits_before_freeing() {
        // The aggregate counterpart of the scalar case above (R13): the copy-out
        // `Blit` must precede `sooth_free`, never aliasing the freed cell.
        let ir = lower_src("type: Point x i64 y i64 ; : w ( -- Point ) 1 2 Point ^ ^> ;");
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let is = instrs(w);
        let blit_at = is
            .iter()
            .position(|i| matches!(i, Instr::Blit(..)))
            .expect("a Blit");
        let free_at = is
            .iter()
            .position(|i| matches!(i, Instr::Call(None, sym, _) if sym == FREE_SYMBOL))
            .expect("a free call");
        assert!(
            blit_at < free_at,
            "aggregate payload must blit out before the cell frees: blit at {blit_at}, free at {free_at}"
        );
    }

    #[test]
    fn lower_borrow_of_a_static_takes_its_data_symbol_address() {
        // R1: a static has an address to hand out, so its borrow is a
        // `StaticAddr` naming the data symbol -- not the `PtrOffset`-off-a-value
        // shape a local's borrow uses, and not an `Alloc` giving it a frame
        // place. Both the `&!` and the `&` reach the same symbol.
        let ir = lower_src("static: COUNT i64 = 0 ;\n: w ( -- i64 ) &!COUNT 1 +! &COUNT @ ;");
        let w = func(&ir, "w");
        let symbols: Vec<&str> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::StaticAddr(_, sym) => Some(sym.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(symbols, ["COUNT", "COUNT"]);
        assert_eq!(
            count(w, |i| matches!(i, Instr::Alloc(..))),
            0,
            "a static is storage already; borrowing one allocates nothing: {:?}",
            instrs(w)
        );
    }

    #[test]
    fn lower_borrow_prefers_a_local_over_a_same_named_static() {
        // R1's resolution order at lowering, the twin of the checker's: a bound
        // local wins over a static of the same name. The struct local is what
        // makes the program checkable at all (a scalar local is not
        // borrowable), and the witness is that *no* data symbol is addressed --
        // a lowering that looked statics up first would read the wrong place
        // and still lower cleanly.
        let ir = lower_src(
            "static: COUNT i64 = 0 ;\n\
             type: P x i64 ;\n\
             : w ( -- i64 ) 1 P | COUNT | &COUNT &x @ ;",
        );
        let w = func(&ir, "w");
        assert_eq!(
            count(w, |i| matches!(i, Instr::StaticAddr(..))),
            0,
            "the local shadows the static: {:?}",
            instrs(w)
        );
    }

    #[test]
    fn lower_borrow_of_a_static_inside_a_materialized_quotation() {
        // R4/R5's escaping-quotation corner reaching lowering: the static is
        // named inside a quotation literal that materializes into its own
        // `IrFunc`, so the static table has to be visible on that separate
        // lowering pass too, not only on the enclosing word's.
        let ir = lower_src("static: COUNT i64 = 0 ;\n: make ( -- [ -- ] ) [ &!COUNT 1 +! ] ;");
        let quot = ir
            .funcs
            .iter()
            .find(|f| f.name != "make")
            .expect("the literal materializes into its own func");
        assert_eq!(
            count(
                quot,
                |i| matches!(i, Instr::StaticAddr(_, sym) if sym == "COUNT")
            ),
            1,
            "the materialized body addresses the static: {:?}",
            instrs(quot)
        );
    }

    /// P7 slice 1 (D2/R6): an owned receiver is *not* consumed, so two
    /// projections off one struct both offset from that struct's own value.
    /// A consuming lowering would have popped it and offset the second
    /// projection from whatever sat beneath.
    #[test]
    fn owned_receiver_projection_leaves_receiver() {
        let ir = lower_src("type: Point x i64 y i64 ;\n: w ( -- ) 1 2 Point &x @ . &y @ . drop ;");
        let w = &ir.funcs[0];
        let alloc = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Alloc(v, ..) => Some(*v),
                _ => None,
            })
            .expect("the struct is alloc'd");
        let bases: Vec<Value> = read_bases(w);
        assert_eq!(
            bases,
            vec![alloc, alloc],
            "both projections offset from the receiver, which stayed: {:?}",
            instrs(w)
        );
    }

    /// P7 slice 1 (D2/R6): a *reference* receiver is consumed, so a chain
    /// hands each step's result to the next and leaves nothing behind. This is
    /// what keeps `u &stats &hp` from stranding an intermediate reference on
    /// the stack at every level.
    ///
    /// The `+` beneath the chain is what makes the emitted code witness the
    /// pop: the chain's own `PtrOffset`s look identical either way (a
    /// non-consuming lowering would orphan the intermediates rather than
    /// re-base anything), but a stranded reference shifts everything under it,
    /// so `+` would add the fetched field to `&stats` instead of to the `5`.
    #[test]
    fn ref_receiver_projection_consumes_receiver() {
        let ir = lower_src(
            "type: Stats hp i64 mp i64 ;\n\
             type: Unit tag i64 stats Stats ;\n\
             : w ( -- ) 5 1 2 3 Stats Unit | u | &u &stats &hp @ add . u drop ;",
        );
        let w = &ir.funcs[0];
        let five = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Const(v, 5) => Some(*v),
                _ => None,
            })
            .expect("the addend is a constant");
        let fetched = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::FieldLoad(v, _) => Some(*v),
                _ => None,
            })
            .expect("`@` fetches the field");
        let (a, b) = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Bin(_, BinOp::Add, a, b) => Some((*a, *b)),
                _ => None,
            })
            .expect("the fetched field is added");
        assert_eq!(
            (a, b),
            (five, fetched),
            "the chain left nothing between the `5` and the fetched field: {:?}",
            instrs(w)
        );
        // ... and each step of the chain bases on the previous step's result.
        let offsets: HashMap<Value, Value> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::PtrOffset(dst, base, _) => Some((*dst, *base)),
                _ => None,
            })
            .collect();
        let read = read_bases(w);
        assert_eq!(read.len(), 1, "one read: {:?}", instrs(w));
        let stats = read[0];
        let borrow = offsets[&stats];
        let storage = offsets[&borrow];
        assert!(
            instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::Alloc(v, ..) if *v == storage)),
            "the chain bottoms out at the receiver's own storage: {:?}",
            instrs(w)
        );
    }

    /// The base each `@` reads through, in emission order: the `PtrOffset`
    /// whose result a `FieldLoad` consumes. Skips the construction-time
    /// offsets a `Point`/`Unit` literal emits, which are not projections.
    fn read_bases(w: &IrFunc) -> Vec<Value> {
        let offsets: HashMap<Value, Value> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::PtrOffset(dst, base, _) => Some((*dst, *base)),
                _ => None,
            })
            .collect();
        instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::FieldLoad(_, src) => Some(offsets[src]),
                _ => None,
            })
            .collect()
    }
}
