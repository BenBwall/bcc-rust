//! Common GCC scalar intrinsics. Calls are checked after their operands on
//! the semantic work stack using neutral descriptor contracts.
//! GNU extension: GCC "Other Builtins", "Bit Operation Builtins",
//! "Byte-Swapping Builtins", "Integer Overflow Builtins", and "Stack
//! Allocation". <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Byte-Swapping-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Stack-Allocation.html>
//! C99: §6.5.2.2 paragraphs 2 and 7, pp. 71-72; PDF pp. 83-84.

use crate::{
    builtins::{
        BuiltinDescriptor,
        BuiltinOperation as CommonBuiltin,
        ConstantConstraint,
        ConversionRule,
        OverflowOperation,
    },
    translation_phases::{
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::{
                Expression,
                ExpressionType,
            },
        },
        semantic_analysis::{
            Analyzer,
            ConstantClass,
            ConversionKind,
            ExpressionInfo,
            Scalar,
            SemanticErrorKind,
            TypeId,
            TypeKind,
            integer::Integer,
        },
    },
    util::arena_list::ArenaList,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Validates overloaded operands without applying integer promotions to
    /// overflow inputs or the `overflow_p` destination. Constant queries never
    /// evaluate their operand; unknown object bounds use GCC's permitted
    /// answer. GNU extension: GCC manuals linked in this module's
    /// documentation.
    pub(in crate::translation_phases::semantic_analysis) fn common_builtin_call(
        &mut self,
        e: &'tu Expression<'tu>,
        callee: &'tu Expression<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> Option<ExpressionInfo<'tu>> {
        let callee = self.selected_designator(callee);
        let ExpressionType::Identifier(name) = callee.kind else {
            return None;
        };
        let binding = self.expression_info(callee).binding?;
        let descriptor = self.bindings[binding].origin.descriptor()?;
        let builtin = descriptor.operation;
        if builtin == CommonBuiltin::Library {
            return None;
        }
        if let Some(feature) = descriptor.availability.feature {
            self.context
                .report_extension(feature, descriptor.spelling, name.source_vectors);
        }
        let args = arguments.as_slice();
        let (min, max) = descriptor.arity;
        if !(min..=max).contains(&args.len()) {
            self.error(
                SemanticErrorKind::InvalidArgumentCount,
                e.source_vectors,
                None,
                None,
            );
            return Some(Self::expression_result(e, self.types.unknown()));
        }
        if descriptor.type_rule == crate::builtins::TypeRule::Varargs
            && let CommonBuiltin::Varargs(operation) = builtin
        {
            return Some(self.type_varargs(e, operation, args, None));
        }
        let intrinsic_type = self.descriptor_type(descriptor);
        let TypeKind::Function { result, .. } = self.types.nodes[intrinsic_type.index] else {
            return None;
        };
        let mut info = Self::expression_result(e, result);
        if descriptor.type_rule == crate::builtins::TypeRule::Fixed {
            for (index, &argument) in args.iter().enumerate() {
                self.check_registry_argument(descriptor, index, argument, name.name);
            }
        }
        match builtin {
            | CommonBuiltin::Float(operation, scalar) => {
                return Some(self.floating_builtin(e, args, operation, scalar));
            },
            | CommonBuiltin::Constant => {
                let operand = self.expression_info(args[0]);
                info.integer = Some(Integer::int(i128::from(
                    operand.ice && operand.integer.is_some(),
                )));
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | CommonBuiltin::ObjectSize(_) => {
                let ConversionRule::Parameter(pattern) = descriptor.arguments[1].conversion else {
                    return None;
                };
                let int = self.signature_type(pattern);
                let value = self
                    .common_integer_constant(args[1], int)
                    .and_then(Integer::to_u64);
                {
                    let (bits, signed) = self.integer_type(result)?;
                    info.integer = Some(
                        Integer::int(if value.is_some_and(|v| v >= 2) { 0 } else { -1 })
                            .cast(bits, signed),
                    );
                    info.ice = true;
                    info.constant = ConstantClass::Arithmetic;
                }
            },
            | CommonBuiltin::Overflow(operation, scalar, predicate) => {
                for &arg in &args[..2] {
                    if let Some(scalar) = scalar {
                        let ty = self.types.scalar(scalar);
                        _ = self.common_argument(arg, ty);
                    } else {
                        let operand = self.expression_info(arg);
                        let ty = self.converted(operand);
                        if self.integer_type(ty).is_none() && !self.types.unanalyzed(ty) {
                            self.error(
                                SemanticErrorKind::InvalidArgumentType,
                                arg.source_vectors,
                                None,
                                None,
                            );
                        }
                    }
                }
                let operand = self.expression_info(args[2]);
                let ty = self.converted(operand);
                let destination = if predicate {
                    Some(ty)
                } else {
                    match self.types.nodes[ty.index] {
                        | TypeKind::Pointer(target) => Some(target),
                        | _ => None,
                    }
                };
                let valid=destination.is_some_and(|ty| matches!(self.types.nodes[ty.index],TypeKind::Scalar(s) if s.integer() && s!=Scalar::Bool) && (predicate || !ty.qualifiers.contains(TypeQualifiers::CONST)) && scalar.is_none_or(|s|self.types.nodes[ty.index]==TypeKind::Scalar(s)));
                if !valid {
                    self.error(
                        SemanticErrorKind::InvalidArgumentType,
                        args[2].source_vectors,
                        None,
                        None,
                    );
                } else if let Some(destination) = destination {
                    info.operation_type = Some(destination);
                    if predicate {
                        let a = self.expression_info(args[0]);
                        let b = self.expression_info(args[1]);
                        if a.ice
                            && b.ice
                            && self.effect_pure(args[2], true)
                            && let Some((a, b)) = a.integer.zip(b.integer)
                        {
                            let (bits, signed) = self.integer_type(destination)?;
                            let bits = operand.bit_field.unwrap_or(bits);
                            info.integer = Some(Integer::int(i128::from(constant_overflow(
                                operation, a, b, bits, signed,
                            ))));
                            info.ice = true;
                            info.constant = ConstantClass::Arithmetic;
                        }
                    }
                }
            },
            | CommonBuiltin::Bit(..)
            | CommonBuiltin::Bswap(_)
            | CommonBuiltin::Expect(_)
            | CommonBuiltin::Aligned
            | CommonBuiltin::Prefetch
            | CommonBuiltin::Alloca
            | CommonBuiltin::Unreachable
            | CommonBuiltin::Trap
            | CommonBuiltin::Varargs(_)
            | CommonBuiltin::Library
            | CommonBuiltin::Offsetof => {},
        }
        Some(info)
    }

    /// Apply compatibility before the descriptor's converted/original constant
    /// constraint, retaining the existing diagnostic catalog and operand
    /// source. C99: §6.5.2.2p7, p. 72; PDF p. 84; §6.3.1.3p1-3, p. 43;
    /// PDF p. 55; §6.3.1.5p2, p. 44; PDF p. 56.
    fn check_registry_argument(
        &mut self,
        descriptor: &BuiltinDescriptor,
        index: usize,
        argument: &'tu Expression<'tu>,
        name: crate::util::string_cache::StringCacheId,
    ) {
        let rule = descriptor.arguments[index];
        let ConversionRule::Parameter(pattern) = rule.conversion else {
            return;
        };
        let ty = self.signature_type(pattern);
        let compatible = self.common_argument(argument, ty);
        if !compatible {
            return;
        }
        let info = self.expression_info(argument);
        let valid = match rule.constraint {
            | ConstantConstraint::None => true,
            | ConstantConstraint::ConvertedIntegerRange(max) => self
                .common_integer_constant(argument, ty)
                .and_then(Integer::to_u64)
                .is_some_and(|v| v <= max),
            | ConstantConstraint::OriginalIntegerRange(max) => info
                .integer
                .filter(|_| info.ice)
                .and_then(Integer::to_u64)
                .is_some_and(|v| v <= max),
            | ConstantConstraint::ConvertedPowerOfTwo => self
                .common_integer_constant(argument, ty)
                .and_then(Integer::to_u64)
                .is_some_and(u64::is_power_of_two),
            | ConstantConstraint::ConvertedProbability =>
                info.constant == ConstantClass::Arithmetic
                    && self
                        .floating_value(info)
                        .and_then(|value| self.round_floating(value, ty))
                        .is_some_and(|v| {
                            matches!(
                                v.real.compare(crate::float_parsing::LongDouble::ZERO),
                                0 | 1
                            ) && matches!(
                                v.real
                                    .compare(crate::float_parsing::LongDouble::from_double(1.0)),
                                -1 | 0
                            )
                        }),
        };
        if !valid {
            self.error(
                super::registry::diagnostic(rule.diagnostic),
                argument.source_vectors,
                Some(name),
                None,
            );
        }
    }

    /// Reports incompatible types before checking dependent value constraints.
    /// C99: §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84.
    fn common_argument(&mut self, e: &'tu Expression<'tu>, ty: TypeId) -> bool {
        let info = self.expression_info(e);
        let compatible = !self.types.unanalyzed(info.ty) && self.assignment_compatible(ty, info);
        if !self.types.unanalyzed(info.ty) && !compatible {
            self.error(
                SemanticErrorKind::InvalidArgumentType,
                e.source_vectors,
                None,
                None,
            );
        }
        _ = self.converted(info);
        self.convert(e, ty, ConversionKind::Assignment);
        compatible
    }

    /// Checks fixed-parameter immediates after the target's integer conversion.
    /// Signed narrowing uses the implementation's truncation/sign extension.
    /// C99: §6.5.2.2p7, p. 72; PDF p. 84; §6.3.1.3p1-3, p. 43;
    /// PDF p. 55. GNU extension: GCC manuals linked above.
    fn common_integer_constant(&self, e: &'tu Expression<'tu>, ty: TypeId) -> Option<Integer> {
        let info = self.expression_info(e);
        let value = info.integer.filter(|_| info.ice)?;
        let (bits, signed) = self.integer_type(ty)?;
        Some(value.cast(bits, signed))
    }
}

/// Infinite precision checking needs only a magnitude and a carry: a result
/// beyond 128 magnitude bits cannot fit any supported C integer destination.
/// GNU extension: GCC "Integer Overflow Builtins".
fn constant_overflow(
    op: OverflowOperation,
    a: Integer,
    b: Integer,
    bits: u32,
    signed: bool,
) -> bool {
    let magnitude = |v: Integer| {
        let negative = v.signed && v.to_i128().is_some_and(|v| v < 0);
        let value = if negative {
            v.to_i128().unwrap().unsigned_abs()
        } else {
            u128::from_le_bytes(
                (v.order_key() ^ if v.signed { 0 } else { i128::MIN }).to_le_bytes(),
            ) & if v.bits == 128 {
                u128::MAX
            } else {
                (1_u128 << v.bits) - 1
            }
        };
        (negative, value)
    };
    let (an, av) = magnitude(a);
    let (mut bn, bv) = magnitude(b);
    if op == OverflowOperation::Sub {
        bn = !bn;
    }
    let (negative, value, carry) = if op == OverflowOperation::Mul {
        let (v, c) = av.overflowing_mul(bv);
        (an ^ bn, v, c)
    } else if an == bn {
        let (v, c) = av.overflowing_add(bv);
        (an, v, c)
    } else if av >= bv {
        (an, av - bv, false)
    } else {
        (bn, bv - av, false)
    };
    let limit = if signed {
        (1_u128 << (bits - 1)) - u128::from(!negative)
    } else if bits == 128 {
        u128::MAX
    } else {
        (1_u128 << bits) - 1
    };
    carry || value > limit || (!signed && negative && value != 0)
}
