//! Expression typing reduces immutable syntax after its children have been
//! visited. [`Analyzer::expression_done`] checks the operator, chooses its type
//! and value category, and retains the result. Contextual conversions are
//! recorded separately so the original category remains available to later
//! phases.
//!
//! For `1 + 2L`, child visits first retain an `int` literal and a `long`
//! literal. The addition then applies the usual arithmetic conversions, records
//! the operand conversions and retains a `long` result. No child is evaluated
//! by a recursive Rust call.
//!
//! Read [`Analyzer::expression_done`] and [`ExpressionInfo`] first, then
//! [`Analyzer::type_binary`] and [`Analyzer::converted`] for operator typing
//! and value conversion. [`Conversion`] records a conversion at its syntax
//! site.
//!
//! - Operator typing and conversions: `typing.rs`.
//! - Pointer constraint classification: `pointers.rs`.
//! - Retained categories and conversions: `results.rs`.
//! - Constant addresses and address differences: `address.rs`.
//!
//! C99: §6.3, pp. 42-48; PDF pp. 54-60; §6.5, pp. 67-94; PDF pp. 79-106
//! (translation phase 7, expression typing and conversions).
//! Statement control flow is checked by the statement analyzer; initializer
//! subobjects are walked by the initializer analyzer.

// Operator typing and conversions
mod pointers;
mod typing;

// Retained categories and conversions
mod results;

// Constant addresses
mod address;

use std::num::NonZeroU8;

pub(crate) use address::AddressBase;
pub(crate) use results::{
    ConstantClass,
    ConstantFolding,
    Conversion,
    ConversionKind,
    ValueCategory,
};

