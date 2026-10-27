//! `_Float16` values evaluated in `float`. Neither GCC nor Clang computes in
//! binary16 on x86-64 without AVX512-FP16: an operation converts its operands
//! to `f32`, computes, and the result keeps that range and precision until a
//! consumer rounds it, so `a * b / c` rounds once, not twice.
//!
//! An [`Operand::Excess`] holds such an `f32`. The arithmetic operators and
//! unary `+` and `-` produce one from operands they read with
//! [`FunctionLowerer::excess`]; popping an operand with
//! [`FunctionLowerer::pop`] rounds it to an `f16` with
//! [`FunctionLowerer::settle`], so every consumer that does not ask for the
//! excess reads the rounded value, as Clang's code generation does. GCC also
//! keeps the excess through a conversion to another type, a comparison,
//! `?:` and the comma operator, and converts an integer operand straight to
//! `float`; [`CompilerConfiguration::half_in_float`] selects that rule.
//! Semantic analysis folds constants by the same rules.
//!
//! For `h = a * b / c`, both operands of `*` widen with `fpext`, `fmul.f32`
//! gives an excess, `/` reads it and the widened `c`, and the assignment
//! rounds the quotient once with `fptrunc`.
//!
//! [`CompilerConfiguration::half_in_float`]: crate::configuration::CompilerConfiguration::half_in_float
//!
//! C99: §5.2.4.2.2 paragraph 7, p. 24; PDF p. 36 (evaluation formats);
//! §6.3.1.8 paragraph 2, p. 45; PDF p. 57 (excess range and precision, which
//! assignment and casts remove). C23: Annex H, §H.2.2 paragraph 1, p. 553;
//! PDF p. 566 (binary16).

