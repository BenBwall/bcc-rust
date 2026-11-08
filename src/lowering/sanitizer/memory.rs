//! The checks of memory accesses: through a null pointer (`null`), through a
//! misaligned one (`alignment`), and of an index outside an array of known
//! size (`bounds`).
//!
//! An access is checked where it goes through a pointer: an address within
//! an object lowering placed (a local, a global, a variable length array)
//! is known not to be null, and as aligned as its offsets keep it
//! ([`FunctionSanitizer::known`](super::FunctionSanitizer)). Clang checks the
//! load of a converted lvalue and the store of an assignment, and the
//! pointer of each member access, at the member's name; an increment's
//! operand is not checked. GCC checks every load and store, an increment's
//! and a compound assignment's included, and one through a member of a
//! record a pointer points to as an access of that member, with the record's
//! type.

use super::{
    super::{
        FunctionLowerer,
        alignment::at_offset,
        work::{
            Item,
            Operand,
            Place,
        },
    },
    Field,
    Handler,
    Parent,
    Style,
    sites::operator_site,
};
use crate::{
    ir::{
        Align,
        InstData,
        InstFlags,
        IntCC,
        Opcode,
        Type,
        Value,
    },
    target::sanitizers::UbChecks,
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            Expression,
            ExpressionType,
        },
        semantic_analysis::{
            ArrayBound,
            ConversionKind,
            TagKind,
            TypeId,
            TypeKind,
        },
    },
};