use super::{
    super::preprocessing::{
        CharacterTokenType,
        FloatTokenType,
        IntegerTokenType,
        LiteralEncoding,
    },
    Analyzer,
    BindingKind,
    Duration,
    SemanticErrorKind,
    constants::{
        Binary128Value,
        Floating,
    },
    integer::Integer,
    scopes::Namespace,
    types::{
        ArrayBound,
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    float_parsing::LongDouble,
    half_precision::HalfFormat,
    translation_phases::parsing::syntax::{
        BinaryOperator,
        Constant,
        Expression,
        ExpressionType,
        UnaryOperator,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Postorder reduction never evaluates an operand through Rust recursion.
    /// C99: §6.5.1-§6.5.17, pp. 69-94; PDF pp. 81-106.
    pub(super) fn expression_done(&mut self, e: &'tu Expression<'tu>) {
        use ExpressionType as E;
        self.retain_expression_actions(e);
        let mut info = Self::expression_result(e, self.types.unknown());
        match e.kind {
            | E::Identifier(name) => {
                let found = self
                    .lookup(Namespace::Ordinary, name.name)
                    .map(|entry| entry.binding)
                    .or_else(|| self.predefined_identifier(name));
                if let Some(index) = found {
                    let binding = self.bindings[index];
                    info.ty = binding.ty;
                    if let Some(&alignment) = self.binding_alignments.get(&index) {
                        info.ty = info.ty.with_alignment(alignment);
                        info.place_alignment = self
                            .types
                            .alignment(binding.ty)
                            .and_then(|a| u8::try_from(a.trailing_zeros() + 1).ok())
                            .and_then(NonZeroU8::new);
                    }
                    info.binding = Some(index);
                    if self.function_extensions.function(index).is_none()
                        && !self.overload_previous.contains_key(&index)
                    {
                        self.record_binding_use(index, name);
                    }
                    info.register = self.register_bindings.contains_key(&index);
                    // The loader writes an imported object's address at
                    // run time, so it is not an address constant; an
                    // imported function's import thunk is (Clang).
                    // C99: §6.6p9, p. 96; PDF p. 108.
                    info.static_address = (binding.duration == Duration::Static
                        && self.dll_storage(index) != Some(super::DllStorage::Import))
                        || binding.kind == BindingKind::Function;
                    info.category = match binding.kind {
                        | BindingKind::Function => ValueCategory::FunctionDesignator,
                        | BindingKind::Enumerator => ValueCategory::Rvalue,
                        | BindingKind::Typedef => {
                            info.ty = self.types.unknown();
                            ValueCategory::Rvalue
                        },
                        | _ => self.object_category(info.ty),
                    };
                    info.ice = binding.kind == BindingKind::Enumerator;
                    if info.ice {
                        info.integer = binding.value;
                        info.constant = ConstantClass::Arithmetic;
                    } else if binding.kind == BindingKind::Object
                        && binding.value.is_some()
                        && self.context.configuration.gnu_extensions()
                    {
                        // Not a C99 constant expression (§6.6p6-8); GCC folds
                        // it, as §6.6p10 allows.
                        info.integer = binding.value;
                        info.constant = ConstantClass::Arithmetic;
                    }
                } else if !e.recovered && !self.builtin_name(name) {
                    self.error(
                        SemanticErrorKind::UndeclaredIdentifier,
                        name.source_vectors,
                        Some(name.name),
                        None,
                    );
                }
            },
            | E::Constant(Constant::Integer(value)) => {
                let scalar = match value {
                    | IntegerTokenType::Int(_) => Scalar::Int,
                    | IntegerTokenType::UnsignedInt(_) => Scalar::UnsignedInt,
                    | IntegerTokenType::Long(_) => Scalar::Long,
                    | IntegerTokenType::UnsignedLong(_) => Scalar::UnsignedLong,
                    | IntegerTokenType::LongLong(_) => Scalar::LongLong,
                    | IntegerTokenType::UnsignedLongLong(_) => Scalar::UnsignedLongLong,
                    // C23 §6.4.4.2p6: a `wb` constant has the narrowest
                    // bit-precise type that holds its value (and a sign bit).
                    | IntegerTokenType::BitInt(_, width, unsigned) => {
                        info.ty = self.types.intern(TypeKind::BitInt {
                            width:  u32::from(width),
                            signed: !unsigned,
                        });
                        info.integer = Some(Integer {
                            value:  value.into(),
                            bits:   u32::from(width),
                            signed: !unsigned,
                            wide:   None,
                        });
                        info.ice = true;
                        info.constant = ConstantClass::Arithmetic;
                        self.retain_expression(info);
                        return;
                    },
                    | IntegerTokenType::Imaginary(magnitude, kind) => {
                        self.imaginary_integer(&mut info, magnitude.get(), kind);
                        self.retain_expression(info);
                        return;
                    },
                    | IntegerTokenType::WideBitInt(spelling, width, unsigned) => {
                        info.ty = self.types.intern(TypeKind::BitInt {
                            width,
                            signed: !unsigned,
                        });
                        let spelling = self.context.string_cache.at(spelling);
                        info.integer = Some(self.wide.literal(spelling, width, !unsigned));
                        info.ice = true;
                        info.constant = ConstantClass::Arithmetic;
                        self.retain_expression(info);
                        return;
                    },
                };
                info.ty = self.types.scalar(scalar);
                if let Some((bits, signed)) = self.integer_type(info.ty) {
                    info.integer = Some(Integer {
                        value: value.into(),
                        bits,
                        signed,
                        wide: None,
                    });
                }
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Char(value)) => {
                // C11 §6.4.4.4p11: `u` and `U` constants have `char16_t` and
                // `char32_t` type; C23 §6.4.4.5p12: `u8` (a C23 construct in
                // every mode) has `char8_t`.
                let scalar = match value {
                    | CharacterTokenType::WideChar(_) => self.types.target.wchar_t,
                    | CharacterTokenType::EncodedChar(_, LiteralEncoding::Utf8) =>
                        Scalar::UnsignedChar,
                    | CharacterTokenType::EncodedChar(_, encoding) =>
                        self.encoded_character_type(encoding),
                    | _ => Scalar::Int,
                };
                info.ty = self.types.scalar(scalar);
                let value = i128::from(value.target_value(&self.types.target));
                let (bits, signed) = self
                    .integer_type(info.ty)
                    .expect("character type is integer");
                info.integer = Some(Integer {
                    value,
                    bits,
                    signed,
                    wide: None,
                });
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Nullptr => {
                info.ty = self.types.intern(TypeKind::Nullptr);
                info.integer = Some(Integer::int(0).cast(64, false));
                info.constant = ConstantClass::Address;
            },
            | E::Boolean(value) => {
                info.ty = self.types.scalar(Scalar::Int);
                info.integer = Some(Integer::int(i128::from(value)));
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Float(FloatTokenType::Float128(value))) => {
                info.ty = self.types.scalar(Scalar::Float128);
                info.set_binary128(Some(Binary128Value::real(value)));
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Float(FloatTokenType::LongDouble128(value))) => {
                info.ty = self.types.scalar(Scalar::LongDouble);
                info.set_binary128(Some(Binary128Value::real(value)));
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Float(FloatTokenType::ImaginaryLongDouble128(value))) => {
                info.ty = self.types.scalar(Scalar::ComplexLongDouble);
                info.set_binary128(Some(Binary128Value {
                    real: crate::binary128::Binary128::zero(false),
                    imag: value,
                }));
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Float(FloatTokenType::ImaginaryFloat128(value))) => {
                info.ty = self.types.scalar(Scalar::ComplexFloat128);
                info.set_binary128(Some(Binary128Value {
                    real: crate::binary128::Binary128::zero(false),
                    imag: value,
                }));
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Float(value)) => {
                info.ty = self.types.scalar(match value {
                    | FloatTokenType::Float(_) => Scalar::Float,
                    | FloatTokenType::Double(_) => Scalar::Double,
                    | FloatTokenType::LongDouble(_) => Scalar::LongDouble,
                    | FloatTokenType::ImaginaryFloat(_) => Scalar::ComplexFloat,
                    | FloatTokenType::ImaginaryDouble(_) => Scalar::ComplexDouble,
                    | FloatTokenType::ImaginaryLongDouble(_) => Scalar::ComplexLongDouble,
                    | FloatTokenType::Float32(_) => Scalar::Float32,
                    | FloatTokenType::Float64(_) => Scalar::Float64,
                    | FloatTokenType::Float32x(_) => Scalar::Float32x,
                    | FloatTokenType::Float64x(_) => Scalar::Float64x,
                    | FloatTokenType::Float16(_) => Scalar::Float16,
                    | FloatTokenType::BFloat16(_) => Scalar::BFloat16,
                    | FloatTokenType::ImaginaryFloat16(_) => Scalar::Float16.complex_type(),
                    | FloatTokenType::ImaginaryFloat32(_) => Scalar::ComplexFloat32,
                    | FloatTokenType::ImaginaryFloat64(_) => Scalar::ComplexFloat64,
                    | FloatTokenType::ImaginaryFloat32x(_) => Scalar::ComplexFloat32x,
                    | FloatTokenType::ImaginaryFloat64x(_) => Scalar::ComplexFloat64x,
                    | _ => {
                        self.retain_expression(info);
                        return;
                    },
                });
                info.set_floating(Some(Floating::real(match value {
                    | FloatTokenType::Float(value) | FloatTokenType::Float32(value) =>
                        LongDouble::from_double(f64::from(value)),
                    | FloatTokenType::Double(value)
                    | FloatTokenType::Float64(value)
                    | FloatTokenType::Float32x(value) => LongDouble::from_double(value.get()),
                    | FloatTokenType::LongDouble(value) | FloatTokenType::Float64x(value) => value,
                    | FloatTokenType::Float16(value) => LongDouble::from_double(f64::from(
                        self.half_constant(value, HalfFormat::Binary16),
                    )),
                    | FloatTokenType::BFloat16(value) => LongDouble::from_double(f64::from(
                        self.half_constant(value, HalfFormat::BFloat16),
                    )),
                    | _ => LongDouble::ZERO,
                })));
                info.set_floating(match value {
                    | FloatTokenType::ImaginaryFloat(value)
                    | FloatTokenType::ImaginaryFloat32(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: LongDouble::from_double(f64::from(value)),
                    }),
                    | FloatTokenType::ImaginaryDouble(value)
                    | FloatTokenType::ImaginaryFloat64(value)
                    | FloatTokenType::ImaginaryFloat32x(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: LongDouble::from_double(value.get()),
                    }),
                    | FloatTokenType::ImaginaryLongDouble(value)
                    | FloatTokenType::ImaginaryFloat64x(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: value,
                    }),
                    // The imaginary part has the value a real `f16`
                    // constant has, excess precision included.
                    | FloatTokenType::ImaginaryFloat16(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: LongDouble::from_double(f64::from(
                            self.half_constant(value, HalfFormat::Binary16),
                        )),
                    }),
                    | _ => info.floating(),
                });
                info.constant = ConstantClass::Arithmetic;
            },
            | E::StringLiteral(value) => {
                let (element, count) = self.string_type(value);
                info.ty = self
                    .types
                    .intern(TypeKind::Array(element, ArrayBound::Constant(count)));
                info.category = ValueCategory::Lvalue;
                info.static_address = true;
            },
            | E::Parenthesized { expression } => {
                info = ExpressionInfo {
                    expression: e,
                    ..self.expression_info(expression)
                };
                if let Some(labels) = self.control.difference_labels(expression) {
                    self.control.record_difference(e, labels);
                }
            },
            | E::LabelAddress(_) => {
                // GNU extension: GCC "Labels as Values". `&&label` is a
                // `void *` address constant of the enclosing function.
                let void = self.types.scalar(Scalar::Void);
                info.ty = self.types.intern(TypeKind::Pointer(void));
                info.constant = ConstantClass::Address;
            },
            | E::Generic(generic) => info = self.type_generic(e, generic),
            | E::Unary {
                operator,
                operand_expression,
            } => {
                info = self.type_unary(e, operator, self.expression_info(operand_expression));
            },
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                info = self.type_binary(
                    e,
                    operator,
                    self.expression_info(left_expression),
                    self.expression_info(right_expression),
                );
            },
            | E::Conditional(c) | E::OmittedConditional(c) => {
                info = self.type_conditional(e, c);
            },
            | E::SizeofType(name) | E::AlignofType(name) => {
                let ty = self
                    .resolved_type_names
                    .get(&super::type_name_identity(name))
                    .copied()
                    .unwrap_or_else(|| self.types.unknown());
                info = self.type_sizeof(e, ty, None, matches!(e.kind, E::AlignofType(_)));
                if matches!(e.kind, E::SizeofType(_))
                    && let Some(site) = self.type_name_sites.get(&super::type_name_identity(name))
                {
                    // C99 6.5.3.4p2: constant sizeof does not execute its
                    // type operand, even when a pointer is VM through its
                    // pointee. The declaration/cast site still retains
                    // typeof's operand evaluation independently.
                    _ = self.size_query_sites.insert(
                        std::ptr::from_ref(e).addr(),
                        super::SizeQueryFacts {
                            formula: None,
                            operand: None,
                            site:    super::BoundSiteFacts {
                                bounds: if info.ice { &[] } else { site.bounds },
                                ty,
                                operand_evaluations: if info.ice {
                                    &[]
                                } else {
                                    site.operand_evaluations
                                },
                            },
                        },
                    );
                }
            },
            | E::SizeofExpr(operand) | E::AlignofExpr(operand) => {
                let operand_syntax = operand;
                let operand = self.expression_info(operand);
                info = self.type_sizeof(
                    e,
                    if matches!(e.kind, E::AlignofExpr(_)) {
                        operand
                            .place_alignment()
                            .map_or(operand.ty, |alignment| operand.ty.with_alignment(alignment))
                    } else {
                        operand.ty
                    },
                    operand.bit_field,
                    matches!(e.kind, E::AlignofExpr(_)),
                );
                if matches!(e.kind, E::SizeofExpr(_)) {
                    _ = self.size_query_sites.insert(
                        std::ptr::from_ref(e).addr(),
                        super::SizeQueryFacts {
                            formula: None,
                            site:    super::BoundSiteFacts {
                                ty:                  operand.ty,
                                bounds:              &[],
                                operand_evaluations: &[],
                            },
                            operand: (!info.ice).then_some(operand_syntax),
                        },
                    );
                }
            },
            | E::Cast {
                target_type,
                operand_expression,
            } => {
                let target = self
                    .resolved_type_names
                    .get(&super::type_name_identity(target_type))
                    .copied()
                    .unwrap_or_else(|| self.types.unknown());
                info = self.type_cast(e, target, self.expression_info(operand_expression));
            },
            | E::DirectMember {
                base_expression,
                member,
            }
            | E::IndirectMember {
                base_expression,
                member,
            } => {
                info = self.type_member(
                    e,
                    self.expression_info(base_expression),
                    member,
                    matches!(e.kind, E::IndirectMember { .. }),
                );
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                // C99 §6.5.1p5: parentheses preserve the callee designation.
                // GNU vector builtins retain constraints through parentheses.
                let vector_callee = self.selected_designator(function_expression);
                let intrinsic = self
                    .expression_info(vector_callee)
                    .binding
                    .is_some_and(|id| self.bindings[id].origin.is_intrinsic());
                info = if intrinsic
                    && let Some(info) = self.common_builtin_call(e, function_expression, arguments)
                {
                    info
                } else {
                    self.type_call(e, self.expression_info(function_expression), arguments)
                };
            },
            | E::Builtin(b) => {
                info = self.type_builtin(e, b);
            },
            | E::CompoundLiteral {
                type_name,
                initializer,
            } => {
                let ty = self
                    .resolved_type_names
                    .get(&super::type_name_identity(type_name))
                    .copied()
                    .unwrap_or_else(|| self.types.unknown());
                // §6.5.2.5p1 excludes variable length array types; a pointer
                // to one is a valid compound literal type.
                if !self.types.unanalyzed(ty)
                    && ((matches!(self.types.nodes[ty.index], TypeKind::Array(..))
                        && self.variably_modified(ty))
                        || (!self.complete_object(ty)
                            && !matches!(
                                self.types.nodes[ty.index],
                                TypeKind::Array(_, ArrayBound::Incomplete)
                            )))
                {
                    info = self.invalid_expression(e, SemanticErrorKind::InvalidCompoundLiteral);
                } else {
                    // A compound literal's flexible array member stays empty.
                    info.ty = self
                        .check_initializer(ty, initializer, self.scope == 0, false, None)
                        .0;
                    self.fold_compound_literal(&mut info, initializer);
                }
                info.category = self.object_category(info.ty);
                info.static_address = self.scope == 0;
            },
            | E::StatementExpression(statement) => {
                if let Some((_, tail)) = self.statement_expression_tail(statement) {
                    // GNU extension: GCC "Statement Exprs" returns the value
                    // of the final expression statement. Its ValueExpression
                    // work already recorded C99 §6.3.2.1p2-4 conversions
                    // (p. 46; PDF p. 58), even under an enclosing sizeof.
                    // Reuse their type without recording them twice. The
                    // result is a value, without the tail's lvalue/bit-field
                    // identity; block effects prevent constant folding.
                    // <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
                    let tail = self.expression_info(tail);
                    let (ty, _) = self.value_conversion(tail);
                    info = Self::expression_result(e, ty);
                    info.unfolded_binary128 = tail.unfolded_binary128;
                } else {
                    info = Self::expression_result(e, self.types.scalar(Scalar::Void));
                }
            },
            // Unmodeled extensions retain unknown results and suppress
            // constraints.
            | _ => {},
        }
        if info.unfolded_binary128 {
            info.integer = None;
            info.set_floating(None);
            info.set_binary128(None);
            info.ice = false;
        }
        if e.recovered {
            info.unfolded_binary128 = false;
            info.ty = self.types.unknown();
            info.integer = None;
            info.set_floating(None);
            info.set_binary128(None);
            info.ice = false;
            info.constant = ConstantClass::None;
        }
        // GNU vector lane designation survives parentheses and selections.
        // The address-of constraint below extends C99 §6.5.3.2p1.
        if let E::Binary {
            operator: BinaryOperator::Subscript,
            left_expression,
            ..
        } = e.kind
        {
            info.vector_element = self
                .vector(
                    self.types
                        .non_atomic(self.expression_info(left_expression).ty),
                )
                .is_some();
        }
        // Size/alignment and generic controlling operands are unevaluated.
        // Generic selection already copied only its selected expression.
        let atomic_cast = match e.kind {
            | E::Unary {
                operand_expression, ..
            } => self.expression_info(operand_expression).atomic_cast(),
            | E::Binary {
                left_expression,
                right_expression,
                ..
            } =>
                self.expression_info(left_expression).atomic_cast()
                    || self.expression_info(right_expression).atomic_cast(),
            | E::Conditional(c) | E::OmittedConditional(c) =>
                self.expression_info(c.condition_expression).atomic_cast()
                    || self.expression_info(c.then_expression).atomic_cast()
                    || self.expression_info(c.else_expression).atomic_cast(),
            | E::Cast {
                operand_expression, ..
            } =>
                matches!(self.types.nodes[info.ty.index], TypeKind::Atomic(_))
                    || self.expression_info(operand_expression).atomic_cast(),
            | _ => false,
        };
        if info.atomic_cast() || atomic_cast {
            info.folding = ConstantFolding::AtomicCast;
            info.ice = false;
        } else {
            self.operand_folding(e, &mut info);
        }
        if let E::StatementExpression(block) = e.kind {
            self.close_block_effects();
            self.retain_statement_expression(e, block, info.ty);
        }
        self.retain_expression(info);
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Carries into `info`, the result of `e`, the folding its operands
    /// had.
    /// C99: §6.6 paragraph 10, p. 96; PDF p. 108.
    fn operand_folding(&self, e: &'tu Expression<'tu>, info: &mut ExpressionInfo<'tu>) {
        use ExpressionType as E;
        // C99 §6.6p10 permits additional constant forms. Preserve the
        // GNU-folded address difference and compound literal value
        // through enclosing arithmetic, a literal's read over an
        // address's; evaluate reports them through the existing
        // extension policy.
        let operands = match e.kind {
            | E::Cast {
                operand_expression, ..
            }
            | E::Unary {
                operand_expression, ..
            } => [Some(operand_expression), None, None],
            | E::Binary {
                left_expression,
                right_expression,
                ..
            } => [Some(left_expression), Some(right_expression), None],
            | E::Conditional(c) | E::OmittedConditional(c) => [
                Some(c.condition_expression),
                Some(c.then_expression),
                Some(c.else_expression),
            ],
            | _ => [None; 3],
        };
        let operands = operands.map(|operand| operand.map(|o| self.expression_info(o)));
        // GCC's overflow mark follows arithmetic and the operand `?:`
        // selects, not a comparison, a logical operator or a conversion to
        // `_Bool`, whose result is a fresh constant, though still no integer
        // constant expression.
        let marked = |operand: Option<ExpressionInfo<'tu>>| {
            operand.is_some_and(|o| o.folding == ConstantFolding::Overflow && o.integer.is_some())
        };
        let carried = match e.kind {
            | E::Binary { operator, .. } =>
                matches!(
                    operator,
                    BinaryOperator::Multiplication
                        | BinaryOperator::Division
                        | BinaryOperator::Modulo
                        | BinaryOperator::Addition
                        | BinaryOperator::Subtraction
                        | BinaryOperator::LeftShift
                        | BinaryOperator::RightShift
                        | BinaryOperator::BitwiseAnd
                        | BinaryOperator::BitwiseXor
                        | BinaryOperator::BitwiseOr
                ) && (marked(operands[0]) || marked(operands[1])),
            | E::Unary { operator, .. } =>
                operator != UnaryOperator::LogicalNot && marked(operands[0]),
            | E::Cast { .. } =>
                !matches!(
                    self.types.nodes[self.types.non_atomic(info.ty).index],
                    TypeKind::Scalar(Scalar::Bool)
                ) && marked(operands[0]),
            | E::Conditional(_) | E::OmittedConditional(_) => {
                let truth = operands[0].and_then(|o| o.integer).map(|v| !v.is_zero());
                match (truth, e.kind) {
                    | (Some(true), E::OmittedConditional(_)) => marked(operands[0]),
                    | (Some(true), _) => marked(operands[1]),
                    | (Some(false), _) => marked(operands[2]),
                    | (None, _) => false,
                }
            },
            | _ => false,
        };
        if info.integer.is_some()
            && operands
                .iter()
                .flatten()
                .any(|o| o.folding.overflowed() && o.integer.is_some())
        {
            info.folding = if carried {
                ConstantFolding::Overflow
            } else {
                ConstantFolding::OverflowOperand
            };
        } else if operands.iter().flatten().any(|o| o.folded_literal()) {
            info.folding = ConstantFolding::CompoundLiteral;
        } else if operands.iter().flatten().any(|o| o.folded_address()) {
            info.folding = ConstantFolding::Address;
        }
    }
}