use super::{
    FunctionLowerer,
    LoweringError,
    expressions::{
        comparison,
        float_comparison,
    },
    floats,
    types::Repr,
    work::{
        Item,
        Operand,
    },
};
use crate::{
    half_precision::HalfFormat,
    ir::{
        FloatCC,
        InstFlags,
        Opcode,
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
    /// Rounds an excess `_Float16` operand to its `f16` value; any other
    /// operand is returned unchanged. A constant rounds here.
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
    pub(super) fn settle(&mut self, item: Item) -> Item {
        let Operand::Excess(value) = item.operand else {
            return item;
        };
        let rounded = match self.single_constant(value) {
            | Some(single) => self.fconst(
                Type::F16,
                u128::from(HalfFormat::Binary16.round_f32(single)),
            ),
            | None => self.convert(Opcode::Fptrunc, Type::F16, value),
        };
        Item {
            operand: Operand::Value(rounded),
            ..item
        }
    }

    /// The `f32` a `_Float16` operand contributes to an operation: its
    /// excess, or its `f16` value widened exactly.
    /// C99: §6.3.1.5 paragraph 1, p. 44; PDF p. 56.
    pub(super) fn excess(
        &mut self,
        item: Item,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        if let Operand::Excess(value) = item.operand {
            return Ok(value);
        }
        let value = self.value(item, source)?;
        Ok(self.widen_half(value))
    }

    /// An `f16` value widened exactly to `f32`; a constant widens here.
    pub(super) fn widen_half(&mut self, value: Value) -> Value {
        match self.draft.float_constant(value) {
            | Some(bits) => {
                let half = u16::try_from(bits).expect("an f16 constant has 16 bits");
                self.fconst(
                    Type::F32,
                    u128::from(HalfFormat::Binary16.widen(half).to_bits()),
                )
            },
            | None => self.convert(Opcode::Fpext, Type::F32, value),
        }
    }

    /// The excess of a folded `_Float16` constant: semantic analysis keeps
    /// it at `float` precision, so its `f32` bits are exact.
    pub(super) fn excess_constant(
        &mut self,
        folded: &crate::float_parsing::LongDouble,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let bits = floats::constant_bits(folded, Type::F32)
            .ok_or_else(|| LoweringError::missing("a floating constant has no bits", source))?;
        Ok(Operand::Excess(self.fconst(Type::F32, bits)))
    }

    /// A binary operator whose operands are `_Float16`: `+`, `-`, `*` and
    /// `/` compute in `f32` and give an excess; a comparison compares the
    /// operands' excess for GCC and their rounded values for Clang.
    /// C99: §6.5.5-§6.5.9, pp. 82-86; PDF pp. 94-98.
    pub(super) fn half_binary(
        &mut self,
        operator: BinaryOperator,
        left: Item,
        right: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        use BinaryOperator as B;
        if let Some(cc) = comparison(operator) {
            let (left, right) = if self.half_in_float() {
                (left, right)
            } else {
                (self.settle(left), self.settle(right))
            };
            let a = self.excess(left, source)?;
            let b = self.excess(right, source)?;
            let truth = self.fcmp(float_comparison(cc), a, b);
            return Ok(Item {
                operand: Operand::Truth(truth),
                ty,
            });
        }
        let opcode = match operator {
            | B::Multiplication => Opcode::Fmul,
            | B::Addition => Opcode::Fadd,
            | B::Subtraction => Opcode::Fsub,
            | B::Division => Opcode::Fdiv,
            | _ =>
                return Err(LoweringError::missing(
                    "a floating operator has no lowering",
                    source,
                )),
        };
        let a = self.excess(left, source)?;
        let b = self.excess(right, source)?;
        let value = self.binary_inst(opcode, InstFlags::empty(), a, b);
        Ok(Item {
            operand: Operand::Excess(value),
            ty,
        })
    }

    /// GCC's conversion of a `_Float16` excess to another type: from the
    /// `f32` value itself, without rounding it to `_Float16` first.
    /// C99: §6.3.1.4-§6.3.1.5, pp. 43-44; PDF pp. 55-56.
    pub(super) fn convert_excess(
        &mut self,
        item: Item,
        to: TypeId,
        target: Repr,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let Operand::Excess(value) = item.operand else {
            return self.convert_value(item, to, source);
        };
        let operand = match target {
            | Repr::Void => Operand::Void,
            | Repr::Bool => {
                let zero = self.fconst(Type::F32, 0);
                let truth = self.fcmp(FloatCC::Une, value, zero);
                Operand::Value(self.extend(truth, false, Type::I8))
            },
            | Repr::Int { ty, signed } => {
                let opcode = if signed {
                    Opcode::Fptosi
                } else {
                    Opcode::Fptoui
                };
                Operand::Value(self.convert(opcode, ty, value))
            },
            | Repr::Float { ty } =>
                Operand::Value(self.float_from(value, Repr::Float { ty: Type::F32 }, ty, source)?),
            // The real part converts from the excess; the imaginary part is
            // zero (C99 §6.3.1.7 paragraph 1, p. 44; PDF p. 56).
            | Repr::Complex { component } => {
                let real =
                    self.float_from(value, Repr::Float { ty: Type::F32 }, component, source)?;
                let imag = self.fconst(component, 0);
                self.complex_pair([real, imag])
            },
            | _ => {
                let item = self.settle(item);
                return self.convert_value(item, to, source);
            },
        };
        Ok(Item { operand, ty: to })
    }

    /// Whether `ty` is `_Float16`.
    pub(super) fn half(&self, ty: TypeId) -> bool {
        self.repr(ty, SourceVectors::empty())
            .is_ok_and(|representation| representation == Repr::Float { ty: Type::F16 })
    }

    /// Whether GCC's rule keeps the excess beyond arithmetic operators.
    pub(super) fn half_in_float(&self) -> bool {
        self.unit.context.configuration.half_in_float()
    }

    fn single_constant(&self, value: Value) -> Option<f32> {
        let bits = self.draft.float_constant(value)?;
        Some(f32::from_bits(u32::try_from(bits).ok()?))
    }
}