/// The library's kinds of access (compiler-rt `TypeCheckKind`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::lowering) enum Access {
    Load = 0,
    Store = 1,
    Member = 3,
}

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Records what is known of the address `value` an instruction defines:
    /// the start of a stack slot, a global or a dynamic allocation, or an
    /// offset from a known address.
    pub(in crate::lowering) fn note_storage(&mut self, inst: InstData, value: Value) {
        let align = match inst {
            | InstData::StackAddr { slot } => self.draft.slot_align(slot),
            | InstData::GlobalAddr { global } => self.unit.module.global(global).align,
            | InstData::DynAlloca { align, .. } => align,
            | InstData::Binary {
                opcode: Opcode::PtrAdd,
                args: [base, offset],
                ..
            } => {
                let Some(&base) = self.ub.known.get(&base) else {
                    return;
                };
                match self.draft.constant(offset) {
                    | Some(offset) => at_offset(base, offset.unsigned_abs()),
                    | None => Align::BYTE,
                }
            },
            | _ => return,
        };
        _ = self.ub.known.insert(value, align);
    }

    /// Records that the known address `address`, an offset from `base` by a
    /// multiple of `stride`, is as aligned as that keeps it.
    pub(in crate::lowering) fn note_stride(&mut self, base: Value, address: Value, stride: u64) {
        if let Some(&align) = self.ub.known.get(&base) {
            _ = self
                .ub
                .known
                .insert(address, at_offset(align, u128::from(stride)));
        }
    }

    /// Checks the load of the lvalue `expression`'s converted value from
    /// `place`, of type `ty`. `update` reads the operand of an increment or a
    /// compound assignment, which Clang checks as the store it precedes, and
    /// only for the compound assignment.
    pub(in crate::lowering) fn check_read(
        &mut self,
        expression: &'tu Expression<'tu>,
        place: Place,
        ty: TypeId,
        update: bool,
    ) {
        if !self.checks(UbChecks::NULL | UbChecks::ALIGNMENT | UbChecks::OBJECT_SIZE) {
            return;
        }
        if self.gcc_reports() {
            let site = if update {
                operator_site(expression)
            } else {
                self.report_site(expression)
            };
            self.check_place(place, ty, Access::Load, site);
            return;
        }
        if matches!(expression.kind, ExpressionType::Identifier(_)) {
            return;
        }
        if !update {
            let site = self.report_site(expression);
            self.check_place(place, ty, Access::Load, site);
        } else if self.parent(expression) == Some(Parent::Assigned) {
            let site = self.report_site(expression);
            self.check_place(place, ty, Access::Store, site);
        }
    }

    /// Checks the store of an assignment, an increment or a compound
    /// assignment, `expression`, to `place` of type `ty`: Clang checks only
    /// the simple assignment's, at its left operand, which is `target`.
    pub(in crate::lowering) fn check_write(
        &mut self,
        expression: &'tu Expression<'tu>,
        target: Option<&'tu Expression<'tu>>,
        place: Item,
    ) {
        if !self.checks(UbChecks::NULL | UbChecks::ALIGNMENT | UbChecks::OBJECT_SIZE) {
            return;
        }
        let Operand::Place(place_operand) = place.operand else {
            return;
        };
        if self.gcc_reports() {
            self.check_place(
                place_operand,
                place.ty,
                Access::Store,
                operator_site(expression),
            );
        } else if let Some(target) = target
            && !matches!(target.kind, ExpressionType::Identifier(_))
        {
            let site = self.report_site(target);
            self.check_place(place_operand, place.ty, Access::Store, site);
        }
    }

    /// Checks a member access `expression` whose record of type `record`
    /// is at `base`: Clang checks the pointer as it forms the member's place,
    /// except through a named record; GCC checks each access of the member
    /// later, so its place remembers the pointer.
    pub(in crate::lowering) fn check_member(
        &mut self,
        expression: &'tu Expression<'tu>,
        base: Value,
        member: Value,
        record: TypeId,
        align: Align,
    ) {
        if !self.checks(UbChecks::NULL | UbChecks::ALIGNMENT | UbChecks::OBJECT_SIZE) {
            return;
        }
        // An access of the member needs the member's alignment, which a
        // packed record lowers, and no more than its record's place has.
        let align = match self.ub.required.get(&base) {
            | Some(&place) if place.bytes() < align.bytes() => place,
            | _ => align,
        };
        _ = self.ub.required.insert(member, align);
        let (ExpressionType::DirectMember {
            base_expression, ..
        }
        | ExpressionType::IndirectMember {
            base_expression, ..
        }) = expression.kind
        else {
            return;
        };
        if self.gcc_reports() {
            let pointer = self
                .ub
                .members
                .get(&base)
                .copied()
                .unwrap_or((base, record));
            if !self.ub.known.contains_key(&pointer.0) {
                _ = self.ub.members.insert(member, pointer);
            }
            return;
        }
        let site = match expression.kind {
            | ExpressionType::IndirectMember { .. } => self.report_site(expression),
            | _ if matches!(base_expression.kind, ExpressionType::Identifier(_)) => return,
            | _ => self.report_site(base_expression),
        };
        // A record that is itself a member of a packed record has only the
        // alignment its place there gives it, as Clang's lvalue keeps.
        let mut align = super::super::alignment::storage_alignment(&self.unit.sema.types, record)
            .unwrap_or(Align::BYTE);
        if let Some(&place) = self.ub.required.get(&base)
            && place.bytes() < align.bytes()
        {
            align = place;
        }
        self.check_pointer(base, record, Access::Member, align, site, None);
    }

    /// GCC: a place computed from a member's place, an element of a member
    /// array, is reached through the same pointer.
    pub(in crate::lowering) fn inherit_member(&mut self, from: Value, to: Value) {
        if let Some(&pointer) = self.ub.members.get(&from) {
            _ = self.ub.members.insert(to, pointer);
        }
        // An element of an array at an underaligned place, such as a member
        // of a packed record, is no better aligned than the array.
        if let Some(&align) = self.ub.required.get(&from) {
            _ = self.ub.required.insert(to, align);
        }
    }

    /// Checks an access of kind `access` to the object of type `ty` at
    /// `place`, reported at `site`. The access needs the alignment of its
    /// type, or of its member, whatever lowering knows of the address.
    fn check_place(&mut self, place: Place, ty: TypeId, access: Access, site: SourceVectors) {
        let address = match place {
            | Place::Memory(address) => address,
            | Place::BitField(address, _) if self.gcc_reports() => address,
            | _ => return,
        };
        if let Some(&(pointer, record)) = self.ub.members.get(&address) {
            let align = super::super::alignment::storage_alignment(&self.unit.sema.types, record)
                .unwrap_or(Align::BYTE);
            self.check_pointer(pointer, record, Access::Member, align, site, None);
            if let Place::Memory(_) = place {
                self.check_object_size(address, ty, access, site);
            }
            return;
        }
        if matches!(place, Place::BitField(..))
            || matches!(
                super::super::types::repr(&self.unit.sema.types, ty),
                Err(_) | Ok(super::super::types::Repr::Void | super::super::types::Repr::Function)
            )
            || matches!(self.unit.sema.types.kind(ty), TypeKind::Array(..))
        {
            return;
        }
        let align = self.ub.required.get(&address).copied().unwrap_or_else(|| {
            super::super::alignment::storage_alignment(&self.unit.sema.types, ty)
                .unwrap_or(Align::BYTE)
        });
        if self.gcc_reports() {
            self.check_pointer(address, ty, access, align, site, None);
            self.check_object_size(address, ty, access, site);
        } else {
            // Clang checks the room in the same comparison, and not for an
            // element whose index it checked.
            let room = self.object_size_failure(address, ty);
            self.check_pointer(address, ty, access, align, site, room);
        }
    }

    /// Checks that the object `address` points into has room for the `ty`
    /// accessed there, which GCC reports on its own.
    fn check_object_size(
        &mut self,
        address: Value,
        ty: TypeId,
        access: Access,
        site: SourceVectors,
    ) {
        let Some(failed) = self.object_size_failure(address, ty) else {
            return;
        };
        self.check_undefined(UbChecks::OBJECT_SIZE, failed, site, |this| {
            let fields = [
                Field::Type(this.type_descriptor(ty, Style::Pointer)),
                Field::Byte(0),
                Field::Byte(access as u8),
            ];
            let operands = [this.value_handle(address)];
            Handler::TypeMismatch.report(this.scratch, &fields, &operands)
        });
    }

    /// Whether the object `address` points into has too little room for the
    /// `ty` accessed there: `None` where nothing is checked. GCC asks of an
    /// object lowering placed, the only kind its instrumentation knows
    /// without optimizing, and of every address when optimizing; Clang asks
    /// only when it optimizes, since LLVM answers the
    /// question only then, and not of an element whose index it checked. The
    /// library reports a pointer that passed the other checks as one with
    /// too little room. GCC manual, "Instrumentation Options".
    fn object_size_failure(&mut self, address: Value, ty: TypeId) -> Option<Value> {
        let gcc = self.gcc_reports();
        let optimizing = self.unit.context.configuration.optimization().0;
        if !self.checks(UbChecks::OBJECT_SIZE)
            || (gcc && !optimizing && !self.ub.known.contains_key(&address))
            || (!gcc && (!optimizing || self.ub.bounded.contains_key(&address)))
        {
            return None;
        }
        let size = self
            .unit
            .sema
            .types
            .layout(ty)
            .map(|layout| layout.size)
            .filter(|&size| size != 0)?;
        let query = crate::ir::ObjectSizeQuery::new(0, true)?;
        let room = self.inst(
            InstData::ObjectSize {
                ty: Type::I64,
                query,
                ptr: address,
            },
            Type::I64,
        );
        let needed = self.iconst(Type::I64, i128::from(size));
        Some(self.icmp(IntCC::Ult, room, needed))
    }

    /// Checks that `pointer`, through which an object of type `ty` that
    /// needs alignment `align` is accessed, is not null and is so aligned,
    /// and, with `room`, that it is not too little.
    /// C99: §6.5.3.2 paragraph 4, p. 79; PDF p. 91; §6.3.2.3 paragraph 7,
    /// pp. 47-48; PDF pp. 59-60.
    fn check_pointer(
        &mut self,
        pointer: Value,
        ty: TypeId,
        access: Access,
        align: Align,
        site: SourceVectors,
        room: Option<Value>,
    ) {
        let known = self.ub.known.get(&pointer).copied();
        let mut failures = [None, None, None];
        if self.checks(UbChecks::NULL) && known.is_none() {
            let null = self.nullary(Opcode::Null);
            failures[0] = Some((UbChecks::NULL, self.icmp(IntCC::Eq, pointer, null)));
        }
        if self.checks(UbChecks::ALIGNMENT)
            && align.bytes() > 1
            && known.is_none_or(|known| known.bytes() < align.bytes())
        {
            let bits = self.convert(Opcode::Ptrtoint, Type::I64, pointer);
            let mask = self.iconst(Type::I64, i128::from(align.bytes() - 1));
            let low = self.binary_inst(Opcode::And, InstFlags::empty(), bits, mask);
            let zero = self.iconst(Type::I64, 0);
            failures[1] = Some((UbChecks::ALIGNMENT, self.icmp(IntCC::Ne, low, zero)));
        }
        failures[2] = room.map(|failed| (UbChecks::OBJECT_SIZE, failed));
        let report = |this: &mut Self| {
            let fields = [
                Field::Type(this.type_descriptor(ty, Style::Pointer)),
                Field::Byte(u8::try_from(align.bytes().trailing_zeros()).unwrap_or(u8::MAX)),
                Field::Byte(access as u8),
            ];
            let operands = [this.value_handle(pointer)];
            Handler::TypeMismatch.report(this.scratch, &fields, &operands)
        };
        self.check_together(failures, site, report);
    }

    /// Checks the index `index`, of C type `index_ty`, of the subscript
    /// `expression` whose array operand is `array`, when that array has a
    /// known number of elements and is not a record's last member, which
    /// both compilers take for a flexible array. An element may be addressed
    /// one past the end with `&`, and, in Clang, wherever it is not loaded
    /// or assigned.
    /// C99: §6.5.6 paragraph 8, p. 83; PDF p. 95; §6.5.2.1 paragraph 2, p. 70;
    /// PDF p. 82.
    pub(in crate::lowering) fn check_bounds(
        &mut self,
        expression: &'tu Expression<'tu>,
        array: &'tu Expression<'tu>,
        index: Value,
        index_ty: TypeId,
        source: SourceVectors,
    ) -> bool {
        if !self.checks(UbChecks::ARRAY_BOUNDS) {
            return false;
        }
        let Ok(info) = self.info(array) else {
            return false;
        };
        let array_ty = info.ty;
        let TypeKind::Array(_, ArrayBound::Constant(count)) = self.unit.sema.types.kind(array_ty)
        else {
            return false;
        };
        if self.trailing_member(array) {
            return false;
        }
        let parent = self.parent(expression);
        let accessed = if self.gcc_reports() {
            parent != Some(Parent::AddressOf)
        } else {
            // Clang counts an element accessed where it is loaded, stored, a
            // member's record or a row of a further subscript, but not where
            // an increment's operand or a decayed array.
            let decayed = self
                .conversions(expression)
                .first()
                .is_some_and(|conversion| conversion.kind == ConversionKind::ArrayDecay);
            !matches!(parent, Some(Parent::AddressOf | Parent::Stepped))
                && (!decayed || parent == Some(Parent::Subscripted))
        };
        let signed = self
            .repr(index_ty, source)
            .is_ok_and(super::super::types::Repr::signed);
        // Clang compares a 128-bit index whole; GCC takes it as `sizetype`.
        let compared = if !self.gcc_reports() && self.draft.value_type(index).bits() > 64 {
            self.draft.value_type(index)
        } else {
            Type::I64
        };
        let wide = self.extend(index, signed, compared);
        let count = self.iconst(compared, i128::from(count));
        let condition = if accessed { IntCC::Uge } else { IntCC::Ugt };
        let failed = self.icmp(condition, wide, count);
        let site = self.front_end_site(source);
        self.check_undefined(UbChecks::ARRAY_BOUNDS, failed, site, |this| {
            let fields = [
                Field::Type(this.type_descriptor(array_ty, Style::Array)),
                Field::Type(this.type_descriptor(index_ty.unqualified(), Style::Plain)),
            ];
            let operands = [this.value_handle(index)];
            Handler::OutOfBounds.report(this.scratch, &fields, &operands)
        });
        true
    }

    /// Records that `address` is an element whose index was checked.
    pub(in crate::lowering) fn note_bounded(&mut self, address: Value) {
        _ = self.ub.bounded.insert(address, ());
    }

    /// Checks the pointer arithmetic that moves `base` by `offset` bytes: the
    /// address may not wrap around, and in Clang, which reads
    /// C as making even `p + 0` undefined for a null `p`, neither pointer may
    /// be null. `product` is the index, the element size and their wrapped
    /// product, when the offset is one: that may not overflow either.
    /// C99: §6.5.6 paragraph 8, p. 83; PDF p. 95.
    pub(in crate::lowering) fn check_pointer_overflow(
        &mut self,
        base: Value,
        offset: Value,
        product: Option<[Value; 3]>,
        source: SourceVectors,
    ) {
        let check = UbChecks::POINTER_OVERFLOW;
        if !self.checks(check) || self.draft.constant(offset) == Some(0) {
            return;
        }
        let plain = InstFlags::empty();
        // The sum of the integers, as Clang computes it: the `inbounds`
        // address itself would be poison where the check fails.
        let base_bits = self.convert(Opcode::Ptrtoint, Type::I64, base);
        let result_bits = self.binary_inst(Opcode::Iadd, plain, base_bits, offset);
        let zero = self.iconst(Type::I64, 0);
        let forward = self.icmp(IntCC::Sge, offset, zero);
        let not_below = self.icmp(IntCC::Uge, result_bits, base_bits);
        let below = self.icmp(IntCC::Ult, result_bits, base_bits);
        let mut valid = self.choose(forward, not_below, below);
        if let Some([index, size, scaled]) = product
            && let Some(overflow) = self.overflow_condition(Opcode::Imul, [index, size], scaled)
        {
            let no = self.iconst(Type::I1, 0);
            let exact = self.icmp(IntCC::Eq, overflow, no);
            valid = self.binary_inst(Opcode::And, plain, valid, exact);
        }
        if !self.gcc_reports() {
            let base_set = if self.ub.known.contains_key(&base) {
                None
            } else {
                Some(self.icmp(IntCC::Ne, base_bits, zero))
            };
            let result_set = self.icmp(IntCC::Ne, result_bits, zero);
            valid = self.binary_inst(Opcode::And, plain, valid, result_set);
            if let Some(base_set) = base_set {
                valid = self.binary_inst(Opcode::And, plain, valid, base_set);
            }
        }
        let no = self.iconst(Type::I1, 0);
        let failed = self.icmp(IntCC::Eq, valid, no);
        let site = self.current_site(source);
        self.check_undefined(check, failed, site, |this| {
            let operands = [base_bits, result_bits];
            Handler::PointerOverflow.report(this.scratch, &[], &operands)
        });
    }

    /// Checks the arguments `arguments`, converted, of the direct call `call`
    /// of the function `binding`, whose argument expressions are
    /// `expressions`: those its `nonnull` names may not be null. Clang reports
    /// at the argument and points to the attribute, GCC at the call.
    /// GNU extension: GCC manual, "Common Function Attributes".
    pub(in crate::lowering) fn check_nonnull_arguments(
        &mut self,
        call: &'tu Expression<'tu>,
        binding: usize,
        arguments: &[Item],
        expressions: &[&'tu Expression<'tu>],
    ) {
        let check = UbChecks::NONNULL_ATTRIBUTE;
        if !self.checks(check) {
            return;
        }
        let Some(attributes) = self.unit.sema.entity_attributes(binding) else {
            return;
        };
        let (Some(positions), written) = (attributes.nonnull, attributes.nonnull_site) else {
            return;
        };
        let gcc = self.gcc_reports();
        for (index, argument) in arguments.iter().enumerate() {
            let Operand::Value(value) = argument.operand else {
                continue;
            };
            if !(positions.is_empty() || positions.contains(&index))
                || self.draft.value_type(value) != Type::Ptr
                || self.ub.known.contains_key(&value)
            {
                continue;
            }
            let null = self.nullary(Opcode::Null);
            let failed = self.icmp(IntCC::Eq, value, null);
            let site = match expressions.get(index) {
                | Some(&expression) if !gcc => self.operand_site(expression),
                | _ => self.call_site(call),
            };
            let position = u32::try_from(index + 1).unwrap_or(u32::MAX);
            self.check_undefined(check, failed, site, |this| {
                let fields = [
                    Field::Location(written.filter(|_| !gcc)),
                    Field::Int(position),
                ];
                Handler::NonnullArg.report(this.scratch, &fields, &[])
            });
        }
    }

    /// Checks the pointer `value` that `statement` returns from a function
    /// declared `returns_nonnull`: it may not be null. Clang reports at the
    /// `return` and points to the attribute, GCC at the returned expression.
    /// GNU extension: GCC manual, "Common Function Attributes".
    pub(in crate::lowering) fn check_nonnull_return(
        &mut self,
        statement: &'tu crate::translation_phases::parsing::syntax::Statement<'tu>,
        value: Value,
    ) {
        use crate::translation_phases::parsing::syntax::{
            ExpressionSlot,
            StatementType,
        };
        let check = UbChecks::RETURNS_NONNULL_ATTRIBUTE;
        if !self.checks(check)
            || self.draft.value_type(value) != Type::Ptr
            || self.ub.known.contains_key(&value)
        {
            return;
        }
        let Some(written) = self
            .record
            .binding
            .and_then(|binding| self.unit.sema.entity_attributes(binding))
            .and_then(|attributes| attributes.returns_nonnull)
        else {
            return;
        };
        let gcc = self.gcc_reports();
        let site = match statement.kind {
            | StatementType::Return(Some(ExpressionSlot::Parsed(expression))) if gcc =>
                self.report_site(expression),
            | _ => statement.source_vectors,
        };
        let null = self.nullary(Opcode::Null);
        let failed = self.icmp(IntCC::Eq, value, null);
        self.check_undefined(check, failed, site, |this| {
            let location = this.unit.check_data(Some(site), &[]);
            let address = this.inst(InstData::GlobalAddr { global: location }, Type::Ptr);
            let operand = this.value_handle(address);
            let mut report = Handler::NonnullReturn.report(this.scratch, &[], &[operand]);
            report.site = super::DataSite::Given((!gcc).then_some(written));
            report
        });
    }

    /// Whether `array`, through parentheses, names the last member of a
    /// structure, or a member of a union.
    fn trailing_member(&self, mut array: &'tu Expression<'tu>) -> bool {
        while let ExpressionType::Parenthesized { expression } = array.kind {
            array = expression;
        }
        if !matches!(
            array.kind,
            ExpressionType::DirectMember { .. } | ExpressionType::IndirectMember { .. }
        ) {
            return false;
        }
        let (Some(facts), Some(field)) = (
            self.unit.sema.member_facts(array),
            self.unit.sema.selected_field(array),
        ) else {
            return true;
        };
        let tag = self.unit.sema.types.tags[facts.tag];
        tag.kind != TagKind::Struct || field.path.index + 1 >= tag.members.get().len()
    }
}
