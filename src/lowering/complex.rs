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
        IntCC,
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
        semantic_analysis::{
            Scalar,
            TypeId,
            TypeKind,
        },
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
        // A `_Complex _Float16` excess is rounded to the parts it stores.
        let item = self.settle(item);
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
    /// A GNU complex integer type's parts convert as its real type does.
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
            | Repr::Complex { .. } => {
                let part = self.part_repr(to, source)?;
                let pair = if let Repr::Complex { .. } = from {
                    // The parts' own type: `f32` for a `_Complex _Float16`
                    // excess.
                    let pair = self.complex_parts(item, source)?;
                    let old = self.parts_repr(item.ty, pair[0], source)?;
                    [
                        self.part_from(pair[0], old, part, source)?,
                        self.part_from(pair[1], old, part, source)?,
                    ]
                } else {
                    let value = self.value(item, source)?;
                    let zero = self.part_zero(part);
                    [self.part_from(value, from, part, source)?, zero]
                };
                self.complex_pair(pair)
            },
            | Repr::Void => Operand::Void,
            | Repr::Bool => {
                let truth = self.truth(item, source)?;
                Operand::Value(self.extend(truth, false, Type::I8))
            },
            | representation @ (Repr::Float { .. } | Repr::Int { .. }) => {
                let real = self.complex_parts(item, source)?[0];
                let old = self.parts_repr(item.ty, real, source)?;
                let value = self.part_from(real, old, representation, source)?;
                Operand::Value(if let Repr::Int { .. } = representation {
                    self.normalize(value, representation)
                } else {
                    value
                })
            },
            | _ => return Err(LoweringError::missing("invalid complex conversion", source)),
        };
        Ok(Item { operand, ty: to })
    }

    /// The representation of one part of the complex type `ty`.
    pub(super) fn part_repr(
        &self,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Repr, LoweringError> {
        let ty = self.unit.sema.types.non_atomic(ty);
        let TypeKind::Scalar(scalar) = self.unit.sema.types.kind(ty) else {
            return Err(LoweringError::missing(
                "a complex type has no scalar",
                source,
            ));
        };
        types::scalar_repr(&self.unit.sema.types, scalar.real_type())
            .map_err(|kind| at(kind, source))
    }

    /// The representation of the parts a complex value of type `ty` holds:
    /// its real type's, or a floating one in the parts' own IR type (`f32`
    /// for a `_Complex _Float16` excess).
    fn parts_repr(
        &self,
        ty: TypeId,
        part: Value,
        source: SourceVectors,
    ) -> Result<Repr, LoweringError> {
        let representation = self.part_repr(ty, source)?;
        Ok(match representation {
            | Repr::Float { .. } => Repr::Float {
                ty: self.draft.value_type(part),
            },
            | representation => representation,
        })
    }

    /// Zero in a part of representation `part`.
    fn part_zero(&mut self, part: Repr) -> Value {
        match part {
            | Repr::Float { ty } => self.fconst(ty, 0),
            | Repr::Int { ty, .. } => self.iconst(ty, 0),
            | _ => self.iconst(Type::I8, 0),
        }
    }

    /// `value`, of representation `from`, converted to a part of
    /// representation `to`: a floating part as `float_from` converts, an
    /// integer one truncated toward zero from a floating value (§6.3.1.4p1)
    /// or extended or wrapped from an integer (§6.3.1.3).
    fn part_from(
        &mut self,
        value: Value,
        from: Repr,
        to: Repr,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        match to {
            | Repr::Float { ty } => self.float_from(value, from, ty, source),
            | Repr::Int { ty, signed, .. } => Ok(match from {
                | Repr::Float { .. } => {
                    let mut value = value;
                    // A 16-bit part widens exactly first.
                    if self.draft.value_type(value).half_format().is_some() {
                        value = self.widen_half(value);
                    }
                    let opcode = if signed {
                        Opcode::Fptosi
                    } else {
                        Opcode::Fptoui
                    };
                    self.convert(opcode, ty, value)
                },
                | from => self.extend(value, from.signed(), ty),
            }),
            | _ => Err(LoweringError::missing(
                "invalid complex part conversion",
                source,
            )),
        }
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
        if !component.is_float() {
            return self.complex_integer_binary(operator, left, right, ty, domains, source);
        }
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
            | B::Multiplication | B::Division => {
                let pair = self.complex_runtime(operator == B::Division, a, b, source)?;
                self.complex_pair(pair)
            },
            | _ => return Err(LoweringError::missing("invalid complex operator", source)),
        };
        Ok(Item { operand, ty })
    }

    /// GNU complex integer arithmetic in the parts' type, wrapping as
    /// unsigned arithmetic does: the sum, difference and product of each
    /// part (`(ac - bd) + (ad + bc)i`), equality of both parts, and a
    /// quotient by the identity's formula
    /// ([`CompilerConfiguration::complex_integer_rules`](crate::configuration::CompilerConfiguration::complex_integer_rules)).
    /// A real divisor divides each part, as both compilers do.
    /// GNU extension: GCC manual, "Complex Numbers".
    fn complex_integer_binary(
        &mut self,
        operator: BinaryOperator,
        left: Item,
        right: Item,
        ty: TypeId,
        domains: u8,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        use BinaryOperator as B;
        let flags = InstFlags::empty();
        let parts = |lowerer: &mut Self, item: Item, complex: bool| -> Result<[Value; 2], _> {
            if complex {
                lowerer.complex_parts(item, source)
            } else {
                let value = lowerer.value(item, source)?;
                let zero = lowerer.iconst(lowerer.draft.value_type(value), 0);
                Ok([value, zero])
            }
        };
        let a = parts(self, left, domains & 1 != 0)?;
        let b = parts(self, right, domains & 2 != 0)?;
        let operand = match operator {
            | B::Equal | B::NotEqual => {
                let cc = if operator == B::Equal {
                    IntCC::Eq
                } else {
                    IntCC::Ne
                };
                let re = self.icmp(cc, a[0], b[0]);
                let im = self.icmp(cc, a[1], b[1]);
                let opcode = if operator == B::Equal {
                    Opcode::And
                } else {
                    Opcode::Or
                };
                Operand::Truth(self.binary_inst(opcode, flags, re, im))
            },
            | B::Addition | B::Subtraction => {
                let opcode = if operator == B::Addition {
                    Opcode::Iadd
                } else {
                    Opcode::Isub
                };
                let pair = [
                    self.binary_inst(opcode, flags, a[0], b[0]),
                    self.binary_inst(opcode, flags, a[1], b[1]),
                ];
                self.complex_pair(pair)
            },
            | B::Multiplication => {
                let ac = self.binary_inst(Opcode::Imul, flags, a[0], b[0]);
                let bd = self.binary_inst(Opcode::Imul, flags, a[1], b[1]);
                let ad = self.binary_inst(Opcode::Imul, flags, a[0], b[1]);
                let bc = self.binary_inst(Opcode::Imul, flags, a[1], b[0]);
                let pair = [
                    self.binary_inst(Opcode::Isub, flags, ac, bd),
                    self.binary_inst(Opcode::Iadd, flags, ad, bc),
                ];
                self.complex_pair(pair)
            },
            | B::Division => {
                let signed = self.part_repr(ty, source)?.signed();
                let divide = if signed { Opcode::Sdiv } else { Opcode::Udiv };
                let pair = if domains & 2 == 0 {
                    [
                        self.binary_inst(divide, flags, a[0], b[0]),
                        self.binary_inst(divide, flags, a[1], b[0]),
                    ]
                } else if self.unit.context.configuration.complex_integer_rules()
                    == crate::configuration::RuleSet::Gcc
                {
                    self.gcc_integer_quotient(a, b, signed)
                } else {
                    // Clang: `(ac + bd) / (cc + dd)` and `(bc - ad) / (cc +
                    // dd)`.
                    let cc = self.binary_inst(Opcode::Imul, flags, b[0], b[0]);
                    let dd = self.binary_inst(Opcode::Imul, flags, b[1], b[1]);
                    let ac = self.binary_inst(Opcode::Imul, flags, a[0], b[0]);
                    let bd = self.binary_inst(Opcode::Imul, flags, a[1], b[1]);
                    let bc = self.binary_inst(Opcode::Imul, flags, a[1], b[0]);
                    let ad = self.binary_inst(Opcode::Imul, flags, a[0], b[1]);
                    let denominator = self.binary_inst(Opcode::Iadd, flags, cc, dd);
                    let real = self.binary_inst(Opcode::Iadd, flags, ac, bd);
                    let imaginary = self.binary_inst(Opcode::Isub, flags, bc, ad);
                    [
                        self.binary_inst(divide, flags, real, denominator),
                        self.binary_inst(divide, flags, imaginary, denominator),
                    ]
                };
                self.complex_pair(pair)
            },
            | _ => return Err(LoweringError::missing("invalid complex operator", source)),
        };
        Ok(Item { operand, ty })
    }

    /// GCC's quotient of complex integers `a / b` (`expand_complex_div_wide`
    /// in its `tree-complex.cc`): dividing by the part of the divisor with
    /// the larger magnitude first, in the parts' type. Where `|c| < |d|`,
    /// with `r = c / d` and `t = c r + d`, it is `((a r + b) / t) + ((b r -
    /// a) / t)i`; otherwise, with `r = d / c` and `t = d r + c`, `((b r + a)
    /// / t) + ((b - a r) / t)i`. The unused ratio divides by 1, so only a
    /// zero divisor traps, as GCC's does.
    fn gcc_integer_quotient(&mut self, a: [Value; 2], b: [Value; 2], signed: bool) -> [Value; 2] {
        let flags = InstFlags::empty();
        let ty = self.draft.value_type(a[0]);
        let divide = if signed { Opcode::Sdiv } else { Opcode::Udiv };
        let zero = self.iconst(ty, 0);
        let one = self.iconst(ty, 1);
        let magnitude = |lowerer: &mut Self, value: Value| {
            if signed {
                let negative = lowerer.icmp(IntCC::Slt, value, zero);
                let negated = lowerer.binary_inst(Opcode::Isub, flags, zero, value);
                lowerer.choose(negative, negated, value)
            } else {
                value
            }
        };
        let (c, d) = (b[0], b[1]);
        let c_magnitude = magnitude(self, c);
        let d_magnitude = magnitude(self, d);
        let smaller = self.icmp(
            if signed { IntCC::Slt } else { IntCC::Ult },
            c_magnitude,
            d_magnitude,
        );
        // |c| < |d|: r = c / d, t = c r + d.
        let d_divisor = self.choose(smaller, d, one);
        let small_ratio = self.binary_inst(divide, flags, c, d_divisor);
        let small_scaled = self.binary_inst(Opcode::Imul, flags, c, small_ratio);
        let small_total = self.binary_inst(Opcode::Iadd, flags, small_scaled, d);
        let ar = self.binary_inst(Opcode::Imul, flags, a[0], small_ratio);
        let small_real = self.binary_inst(Opcode::Iadd, flags, ar, a[1]);
        let br = self.binary_inst(Opcode::Imul, flags, a[1], small_ratio);
        let small_imaginary = self.binary_inst(Opcode::Isub, flags, br, a[0]);
        // Otherwise: r = d / c, t = d r + c.
        let c_divisor = self.choose(smaller, one, c);
        let large_ratio = self.binary_inst(divide, flags, d, c_divisor);
        let large_scaled = self.binary_inst(Opcode::Imul, flags, d, large_ratio);
        let large_total = self.binary_inst(Opcode::Iadd, flags, large_scaled, c);
        let br = self.binary_inst(Opcode::Imul, flags, a[1], large_ratio);
        let large_real = self.binary_inst(Opcode::Iadd, flags, br, a[0]);
        let ar = self.binary_inst(Opcode::Imul, flags, a[0], large_ratio);
        let large_imaginary = self.binary_inst(Opcode::Isub, flags, a[1], ar);
        let total = self.choose(smaller, small_total, large_total);
        let real = self.choose(smaller, small_real, large_real);
        let imaginary = self.choose(smaller, small_imaginary, large_imaginary);
        [
            self.binary_inst(divide, flags, real, total),
            self.binary_inst(divide, flags, imaginary, total),
        ]
    }

    /// The product or quotient of two complex values from libgcc's
    /// `__mulsc3`, `__divsc3` and their relatives, chosen by the parts'
    /// type; a `_Complex _Float16` computes in `float _Complex`.
    fn complex_runtime(
        &mut self,
        divide: bool,
        a: [Value; 2],
        b: [Value; 2],
        source: SourceVectors,
    ) -> Result<[Value; 2], LoweringError> {
        let component = self.draft.value_type(a[0]);
        let scalar = match component {
            | Type::F32 => Scalar::ComplexFloat,
            | Type::F64 if self.unit.sema.types.target.long_double_is_double() =>
                Scalar::ComplexLongDouble,
            | Type::F64 => Scalar::ComplexDouble,
            | Type::F80 => Scalar::ComplexLongDouble,
            | _ => Scalar::ComplexFloat128,
        };
        let name = match (divide, component) {
            | (false, Type::F32) => "__mulsc3",
            | (true, Type::F32) => "__divsc3",
            | (false, Type::F64) => "__muldc3",
            | (true, Type::F64) => "__divdc3",
            | (false, Type::F80) => "__mulxc3",
            | (true, Type::F80) => "__divxc3",
            | (false, Type::F128) => "__multc3",
            | (true, Type::F128) => "__divtc3",
            | _ => return Err(LoweringError::missing("complex runtime precision", source)),
        };
        let layout = types::complex_scalar_layout(&self.unit.sema.types, scalar, self.scratch)
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
        let half = layout.size / 2;
        let align = crate::ir::Align::from_bytes(layout.align.bytes().min(half))
            .expect("component alignment");
        let mut parts = [object; 2];
        for (index, part) in parts.iter_mut().enumerate() {
            let address = self.offset_address(object, index as u64 * half);
            *part = self.inst(
                InstData::Load {
                    ty: component,
                    flags: InstFlags::empty(),
                    align,
                    tag: None,
                    addr: address,
                },
                component,
            );
        }
        Ok(parts)
    }
}
