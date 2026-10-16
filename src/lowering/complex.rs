//! Complex scalar values are immutable pairs in function scratch. Object
//! accesses capture both components; only ABI boundaries materialize objects.
//! Arithmetic helpers use the target's ordinary C complex result convention.
//! C99: §6.2.5p13, p. 34; PDF p. 46; §6.3.1.6-§6.3.1.7, p. 44;
//! PDF p. 56; §6.5.5-§6.5.6, pp. 82-84; PDF pp. 94-96.

use super::{
    FunctionLowerer,
    LoweringError,
    module_items::at,
    ssa::Op,
    types::{
        self,
        Repr,
    },
    work::{
        Item,
        Operand,
        Place,
    },
};
use crate::{
    ir::{
        AggregateParam,
        FloatCC,
        InstData,
        InstFlags,
        Linkage,
        Opcode,
        ParamAbi,
        Signature,
        Symbol,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::BinaryOperator,
        semantic_analysis::TypeId,
    },
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    pub(super) fn complex_pair(&mut self, pair: [Value; 2]) -> Operand {
        let id = u32::try_from(self.complex_values.len()).expect("complex value index fits u32");
        self.complex_values.push(pair);
        Operand::Complex(id)
    }

    pub(super) fn complex_parts(
        &self,
        item: Item,
        source: SourceVectors,
    ) -> Result<[Value; 2], LoweringError> {
        if let Operand::Complex(id) = item.operand {
            Ok(self.complex_values[id as usize])
        } else {
            Err(LoweringError::missing(
                "a complex value has no components",
                source,
            ))
        }
    }

    /// Snapshot a scalar before any subsequent evaluation can mutate it.
    /// C99: §6.3.2.1p2, p. 46; PDF p. 58.
    pub(super) fn load_complex(
        &mut self,
        address: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let Repr::Complex { component } = self.repr(ty, source)? else {
            return Err(LoweringError::missing("complex load type", source));
        };
        let layout = types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
        let align = self.place_alignment(address, types::align(layout));
        let flags = if types::is_volatile(ty) {
            InstFlags::VOLATILE
        } else {
            InstFlags::empty()
        };
        let mut values = [address; 2];
        for (index, value) in values.iter_mut().enumerate() {
            let at = self.offset_address(address, index as u64 * (layout.size / 2));
            let align = crate::ir::Align::from_bytes(align.bytes().min(layout.size / 2))
                .expect("component alignment");
            *value = self.inst(
                InstData::Load {
                    ty: component,
                    flags,
                    align,
                    tag: None,
                    addr: at,
                },
                component,
            );
        }
        Ok(self.complex_pair(values))
    }

    /// Write only component value bytes, leaving x87 padding unspecified.
    /// C99: §6.5.16p3, p. 91; PDF p. 103; §6.2.5p13, p. 34; PDF p. 46.
    pub(super) fn store_complex(
        &mut self,
        address: Value,
        item: Item,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let pair = self.complex_parts(item, source)?;
        let layout =
            types::layout(&self.unit.sema.types, item.ty).map_err(|kind| at(kind, source))?;
        let align = self.place_alignment(address, types::align(layout));
        let flags = if types::is_volatile(item.ty) {
            InstFlags::VOLATILE
        } else {
            InstFlags::empty()
        };
        for (index, value) in pair.into_iter().enumerate() {
            let at = self.offset_address(address, index as u64 * (layout.size / 2));
            let align = crate::ir::Align::from_bytes(align.bytes().min(layout.size / 2))
                .expect("component alignment");
            _ = self.draft.push(
                self.memory_op(InstData::Store {
                    ty: self.draft.value_type(value),
                    flags,
                    align,
                    tag: None,
                    args: [value, at],
                }),
                None,
            );
        }
        Ok(())
    }

    /// A call receives a fresh copy of the already evaluated scalar value.
    /// C99: §6.5.2.2p4, p. 71; PDF p. 83.
    pub(super) fn materialize_complex(
        &mut self,
        item: Item,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        // The ABI packer may load at the nominal scalar alignment even when
        // a typedef lowers its source object's storage alignment.
        let layout = types::complex_layout(&self.unit.sema.types, item.ty, self.scratch)
            .map_err(|kind| at(kind, source))?;
        let address = self.fresh_object(layout.size, layout.align, source)?;
        self.store_complex(address, item, source)?;
        Ok(address)
    }

    /// GNU component designators preserve complex lvalue qualifiers and
    /// storage. C99 representation: §6.2.5p13, p. 34; PDF p. 46.
    pub(super) fn complex_component(
        &mut self,
        item: Item,
        result: TypeId,
        imaginary: bool,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        if matches!(self.repr(item.ty, source)?, Repr::Complex { .. }) {
            if let Operand::Place(Place::Memory(base)) = item.operand {
                let layout = types::layout(&self.unit.sema.types, item.ty)
                    .map_err(|kind| at(kind, source))?;
                let guaranteed = self.place_alignment(base, types::align(layout));
                let address =
                    self.offset_address(base, if imaginary { layout.size / 2 } else { 0 });
                self.retain_alignment(
                    address,
                    crate::ir::Align::from_bytes(guaranteed.bytes().min(layout.size / 2))
                        .expect("component alignment"),
                );
                return Ok(Operand::Place(Place::Memory(address)));
            }
            return Ok(Operand::Value(
                self.complex_parts(item, source)?[usize::from(imaginary)],
            ));
        }
        if imaginary {
            // GNU __imag__ evaluates a real operand even though its value is
            // zero. In particular, its lvalue conversion observes volatile.
            if let Operand::Place(place) = item.operand {
                _ = self.read(place, item.ty, item.ty, source)?;
            }
            let ty = self.value_type(result, source)?;
            return Ok(Operand::Value(if ty.is_float() {
                self.fconst(ty, 0)
            } else {
                self.iconst(ty, 0)
            }));
        }
        Ok(item.operand)
    }

    /// Componentwise conversions retain unsigned integer source semantics.
    /// C99: §6.3.1.2p1, p. 43; PDF p. 55; §6.3.1.6-§6.3.1.7, p. 44; PDF p. 56.
    pub(super) fn convert_complex(
        &mut self,
        item: Item,
        to: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let from = self.repr(item.ty, source)?;
        let target = self.repr(to, source)?;
        let operand = match target {
            | Repr::Complex { component } => {
                let pair = if let Repr::Complex { component: old } = from {
                    let pair = self.complex_parts(item, source)?;
                    [
                        self.float_from(pair[0], Repr::Float { ty: old }, component, source)?,
                        self.float_from(pair[1], Repr::Float { ty: old }, component, source)?,
                    ]
                } else {
                    let value = self.value(item, source)?;
                    [
                        self.float_from(value, from, component, source)?,
                        self.fconst(component, 0),
                    ]
                };
                self.complex_pair(pair)
            },
            | Repr::Void => Operand::Void,
            | Repr::Bool => {
                let truth = self.truth(item, source)?;
                Operand::Value(self.extend(truth, false, Type::I8))
            },
            | Repr::Float { ty } => {
                let real = self.complex_parts(item, source)?[0];
                Operand::Value(self.float_from(
                    real,
                    Repr::Float {
                        ty: self.draft.value_type(real),
                    },
                    ty,
                    source,
                )?)
            },
            | Repr::Int { ty, signed } => {
                let real = self.complex_parts(item, source)?[0];
                Operand::Value(self.convert(
                    if signed {
                        Opcode::Fptosi
                    } else {
                        Opcode::Fptoui
                    },
                    ty,
                    real,
                ))
            },
            | _ => return Err(LoweringError::missing("invalid complex conversion", source)),
        };
        Ok(Item { operand, ty: to })
    }

    /// Evaluate in the operands' common real precision, preserving domains.
    /// C99: §6.3.1.8p1, pp. 44-45; PDF pp. 56-57;
    /// §6.5.5-§6.5.6, pp. 82-84; PDF pp. 94-96;
    /// §6.5.9p3-4, p. 86; PDF p. 98.
    pub(super) fn complex_binary(
        &mut self,
        operator: BinaryOperator,
        left: Item,
        right: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        use BinaryOperator as B;
        let domains = u8::from(matches!(left.operand, Operand::Complex(_)))
            | (u8::from(matches!(right.operand, Operand::Complex(_))) << 1);
        let component = if domains & 1 != 0 {
            self.draft.value_type(self.complex_parts(left, source)?[0])
        } else {
            self.draft.value_type(self.complex_parts(right, source)?[0])
        };
        let a = if domains & 1 != 0 {
            self.complex_parts(left, source)?
        } else {
            [self.value(left, source)?, self.fconst(component, 0)]
        };
        let b = if domains & 2 != 0 {
            self.complex_parts(right, source)?
        } else {
            [self.value(right, source)?, self.fconst(component, 0)]
        };
        if domains != 3
            && matches!(
                operator,
                B::Addition | B::Subtraction | B::Multiplication | B::Division
            )
        {
            let imaginary = if domains == 1 { a[1] } else { b[1] };
            let real_operand = if domains == 1 { b[0] } else { a[0] };
            let pair = match operator {
                | B::Addition => Some([
                    self.binary_inst(Opcode::Fadd, InstFlags::empty(), a[0], b[0]),
                    imaginary,
                ]),
                | B::Subtraction => {
                    let imag = if domains == 1 {
                        imaginary
                    } else {
                        self.convert(Opcode::Fneg, component, imaginary)
                    };
                    Some([
                        self.binary_inst(Opcode::Fsub, InstFlags::empty(), a[0], b[0]),
                        imag,
                    ])
                },
                | B::Multiplication => Some([
                    self.binary_inst(Opcode::Fmul, InstFlags::empty(), a[0], b[0]),
                    self.binary_inst(Opcode::Fmul, InstFlags::empty(), imaginary, real_operand),
                ]),
                | B::Division if domains == 1 => Some([
                    self.binary_inst(Opcode::Fdiv, InstFlags::empty(), a[0], b[0]),
                    self.binary_inst(Opcode::Fdiv, InstFlags::empty(), imaginary, b[0]),
                ]),
                | _ => None,
            };
            if let Some(pair) = pair {
                return Ok(Item {
                    operand: self.complex_pair(pair),
                    ty,
                });
            }
        }
        let operand = match operator {
            | B::Equal | B::NotEqual => {
                let cc = if operator == B::Equal {
                    FloatCC::Oeq
                } else {
                    FloatCC::Une
                };
                let re = self.fcmp(cc, a[0], b[0]);
                let im = self.fcmp(cc, a[1], b[1]);
                Operand::Truth(self.binary_inst(
                    if operator == B::Equal {
                        Opcode::And
                    } else {
                        Opcode::Or
                    },
                    InstFlags::empty(),
                    re,
                    im,
                ))
            },
            | B::Addition | B::Subtraction => {
                let op = if operator == B::Addition {
                    Opcode::Fadd
                } else {
                    Opcode::Fsub
                };
                let pair = [
                    self.binary_inst(op, InstFlags::empty(), a[0], b[0]),
                    self.binary_inst(op, InstFlags::empty(), a[1], b[1]),
                ];
                self.complex_pair(pair)
            },
            | B::Multiplication | B::Division =>
                self.complex_runtime(operator == B::Division, a, b, ty, source)?,
            | _ => return Err(LoweringError::missing("invalid complex operator", source)),
        };
        Ok(Item { operand, ty })
    }

    fn complex_runtime(
        &mut self,
        divide: bool,
        a: [Value; 2],
        b: [Value; 2],
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let component = self.draft.value_type(a[0]);
        let name = match (divide, component) {
            | (false, Type::F32) => "__mulsc3",
            | (true, Type::F32) => "__divsc3",
            | (false, Type::F64) => "__muldc3",
            | (true, Type::F64) => "__divdc3",
            | (false, Type::F80) => "__mulxc3",
            | (true, Type::F80) => "__divxc3",
            | _ => return Err(LoweringError::missing("complex runtime precision", source)),
        };
        let layout = types::complex_layout(&self.unit.sema.types, ty, self.scratch)
            .map_err(|kind| at(kind, source))?;
        let signature = Signature {
            params:           &[Type::Ptr, component, component, component, component],
            aggregates:       &[AggregateParam {
                index: 0,
                abi:   ParamAbi::Result(layout),
            }],
            noundef:          &[1, 2, 3, 4],
            result:           None,
            variadic:         false,
            variadic_fixed:   None,
            extensions:       &[],
            result_extension: None,
            result_noundef:   false,
            variadic_noundef: false,
        };
        let signature = self.unit.module.intern(signature);
        let function = match self.unit.module.symbol(name) {
            | Some(Symbol::Function(function)) => function,
            | Some(_) =>
                return Err(LoweringError::missing(
                    "complex runtime symbol collision",
                    source,
                )),
            | None => self
                .unit
                .module
                .declare_function(name, signature, Linkage::External),
        };
        let object = self.fresh_object(layout.size, layout.align, source)?;
        let args = self
            .scratch
            .alloc_slice_copy(&[object, a[0], a[1], b[0], b[1]]);
        _ = self.draft.push(Op::Call(function, args), None);
        self.load_complex(object, ty, source)
    }
}
