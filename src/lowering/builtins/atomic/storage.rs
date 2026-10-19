//! Atomic object representation and runtime-sized buffer operations.
use super::{
    A,
    Align,
    AtomicBuiltin,
    BuiltinCallFacts,
    Expression,
    FunctionLowerer,
    InstData,
    InstFlags,
    Item,
    LoweringError,
    M,
    O,
    Op,
    Opcode,
    Operand,
    SourceVectors,
    Type,
    TypeId,
    Value,
    types,
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    pub(super) fn finish_atomic_buffer(
        &mut self,
        e: &'tu Expression<'tu>,
        instance: &'tu BuiltinCallFacts<'tu>,
        builtin: AtomicBuiltin,
        items: &[Item],
        object: TypeId,
        order: M,
    ) -> Result<Operand, LoweringError> {
        let source = e.source_vectors;
        let address = self.value(items[0], source)?;
        let layout = types::layout(&self.unit.sema.types, object)
            .map_err(|kind| crate::lowering::module_items::at(kind, source))?;
        let size = self.iconst(Type::I64, i128::from(layout.size));
        let align = self.place_alignment(address, types::align(layout));
        let volatile = types::is_volatile(object);
        if builtin.operation == O::Load {
            let destination = if builtin.generic {
                self.value(items[1], source)?
            } else {
                self.fresh_object(layout.size, align, source)?
            };
            _ = self.atomic_inst(
                A::LoadBytes,
                Type::I8,
                &[address, destination, size],
                [order, M::Relaxed],
                align,
                volatile,
            );
            return Ok(if builtin.generic {
                Operand::Void
            } else {
                Operand::Aggregate(destination)
            });
        }
        let desired_index = if builtin.operation == O::Compare {
            2
        } else {
            1
        };
        let desired = if builtin.generic {
            self.value(items[desired_index], source)?
        } else {
            match items[desired_index].operand {
                | Operand::Aggregate(value) => value,
                | _ =>
                    return Err(LoweringError::unsupported(
                        crate::lowering::Construct::Atomic,
                        source,
                    )),
            }
        };
        let operation = match builtin.operation {
            | O::Store => A::StoreBytes,
            | O::Exchange => A::ExchangeBytes,
            | O::Compare => A::CompareExchangeBytes,
            | O::Init => {
                _ = self.draft.push(
                    self.memory_op(InstData::MemoryRange {
                        opcode: Opcode::Copy,
                        flags: if volatile {
                            InstFlags::VOLATILE
                        } else {
                            InstFlags::empty()
                        },
                        align,
                        args: [address, desired, size],
                    }),
                    None,
                );
                return Ok(Operand::Void);
            },
            | _ =>
                return Err(LoweringError::unsupported(
                    crate::lowering::Construct::Atomic,
                    source,
                )),
        };
        let (output, args, failure) = if operation == A::StoreBytes {
            (None, [address, desired, size, size], M::Relaxed)
        } else if operation == A::ExchangeBytes {
            let output = if builtin.generic {
                self.value(items[2], source)?
            } else {
                self.fresh_object(layout.size, align, source)?
            };
            (Some(output), [address, desired, output, size], M::Relaxed)
        } else {
            let expected = self.value(items[1], source)?;
            let failure = instance.operands[builtin.count - 1]
                .integer
                .map_or(M::SeqCst, |n| M::from_c(n.value));
            (None, [address, expected, desired, size], failure)
        };
        let result = self.atomic_inst(
            operation,
            Type::I8,
            &args[..operation.arity()],
            [order, failure],
            align,
            volatile,
        );
        Ok(if operation == A::CompareExchangeBytes {
            Operand::Value(result.expect("compare result"))
        } else if !builtin.generic && operation == A::ExchangeBytes {
            Operand::Aggregate(output.expect("exchange output"))
        } else {
            Operand::Void
        })
    }

    pub(in crate::lowering) fn atomic_item_bits(
        &mut self,
        item: Item,
        bits_ty: Type,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        if let Operand::Aggregate(address) = item.operand {
            return self.load_atomic_buffer(address, item.ty, bits_ty, source);
        }
        let value = self.value(item, source)?;
        let ty = self.draft.value_type(value);
        Ok(if ty == Type::Ptr {
            self.convert(Opcode::Ptrtoint, bits_ty, value)
        } else if ty == Type::F80 {
            let address =
                self.fresh_object(16, Align::from_bytes(16).expect("alignment"), source)?;
            let zero = self.iconst(bits_ty, 0);
            self.plain_store_bits(zero, address, Align::BYTE, false);
            self.plain_store_bits(value, address, Align::BYTE, false);
            self.inst(
                InstData::Load {
                    ty:    bits_ty,
                    flags: InstFlags::empty(),
                    align: Align::BYTE,
                    tag:   None,
                    addr:  address,
                },
                bits_ty,
            )
        } else if ty.is_float() {
            self.convert(Opcode::Bitcast, bits_ty, value)
        } else {
            self.extend(value, false, bits_ty)
        })
    }

    pub(in crate::lowering) fn atomic_bits_item(
        &mut self,
        bits: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let operand = match self.repr(ty, source)? {
            | types::Repr::Aggregate => {
                let layout = types::layout(&self.unit.sema.types, ty)
                    .map_err(|kind| crate::lowering::module_items::at(kind, source))?;
                let address = self.fresh_object(layout.size, types::align(layout), source)?;
                self.store_atomic_buffer(bits, address, ty, source)?;
                Operand::Aggregate(address)
            },
            | types::Repr::Pointer =>
                Operand::Value(self.convert(Opcode::Inttoptr, Type::Ptr, bits)),
            | types::Repr::Float { ty: Type::F80 } => {
                let address =
                    self.fresh_object(16, Align::from_bytes(16).expect("alignment"), source)?;
                self.plain_store_bits(bits, address, Align::BYTE, false);
                Operand::Value(self.inst(
                    InstData::Load {
                        ty:    Type::F80,
                        flags: InstFlags::empty(),
                        align: Align::BYTE,
                        tag:   None,
                        addr:  address,
                    },
                    Type::F80,
                ))
            },
            | types::Repr::Float { ty } => Operand::Value(self.convert(Opcode::Bitcast, ty, bits)),
            | _ => {
                let result_ty = self.value_type(ty, source)?;
                Operand::Value(self.extend(bits, false, result_ty))
            },
        };
        Ok(Item { operand, ty })
    }

    pub(super) fn load_atomic_buffer(
        &mut self,
        address: Value,
        ty: TypeId,
        bits_ty: Type,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let size = if self.repr(ty, source)?.value_type() == Some(Type::F80) {
            10
        } else {
            types::layout(&self.unit.sema.types, ty)
                .map_err(|kind| crate::lowering::module_items::at(kind, source))?
                .size
        };
        let mut bits = self.iconst(bits_ty, 0);
        for offset in 0..size {
            let addr = self.offset_address(address, offset);
            let byte = self.inst(
                InstData::Load {
                    ty: Type::I8,
                    flags: InstFlags::empty(),
                    align: Align::BYTE,
                    tag: None,
                    addr,
                },
                Type::I8,
            );
            let byte = self.extend(byte, false, bits_ty);
            let shift = self.iconst(bits_ty, i128::from(offset * 8));
            let shifted = self.plain(Opcode::Shl, byte, shift);
            bits = self.plain(Opcode::Or, bits, shifted);
        }
        Ok(bits)
    }

    pub(super) fn store_atomic_buffer(
        &mut self,
        bits: Value,
        address: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let size = types::layout(&self.unit.sema.types, ty)
            .map_err(|kind| crate::lowering::module_items::at(kind, source))?
            .size;
        let bits_ty = self.draft.value_type(bits);
        for offset in 0..size {
            let shift = self.iconst(bits_ty, i128::from(offset * 8));
            let byte = self.plain(Opcode::Lshr, bits, shift);
            let byte = self.extend(byte, false, Type::I8);
            let address = self.offset_address(address, offset);
            self.plain_store_bits(byte, address, Align::BYTE, false);
        }
        Ok(())
    }

    pub(super) fn plain_store_bits(
        &mut self,
        value: Value,
        address: Value,
        align: Align,
        volatile: bool,
    ) {
        let ty = self.draft.value_type(value);
        _ = self.draft.push(
            Op::Inst(InstData::Store {
                ty,
                flags: if volatile {
                    InstFlags::VOLATILE
                } else {
                    InstFlags::empty()
                },
                align,
                tag: None,
                args: [value, address],
            }),
            None,
        );
    }
}