/// Retained result keyed by immutable expression identity. The sequence is
/// deterministic postorder; identities are never printed as host addresses.
/// C99: §6.5p1, p. 67; PDF p. 79; §6.3.2.1, pp. 46-47; PDF pp. 58-59.
#[derive(Debug, Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent retained facts describe category, address eligibility and constant \
              evaluation; none is a state transition."
)]
pub(crate) struct ExpressionInfo<'tu> {
    pub(crate) expression: &'tu Expression<'tu>,
    /// Selected operand identity for address decomposition.
    /// C11: §6.5.1.1p4, p. 79; PDF p. 97; GNU `choose_expr` follows it.
    /// Constant address conditionals: C99 §6.5.15p4, p. 90; PDF p. 102.
    pub(crate) selected_expression: Option<&'tu Expression<'tu>>,
    /// The whole operand is skipped by Clang code generation.
    pub(crate) elided: bool,
    /// Function code computes a different value than `integer` and
    /// `floating`, which hold Clang's constant-context value: the operand
    /// holds a `__builtin_constant_p` whose emitted value is one where a
    /// constant context reads zero. Lowering evaluates it structurally.
    /// GNU extension: GCC manual, "Other Builtins".
    pub(crate) runtime_query: bool,
    pub(crate) ty: TypeId,
    /// Arithmetic type before the final compound-assignment conversion.
    pub(crate) operation_type: Option<TypeId>,
    /// Alignment guaranteed by the containing place and resolved member offset.
    /// It survives independently of contextual destination-type conversions.
    /// C99: §6.7.2.1p12, p. 103; PDF p. 115; GNU packed extension.
    /// Stored as log2(bytes) + 1, so the optional fact uses one byte.
    pub(in crate::translation_phases::semantic_analysis) place_alignment: Option<NonZeroU8>,
    pub(crate) category: ValueCategory,
    pub(crate) binding: Option<usize>,
    pub(crate) bit_field: Option<u32>,
    /// Width used by integer promotions, independent of bit-field designation.
    /// GCC/Clang preserve it through comma, assignment and prefix updates;
    /// casts and postfix updates produce ordinary values of the declared type.
    /// C99: §6.3.1.1p2, pp. 42-43; PDF pp. 54-55.
    /// Checked integer bit-fields have at most 128 bits; one byte fits the
    /// retained record's existing padding.
    pub(in crate::translation_phases::semantic_analysis) promotion_width: Option<NonZeroU8>,
    pub(crate) register: bool,
    /// GNU vector extension: a lane remains assignable but has no address.
    pub(crate) vector_element: bool,
    /// A GCC vector comparison's mask, which GCC types as an opaque vector
    /// that converts to any vector of its size.
    pub(crate) opaque_vector: bool,
    /// Whether designation can form an address constant without reading an
    /// object.
    pub(crate) static_address: bool,
    /// An arithmetic constant containing a binary128 operation that is not
    /// folded (the product or quotient of two complex values), retained
    /// without approximate folding.
    pub(crate) unfolded_binary128: bool,
    /// The folded value of a floating constant: a standard floating value
    /// in the host carrier, or a binary128 value computed with IEEE
    /// binary128 arithmetic. A constant has one floating type, so one slot
    /// holds either and the record keeps its size; read it through
    /// [`Self::floating`] and [`Self::binary128`].
    pub(in crate::translation_phases::semantic_analysis) folded: Option<FoldedFloating>,
    pub(crate) integer: Option<Integer>,
    pub(crate) ice: bool,
    pub(crate) constant: ConstantClass,
    pub(crate) folding: ConstantFolding,
}

