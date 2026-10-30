//! Atomic lvalue reads, stores and compare-exchange retry loops.
use super::{
    A,
    FunctionLowerer,
    IntCC,
    Item,
    LoweringError,
    M,
    Operand,
    Place,
    SourceVectors,
    Type,
    TypeId,
    Value,
    types,
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Atomic compound updates retry using the observed value from cmpxchg.
    /// C11: §6.5.16.2p3, pp. 103-104; PDF pp. 121-122.
    pub(in crate::lowering) fn atomic_update(
        &mut self,
        place: Item,
        initial: Item,
        source: SourceVectors,
        update: impl FnOnce(&mut Self, Item) -> Result<Item, LoweringError>,
    ) -> Result<(Item, Item), LoweringError> {
        let Operand::Place(Place::Memory(address)) = place.operand else {
            return Err(LoweringError::missing(
                "atomic update has no storage",
                source,
            ));
        };
        let layout = types::layout(&self.unit.sema.types, place.ty)
            .map_err(|kind| crate::lowering::module_items::at(kind, source))?;
        // No register holds a wide bit-precise integer or a GNU vector of
        // more than 16 bytes (or of a size no integer has). Nor does one
        // whose lanes leave padding, like the last 4 bytes of a 3 x `int`
        // vector's 16: no lane writes them, so a register's value would
        // read bytes no one stored.
        let padded = self
            .vector(place.ty)
            .is_some_and(|shape| shape.count * shape.stride < shape.size);
        if self.wide_bit_int(place.ty).is_some()
            || padded
            || !matches!(layout.size, 1 | 2 | 4 | 8 | 16)
        {
            return self.atomic_update_bytes(address, place.ty, source, update);
        }
        let ty = self.unit.sema.types.non_atomic(place.ty).unqualified();
        let bits_ty = types::int_type(u32::try_from(layout.size * 8).expect("scalar atomic width"));
        let initial = self.atomic_item_bits(initial, bits_ty, source)?;
        let retry = self.draft.create_block(&[bits_ty]);
        let done = self.draft.create_block(&[]);
        self.draft.jump(retry, &[initial]);
        self.draft.switch_to(retry);
        let expected = self.draft.block_params(retry)[0];
        let old = self.atomic_bits_item(expected, ty, source)?;
        // The update runs again on each retry, so the wide results it reads
        // keep their slots.
        self.wide_holds += 1;
        let desired = update(self, old);
        self.wide_holds -= 1;
        let desired = desired?;
        let bits = self.atomic_item_bits(desired, bits_ty, source)?;
        let align = self.place_alignment(address, types::align(layout));
        let observed = self
            .atomic_inst(
                A::CompareExchange,
                bits_ty,
                &[address, bits, expected],
                [M::SeqCst, M::SeqCst],
                align,
                types::is_volatile(place.ty),
            )
            .expect("compare result");
        let success = self.icmp(IntCC::Eq, observed, expected);
        let edges = [
            self.draft.edge(done, &[]),
            self.draft.edge(retry, &[observed]),
        ];
        self.draft
            .terminate(crate::lowering::ssa::Terminator::Brif(success, edges));
        self.draft.seal(retry);
        self.draft.seal(done);
        self.draft.switch_to(done);
        Ok((old, desired))
    }

    /// [`Self::atomic_update`] of an object no register holds, a wide
    /// bit-precise integer or a GNU vector of more than 16 bytes: its bytes
    /// are loaded into an object, the update computes from that object, and
    /// a compare-exchange of the bytes stores the result or refreshes the
    /// object with the value it found for another try.
    /// C11: §6.5.16.2p3, pp. 103-104; PDF pp. 121-122.
    fn atomic_update_bytes(
        &mut self,
        address: Value,
        object: TypeId,
        source: SourceVectors,
        update: impl FnOnce(&mut Self, Item) -> Result<Item, LoweringError>,
    ) -> Result<(Item, Item), LoweringError> {
        let ty = self.unit.sema.types.non_atomic(object).unqualified();
        let layout = types::layout(&self.unit.sema.types, object)
            .map_err(|kind| crate::lowering::module_items::at(kind, source))?;
        let align = self.place_alignment(address, types::align(layout));
        let volatile = types::is_volatile(object);
        let expected = self.fresh_object(layout.size, types::align(layout), source)?;
        let size = self.iconst(Type::I64, i128::from(layout.size));
        _ = self.atomic_inst(
            A::LoadBytes,
            Type::I8,
            &[address, expected, size],
            [M::SeqCst, M::Relaxed],
            align,
            volatile,
        );
        let retry = self.draft.create_block(&[]);
        let done = self.draft.create_block(&[]);
        self.draft.jump(retry, &[]);
        self.draft.switch_to(retry);
        let old = Item {
            operand: Operand::Aggregate(expected),
            ty,
        };
        // The update runs again on each retry, so the wide results it reads
        // keep their slots.
        self.wide_holds += 1;
        let desired = update(self, old);
        self.wide_holds -= 1;
        let desired = desired?;
        let Operand::Aggregate(bytes) = desired.operand else {
            return Err(LoweringError::unsupported(
                crate::lowering::Construct::Atomic,
                source,
            ));
        };
        let exchanged = self
            .atomic_inst(
                A::CompareExchangeBytes,
                Type::I8,
                &[address, expected, bytes, size],
                [M::SeqCst, M::SeqCst],
                align,
                volatile,
            )
            .expect("compare result");
        let zero = self.iconst(Type::I8, 0);
        let success = self.icmp(IntCC::Ne, exchanged, zero);
        let edges = [self.draft.edge(done, &[]), self.draft.edge(retry, &[])];
        self.draft
            .terminate(crate::lowering::ssa::Terminator::Brif(success, edges));
        self.draft.seal(retry);
        self.draft.seal(done);
        self.draft.switch_to(done);
        Ok((old, desired))
    }

    pub(in crate::lowering) fn atomic_read(
        &mut self,
        address: Value,
        object: TypeId,
        to: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let layout = types::layout(&self.unit.sema.types, object)
            .map_err(|kind| crate::lowering::module_items::at(kind, source))?;
        if matches!(self.repr(object, source)?, types::Repr::Aggregate)
            || !matches!(layout.size, 1 | 2 | 4 | 8 | 16)
        {
            let destination = self.fresh_object(layout.size, types::align(layout), source)?;
            let size = self.iconst(Type::I64, i128::from(layout.size));
            _ = self.atomic_inst(
                A::LoadBytes,
                Type::I8,
                &[address, destination, size],
                [M::SeqCst, M::Relaxed],
                types::align(layout),
                types::is_volatile(object),
            );
            return Ok(Item {
                operand: Operand::Aggregate(destination),
                ty:      to,
            });
        }
        let ty = types::int_type(u32::try_from(layout.size * 8).expect("small atomic"));
        let align = self.place_alignment(address, types::align(layout));
        let bits = self
            .atomic_inst(
                A::Load,
                ty,
                &[address],
                [M::SeqCst, M::Relaxed],
                align,
                types::is_volatile(object),
            )
            .expect("load result");
        self.atomic_bits_item(bits, to, source)
    }

    pub(in crate::lowering) fn atomic_write(
        &mut self,
        address: Value,
        object: TypeId,
        value: Item,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let layout = types::layout(&self.unit.sema.types, object)
            .map_err(|kind| crate::lowering::module_items::at(kind, source))?;
        if matches!(self.repr(object, source)?, types::Repr::Aggregate)
            || !matches!(layout.size, 1 | 2 | 4 | 8 | 16)
        {
            let Operand::Aggregate(desired) = value.operand else {
                return Err(LoweringError::unsupported(
                    crate::lowering::Construct::Atomic,
                    source,
                ));
            };
            let size = self.iconst(Type::I64, i128::from(layout.size));
            _ = self.atomic_inst(
                A::StoreBytes,
                Type::I8,
                &[address, desired, size],
                [M::SeqCst, M::Relaxed],
                types::align(layout),
                types::is_volatile(object),
            );
            return Ok(());
        }
        let ty = types::int_type(u32::try_from(layout.size * 8).expect("small atomic"));
        let bits = self.atomic_item_bits(value, ty, source)?;
        let align = self.place_alignment(address, types::align(layout));
        _ = self.atomic_inst(
            A::Store,
            ty,
            &[address, bits],
            [M::SeqCst, M::Relaxed],
            align,
            types::is_volatile(object),
        );
        Ok(())
    }
}
