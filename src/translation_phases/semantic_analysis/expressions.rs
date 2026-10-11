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
//! - Retained categories and conversions: `results.rs`.
//! - Constant addresses and address differences: `address.rs`.
//!
//! C99: §6.3, pp. 42-48; PDF pp. 54-60; §6.5, pp. 67-94; PDF pp. 79-106
//! (translation phase 7, expression typing and conversions).
//! Statement control flow is checked by the statement analyzer; initializer
//! subobjects are walked by the initializer analyzer.

// Operator typing and conversions
mod typing;

// Retained categories and conversions
mod results;

// Constant addresses
mod address;

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
    },
    Analyzer,
    BindingKind,
    Duration,
    SemanticErrorKind,
    builtins::atomics,
    constants::Floating,
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
    translation_phases::parsing::syntax::{
        BinaryOperator,
        Constant,
        Expression,
        ExpressionType,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Postorder reduction never evaluates an operand through Rust recursion.
    /// C99: §6.5.1-§6.5.17, pp. 69-94; PDF pp. 81-106.
    pub(super) fn expression_done(&mut self, e: &'tu Expression<'tu>) {
        use ExpressionType as E;
        let mut info = Self::expression_result(e, self.types.unknown());
        match e.kind {
            | E::Identifier(name) => {
                if let Some(entry) = self.lookup(Namespace::Ordinary, name.name) {
                    let binding = self.bindings[entry.binding];
                    info.ty = binding.ty;
                    info.binding = Some(entry.binding);
                    self.record_binding_use(entry.binding, name);
                    info.register = self.register_bindings.contains_key(&entry.binding);
                    info.static_address = binding.duration == Duration::Static
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
                    | _ => {
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
                    });
                }
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Char(value)) => {
                let scalar = if matches!(value, CharacterTokenType::WideChar(_)) {
                    self.types.target.wchar_t
                } else {
                    Scalar::Int
                };
                info.ty = self.types.scalar(scalar);
                let mut value = i128::from(value.target_value(&self.types.target));
                if matches!(
                    e.kind,
                    E::Constant(Constant::Char(CharacterTokenType::Char(_)))
                ) && self.types.target.char_signed
                    && (128..=255).contains(&value)
                {
                    value -= 256;
                }
                let (bits, signed) = self
                    .integer_type(info.ty)
                    .expect("character type is integer");
                info.integer = Some(Integer {
                    value,
                    bits,
                    signed,
                });
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Boolean(value) => {
                info.ty = self.types.scalar(Scalar::Int);
                info.integer = Some(Integer::int(i128::from(value)));
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | E::Constant(Constant::Float(
                FloatTokenType::Float128(_) | FloatTokenType::ImaginaryFloat128(_),
            )) => {
                info.ty = self.types.scalar(
                    if matches!(
                        e.kind,
                        E::Constant(Constant::Float(FloatTokenType::ImaginaryFloat128(_)))
                    ) {
                        Scalar::ComplexFloat128
                    } else {
                        Scalar::Float128
                    },
                );
                info.unfolded_binary128 = true;
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
                    | _ => {
                        self.retain_expression(info);
                        return;
                    },
                });
                info.floating = Some(Floating::real(match value {
                    | FloatTokenType::Float(value) => LongDouble::from_double(f64::from(value)),
                    | FloatTokenType::Double(value) => LongDouble::from_double(value.get()),
                    | FloatTokenType::LongDouble(value) => value,
                    | _ => LongDouble::ZERO,
                }));
                info.floating = match value {
                    | FloatTokenType::ImaginaryFloat(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: LongDouble::from_double(f64::from(value)),
                    }),
                    | FloatTokenType::ImaginaryDouble(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: LongDouble::from_double(value.get()),
                    }),
                    | FloatTokenType::ImaginaryLongDouble(value) => Some(Floating {
                        real: LongDouble::ZERO,
                        imag: value,
                    }),
                    | _ => info.floating,
                };
                info.constant = ConstantClass::Arithmetic;
            },
            | E::StringLiteral(value) =>
                if let Some((element, count)) = self.string_type(value) {
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
                    .get(&name.source_vectors)
                    .copied()
                    .unwrap_or_else(|| self.types.unknown());
                info = self.type_sizeof(e, ty, None, matches!(e.kind, E::AlignofType(_)));
            },
            | E::SizeofExpr(operand) | E::AlignofExpr(operand) => {
                let operand = self.expression_info(operand);
                info = self.type_sizeof(
                    e,
                    operand.ty,
                    operand.bit_field,
                    matches!(e.kind, E::AlignofExpr(_)),
                );
            },
            | E::Cast {
                target_type,
                operand_expression,
            } => {
                let target = self
                    .resolved_type_names
                    .get(&target_type.source_vectors)
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
                let mut vector_callee = function_expression;
                while let ExpressionType::Parenthesized { expression } = vector_callee.kind {
                    vector_callee = expression;
                }
                info = if let ExpressionType::Identifier(name) = vector_callee.kind
                    && super::vectors::named_builtin(self.context.string_cache.at(name.name))
                {
                    self.context.report_extension(
                        crate::configuration::Feature::VectorBuiltins,
                        "__builtin_shufflevector",
                        name.source_vectors,
                    );
                    self.shuffle_vector_builtin(e, arguments)
                } else if let Some(info) = self.vector_overload(e, vector_callee, arguments) {
                    info
                } else if let Some(info) = self.atomic_call(e, function_expression, arguments) {
                    info
                } else if self.classify_type_callee(function_expression) {
                    self.classify_type(e, &arguments)
                } else if let Some(info) = self.math128_builtin(e, function_expression, &arguments)
                {
                    info
                } else if let Some(info) = self.constant_p(e, function_expression, arguments) {
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
                    .get(&type_name.source_vectors)
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
                    info.ty = self.check_initializer(ty, initializer, self.scope == 0);
                }
                info.category = self.object_category(info.ty);
                info.static_address = self.scope == 0;
            },
            // Unmodeled extensions retain unknown results and suppress constraints.
            | _ => {},
        }
        if info.unfolded_binary128 {
            info.integer = None;
            info.floating = None;
            info.ice = false;
        }
        if e.recovered {
            info.unfolded_binary128 = false;
            info.ty = self.types.unknown();
            info.integer = None;
            info.floating = None;
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
                .vector(self.expression_info(left_expression).ty)
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
            // C99 §6.6p10 permits additional constant forms. Preserve the
            // GNU-folded address difference through enclosing arithmetic;
            // evaluate reports it through the existing extension policy.
            let folded_address = match e.kind {
                | E::Unary {
                    operand_expression, ..
                } => self.expression_info(operand_expression).folded_address(),
                | E::Binary {
                    left_expression,
                    right_expression,
                    ..
                } =>
                    self.expression_info(left_expression).folded_address()
                        || self.expression_info(right_expression).folded_address(),
                | E::Conditional(c) | E::OmittedConditional(c) =>
                    self.expression_info(c.condition_expression)
                        .folded_address()
                        || self.expression_info(c.then_expression).folded_address()
                        || self.expression_info(c.else_expression).folded_address(),
                | _ => false,
            };
            if folded_address {
                info.folding = ConstantFolding::AddressDifference;
            }
        }
        self.retain_expression(info);
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
    pub(crate) expression:          &'tu Expression<'tu>,
    /// Selected operand identity for address decomposition.
    /// C11: §6.5.1.1p4, p. 79; PDF p. 97; GNU `choose_expr` follows it.
    pub(crate) selected_expression: Option<&'tu Expression<'tu>>,
    pub(crate) ty:                  TypeId,
    /// Arithmetic type before the final compound-assignment conversion.
    pub(crate) operation_type:      Option<TypeId>,
    pub(crate) category:            ValueCategory,
    pub(crate) binding:             Option<usize>,
    pub(crate) bit_field:           Option<u32>,
    pub(crate) register:            bool,
    /// GNU vector extension: a lane remains assignable but has no address.
    pub(crate) vector_element:      bool,
    /// Whether designation can form an address constant without reading an
    /// object.
    pub(crate) static_address:      bool,
    /// An arithmetic constant containing binary128, retained without
    /// approximate folding.
    pub(crate) unfolded_binary128:  bool,
    pub(crate) floating:            Option<Floating>,
    pub(crate) integer:             Option<Integer>,
    pub(crate) ice:                 bool,
    pub(crate) constant:            ConstantClass,
    pub(crate) folding:             ConstantFolding,
}