impl ExpressionInfo<'_> {
    /// The folded value of a constant of standard floating type.
    pub(crate) fn floating(&self) -> Option<Floating> {
        match self.folded {
            | Some(FoldedFloating::Standard(value)) => Some(value),
            | _ => None,
        }
    }

    /// The folded value of a constant of binary128 type.
    pub(crate) fn binary128(&self) -> Option<Binary128Value> {
        match self.folded {
            | Some(FoldedFloating::Binary128(bytes)) => Some(Binary128Value::from_bytes(bytes)),
            | _ => None,
        }
    }

    /// Records a standard floating value; `None` clears only such a value.
    pub(crate) fn set_floating(&mut self, value: Option<Floating>) {
        match value {
            | Some(value) => self.folded = Some(FoldedFloating::Standard(value)),
            | None if self.floating().is_some() => self.folded = None,
            | None => {},
        }
    }

    /// Records a binary128 value; `None` clears only such a value.
    pub(crate) fn set_binary128(&mut self, value: Option<Binary128Value>) {
        match value {
            | Some(value) => self.folded = Some(FoldedFloating::Binary128(value.to_bytes())),
            | None if self.binary128().is_some() => self.folded = None,
            | None => {},
        }
    }
}

/// The slot behind [`ExpressionInfo::floating`] and
/// [`ExpressionInfo::binary128`]. A binary128 value is kept as bytes, so the
/// slot has byte alignment and the size of the standard value alone.
#[derive(Debug, Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) enum FoldedFloating {
    Standard(Floating),
    Binary128([u8; 32]),
}

impl Analyzer<'_, '_, '_> {
    /// GNU: an integer imaginary constant has the complex type of its real
    /// constant's type, held exactly in the floating carrier as complex
    /// constants are. GNU extension: GCC manual, "Complex Numbers".
    fn imaginary_integer(
        &mut self,
        info: &mut ExpressionInfo<'_>,
        magnitude: u64,
        kind: crate::translation_phases::preprocessing::ImaginaryIntegerKind,
    ) {
        use crate::translation_phases::preprocessing::ImaginaryIntegerKind as K;
        let real = match kind {
            | K::Int => Scalar::Int,
            | K::UnsignedInt => Scalar::UnsignedInt,
            | K::Long => Scalar::Long,
            | K::UnsignedLong => Scalar::UnsignedLong,
            | K::LongLong => Scalar::LongLong,
            | K::UnsignedLongLong => Scalar::UnsignedLongLong,
        };
        info.ty = self.types.scalar(real.complex_type());
        if let Some(imag) =
            self.integer_value(Integer::int(i128::from(magnitude)), crate::ir::Type::F80)
        {
            info.set_floating(Some(Floating {
                real: LongDouble::ZERO,
                imag,
            }));
            info.constant = ConstantClass::Arithmetic;
        }
    }
}
