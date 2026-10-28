//! `_Float16` and `__bf16` values evaluated in `float`. Neither GCC nor
//! Clang computes in binary16 or bfloat16 on x86-64 without AVX512-FP16: an
//! operation converts its operands to `f32`, computes, and the result keeps
//! that range and precision until a consumer rounds it, so `a * b / c` rounds
//! once, not twice.
//!
//! An [`Operand::Excess`] holds such an `f32`. The arithmetic operators and
//! unary `+` and `-` produce one from operands they read with
//! [`FunctionLowerer::excess`]; popping an operand with
//! [`FunctionLowerer::pop`] rounds it to an `f16` or `bf16` with
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
    ir::{
        FloatCC,
        InstFlags,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator,
            Expression,
        },
        semantic_analysis::{
            self,
            Scalar,
            TypeId,
            TypeKind,
        },
    },
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Rounds an excess `_Float16` or `__bf16` operand to its `f16` or
    /// `bf16` value; any other operand is returned unchanged. A constant
    /// rounds here.
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
    pub(super) fn settle(&mut self, item: Item) -> Item {
        if let Some([real, imag]) = self.complex_excess(item) {
            let pair = [
                self.round_single(real, Type::F16),
                self.round_single(imag, Type::F16),
            ];
            return Item {
                operand: self.complex_pair(pair),
                ..item
            };
        }
        let Operand::Excess(value) = item.operand else {
            return item;
        };
        let ty = self.half(item.ty).expect("an excess has a half type");
        Item {
            operand: Operand::Value(self.round_single(value, ty)),
            ..item
        }
    }

    /// The `f32` parts of a `_Complex _Float16` value evaluated in `float
    /// _Complex` and not yet rounded: an excess held as a complex pair.
    pub(super) fn complex_excess(&self, item: Item) -> Option<[Value; 2]> {
        let Operand::Complex(id) = item.operand else {
            return None;
        };
        let parts = self.complex_values[id as usize];
        (self.complex_half(item.ty) && self.draft.value_type(parts[0]) == Type::F32)
            .then_some(parts)
    }

    /// A binary operator whose result or operands are `_Complex _Float16`.
    /// Both compilers compute it in `float _Complex`, with `__mulsc3` and
    /// `__divsc3`, and keep the result's excess as for a real `_Float16`;
    /// a comparison reads the operands' excess for GCC and their rounded
    /// values for Clang.
    /// C99: §6.5.5-§6.5.6, pp. 82-84; PDF pp. 94-96; §6.5.9, p. 86;
    /// PDF p. 98. C23: §H.2.2 paragraph 3, p. 553; PDF p. 566.
    pub(super) fn complex_half_binary(
        &mut self,
        operator: BinaryOperator,
        left: Item,
        right: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let keep = comparison(operator).is_none() || self.half_in_float();
        let left = self.single_operand(left, keep, source)?;
        let right = self.single_operand(right, keep, source)?;
        self.complex_binary(operator, left, right, ty, source)
    }

    /// A real or complex half operand as `f32` values: its excess when
    /// `keep`, and otherwise its rounded value widened exactly.
    pub(super) fn single_operand(
        &mut self,
        item: Item,
        keep: bool,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let item = if keep { item } else { self.settle(item) };
        let operand = match item.operand {
            | Operand::Complex(id) => {
                let [real, imag] = self.complex_values[id as usize];
                if self.draft.value_type(real) == Type::F16 {
                    let pair = [self.widen_half(real), self.widen_half(imag)];
                    self.complex_pair(pair)
                } else {
                    item.operand
                }
            },
            | Operand::Excess(value) => Operand::Value(value),
            | _ => {
                let value = self.value(item, source)?;
                Operand::Value(if self.draft.value_type(value).half_format().is_some() {
                    self.widen_half(value)
                } else {
                    value
                })
            },
        };
        Ok(Item { operand, ..item })
    }

    /// An `f32` rounded to the 16-bit type `ty`; a constant rounds here.
    pub(super) fn round_single(&mut self, value: Value, ty: Type) -> Value {
        match (self.single_constant(value), ty.half_format()) {
            | (Some(single), Some(format)) => self.fconst(ty, u128::from(format.round_f32(single))),
            | _ => self.convert(Opcode::Fptrunc, ty, value),
        }
    }

    /// The `f32` a `_Float16` or `__bf16` operand contributes to an
    /// operation: its excess, or its value widened exactly.
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

    /// An `f16` or `bf16` value widened exactly to `f32`; a constant widens
    /// here.
    pub(super) fn widen_half(&mut self, value: Value) -> Value {
        let format = self
            .draft
            .value_type(value)
            .half_format()
            .expect("a 16-bit floating value");
        match self.draft.float_constant(value) {
            | Some(bits) => {
                let half = u16::try_from(bits).expect("a 16-bit constant has 16 bits");
                self.fconst(Type::F32, u128::from(format.widen(half).to_bits()))
            },
            | None => self.convert(Opcode::Fpext, Type::F32, value),
        }
    }

    /// The excess of a folded `_Float16` or `__bf16` constant: semantic
    /// analysis keeps it at `float` precision, so its `f32` bits are exact.
    pub(super) fn excess_constant(
        &mut self,
        folded: &crate::float_parsing::LongDouble,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let bits = floats::constant_bits(folded, Type::F32)
            .ok_or_else(|| LoweringError::missing("a floating constant has no bits", source))?;
        Ok(Operand::Excess(self.fconst(Type::F32, bits)))
    }

    /// GCC's folded plain half constant: a value in its own format, unless
    /// semantic analysis kept a value it cannot hold (GCC's unrounded
    /// conversion of a run-time-like value), which stays a widened `f32`.
    /// One GCC does not fold as it parses converts as a run-time value.
    pub(super) fn plain_constant(
        &mut self,
        expression: &'tu Expression<'tu>,
        folded: &crate::float_parsing::LongDouble,
        half: Type,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let bits = floats::constant_bits(folded, half)
            .ok_or_else(|| LoweringError::missing("a floating constant has no bits", source))?;
        // Both are exact in `float`, so their binary32 bits compare them.
        let format = half.half_format().expect("a 16-bit floating type");
        let exact = u16::try_from(bits)
            .ok()
            .map(|bits| u128::from(format.widen(bits).to_bits()))
            == floats::constant_bits(folded, Type::F32);
        if !exact {
            let Operand::Excess(value) = self.excess_constant(folded, source)? else {
                unreachable!("an excess constant")
            };
            self.widened.push(value);
            return Ok(Operand::Excess(value));
        }
        let value = self.fconst(half, bits);
        if !self.folded_constant(expression) {
            self.runtime_constants.push(value);
        }
        Ok(Operand::Value(value))
    }

    /// A binary operator whose operands are `_Float16` or `__bf16`: `+`,
    /// `-`, `*` and `/` compute in `f32` and give an excess; a comparison
    /// compares the operands' excess for GCC and their rounded values for
    /// Clang.
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

    /// GCC's conversion of a `_Float16` or `__bf16` excess to another type:
    /// from the `f32` value itself, without rounding it to its type first.
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
            | Repr::Int { ty, signed, .. } => {
                let opcode = if signed {
                    Opcode::Fptosi
                } else {
                    Opcode::Fptoui
                };
                let value = self.convert(opcode, ty, value);
                Operand::Value(self.normalize(value, target))
            },
            | Repr::Float { ty } =>
                Operand::Value(self.float_from(value, Repr::Float { ty: Type::F32 }, ty, source)?),
            // To `_Complex _Float16` the excess stays one, in `f32` parts.
            | Repr::Complex { .. } if self.complex_half(to) => {
                let imag = self.fconst(Type::F32, 0);
                self.complex_pair([value, imag])
            },
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

    /// The IR type of `_Float16` (`f16`) or `__bf16` (`bf16`), which
    /// compute in `f32`; `None` for any other type, `__fp16` included,
    /// whose values semantic analysis promotes to `float`.
    pub(super) fn half(&self, ty: TypeId) -> Option<Type> {
        let types = &self.unit.sema.types;
        let mut ty = ty;
        while let TypeKind::Atomic(inner) = types.kind(ty) {
            ty = inner;
        }
        match types.kind(ty) {
            | TypeKind::Scalar(Scalar::Float16) => Some(Type::F16),
            | TypeKind::Scalar(Scalar::BFloat16) => Some(Type::BF16),
            | _ => None,
        }
    }

    /// GCC's conversion of a plain (not excess) value of one half type to
    /// the other, `_Float16` or `__bf16`, real or complex: the value
    /// widened exactly to `float`, its evaluation format, and not rounded.
    /// GCC rounds such a value only to store it, to pass or return it, or
    /// as a plain arm of a `?:` whose type is `_Complex _Float16`; a cast,
    /// which rounds an excess, leaves it, so `(_Float16)b` keeps a
    /// `__bf16` beyond binary16's range. `None` when the rule does not
    /// apply.
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56; §5.2.4.2.2 paragraph 7,
    /// p. 24; PDF p. 36 (evaluation formats).
    pub(super) fn widen_plain(&mut self, item: Item, to: TypeId) -> Option<Item> {
        // A plain `_Complex _Float16` gives its real part.
        let (value, from) = match item.operand {
            | Operand::Value(value) => (value, self.half(item.ty)?),
            | Operand::Complex(id)
                if self.complex_half(item.ty) && self.complex_excess(item).is_none() =>
                (self.complex_values[id as usize][0], Type::F16),
            | _ => return None,
        };
        let target = if self.complex_half(to) {
            Type::F16
        } else {
            self.half(to)?
        };
        if !self.half_in_float() || from == target {
            return None;
        }
        let constant =
            self.draft.float_constant(value).is_some() && !self.runtime_constants.contains(&value);
        let single = self.widen_half(value);
        self.widened.push(single);
        if constant {
            self.widened_constants.push(single);
        }
        let operand = if self.complex_half(to) {
            let imag = self.fconst(Type::F32, 0);
            self.complex_pair([single, imag])
        } else {
            Operand::Excess(single)
        };
        Some(Item { operand, ty: to })
    }

    /// Whether GCC holds `item`, a half value, as a plain value: its own
    /// format's value (in a register or as a folded constant), or one
    /// [`Self::widen_plain`] gave. Clang has no such class.
    pub(super) fn plain_half(&self, item: Item) -> bool {
        self.half_in_float()
            && match item.operand {
                | Operand::Value(_) => true,
                | Operand::Complex(_) => self.complex_excess(item).is_none() || self.widened(item),
                | _ => self.widened(item),
            }
    }

    /// Whether `item` holds a value [`Self::widen_plain`] gave.
    pub(super) fn widened(&self, item: Item) -> bool {
        let value = match item.operand {
            | Operand::Excess(value) => value,
            | Operand::Complex(id) => self.complex_values[id as usize][0],
            | _ => return false,
        };
        self.widened.contains(&value)
    }

    /// Whether `item` holds a constant [`Self::widen_plain`] widened, which
    /// GCC rounds as a `?:` arm: it folds the conversion to the arm's type.
    pub(super) fn widened_constant(&self, item: Item) -> bool {
        matches!(item.operand, Operand::Excess(value) if self.widened_constants.contains(&value))
    }

    /// Whether `ty` is `_Complex _Float16`.
    pub(super) fn complex_half(&self, ty: TypeId) -> bool {
        let types = &self.unit.sema.types;
        matches!(
            types.kind(types.non_atomic(ty)),
            TypeKind::Scalar(Scalar::ComplexFloat16)
        )
    }

    /// Whether `ty` keeps an excess: `_Float16`, `__bf16` or
    /// `_Complex _Float16`.
    pub(super) fn excess_type(&self, ty: TypeId) -> bool {
        self.half(ty).is_some() || self.complex_half(ty)
    }

    /// Whether GCC's rule keeps the excess beyond arithmetic operators.
    pub(super) fn half_in_float(&self) -> bool {
        self.unit.context.configuration.half_in_float()
    }

    fn single_constant(&self, value: Value) -> Option<f32> {
        let bits = self.draft.float_constant(value)?;
        Some(f32::from_bits(u32::try_from(bits).ok()?))
    }

    /// Whether GCC evaluates `expression` with excess precision
    /// ([`semantic_analysis::excess_expression`]).
    pub(super) fn excess_expression(&self, expression: &'tu Expression<'tu>) -> bool {
        semantic_analysis::excess_expression(expression, &|e| self.info(e).ok().copied())
    }

    /// Whether GCC folds `expression` to a constant as it parses it
    /// ([`semantic_analysis::folded_constant`]).
    pub(super) fn folded_constant(&self, expression: &'tu Expression<'tu>) -> bool {
        let types = &self.unit.sema.types;
        semantic_analysis::folded_constant(expression, &|e| self.info(e).ok().copied(), &|ty| {
            match types.kind(types.non_atomic(ty)) {
                | TypeKind::Scalar(scalar) => Some(scalar),
                | _ => None,
            }
        })
    }
}
