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
                BinaryOperator,
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
        let function = callee;
        let callee = self.selected_designator(callee);
        let ExpressionType::Identifier(name) = callee.kind else {
            return None;
        };
        let binding = self.expression_info(callee).binding?;
        let descriptor = self.bindings[binding].origin.descriptor()?;
        let builtin = descriptor.operation;
        match descriptor.type_rule {
            | crate::builtins::TypeRule::Atomic => {
                let CommonBuiltin::Atomic(atomic) = builtin else {
                    return None;
                };
                return Some(self.atomic_call(e, atomic, arguments));
            },
            // A prototyped call whose immediates `type_call` checks.
            | crate::builtins::TypeRule::X86Signature => return None,
            | crate::builtins::TypeRule::VectorShuffle =>
                return Some(self.shuffle_vector_builtin(e, arguments)),
            | crate::builtins::TypeRule::GnuShuffle =>
                return Some(self.gnu_shuffle_builtin(e, arguments)),
            | crate::builtins::TypeRule::VectorOverload(operation) =>
                return Some(self.vector_overload(e, name, operation, arguments)),
            | crate::builtins::TypeRule::TypeClassify =>
                return Some(self.classify_type(e, &arguments)),
            | crate::builtins::TypeRule::Math128(rule) =>
                return Some(self.math128_builtin(e, rule, &arguments)),
            | crate::builtins::TypeRule::GenericBit => {
                let CommonBuiltin::GenericBit(operation) = builtin else {
                    return None;
                };
                return Some(self.generic_bit_builtin(
                    e,
                    name,
                    descriptor,
                    operation,
                    arguments.as_slice(),
                ));
            },
            | _ => {},
        }
        if builtin == CommonBuiltin::Library {
            return None;
        }
        // A builtin's name is reserved to the implementation (C99 §7.1.3p1),
        // so neither GCC nor Clang reports a call under `-pedantic`; its
        // feature only decides whether the name is a builtin at all.
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
        // C99 §6.4.5p2 wide string literals, as Clang requires of
        // Microsoft's `__annotation`.
        if descriptor.type_rule == crate::builtins::TypeRule::WideStrings {
            for &argument in args {
                let mut literal = argument;
                while let ExpressionType::Parenthesized { expression } = literal.kind {
                    literal = expression;
                }
                if !matches!(
                    literal.kind,
                    ExpressionType::StringLiteral(
                        crate::translation_phases::preprocessing::StringTokenType::WideString(_)
                    )
                ) {
                    self.error(
                        SemanticErrorKind::AnnotationArgument,
                        argument.source_vectors,
                        None,
                        None,
                    );
                }
            }
        }
        if descriptor.type_rule == crate::builtins::TypeRule::Fixed {
            for (index, &argument) in args.iter().enumerate() {
                self.check_registry_argument(descriptor, index, argument, name.name);
            }
        }
        match builtin {
            | CommonBuiltin::Float(operation, scalar) => {
                return Some(self.floating_builtin(e, args, operation, scalar));
            },
            | CommonBuiltin::Complex => return Some(self.complex_builtin(e, args)),
            | CommonBuiltin::FloatConstant(scalar, nan) => {
                if nan {
                    // A dynamic tag lowers as a library call. Retain its
                    // ordinary function decay on the original callee,
                    // including transparent selections and parentheses.
                    // C99: §6.3.2.1p4, p. 46; PDF p. 58.
                    _ = self.converted(self.expression_info(function));
                }
                return Some(self.floating_constant_builtin(e, args, scalar, nan));
            },
            | CommonBuiltin::Rotate(width, right) => {
                let value = self.expression_info(args[0]);
                let count = self.expression_info(args[1]);
                if value.ice
                    && count.ice
                    && let Some((value, count)) = value.integer.zip(count.integer)
                {
                    let value = value.cast(width, false).value.unsigned_abs();
                    let count = u32::try_from(count.cast(width, false).value % i128::from(width))
                        .expect("rotation count is reduced modulo a supported width");
                    let opposite = (width - count) % width;
                    let value = if right {
                        (value >> count) | (value << opposite)
                    } else {
                        (value << count) | (value >> opposite)
                    } & ((1_u128 << width) - 1);
                    info.integer = Some(
                        Integer::int(
                            i128::try_from(value).expect("rotation result has at most 64 bits"),
                        )
                        .cast(width, false),
                    );
                    info.ice = true;
                    info.constant = ConstantClass::Arithmetic;
                }
            },
            | CommonBuiltin::Constant => {
                info.integer = Some(self.constant_query_integer(args[0]));
                info.runtime_query = self.constant_query_differs_at_runtime(args[0]);
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            },
            | CommonBuiltin::ObjectSize(dynamic) => {
                let ConversionRule::Parameter(pattern) = descriptor.arguments[1].conversion else {
                    return None;
                };
                let int = self.signature_type(pattern);
                let mode = self
                    .common_integer_constant(args[1], int)
                    .and_then(Integer::to_u64)
                    .and_then(|mode| u8::try_from(mode).ok())
                    .unwrap_or(0);
                let pure = self.effects_pure(args[0], super::super::EffectContext::Value);
                let size = if pure {
                    self.infer_object_size(args[0], mode, dynamic)
                } else {
                    super::super::ObjectSize::Unknown
                };
                // In a function the object may be known after inlining, as
                // for a wrapper's parameter: the query is then no constant
                // expression, as in Clang, and the middle end answers it. An
                // operand whose evaluation could do anything GCC's
                // unevaluated one does not (side effects, reads through a
                // pointer, atomic or volatile reads, calls) is not
                // evaluated, so its answer is the unknown one now (GCC
                // manual, "Object Size Checking").
                let size = match size {
                    | super::super::ObjectSize::Unknown
                        if pure
                            && self.functions.current.is_some()
                            && mode <= 3
                            && self.deferrable_operand(args[0]) =>
                        super::super::ObjectSize::Deferred {
                            mode,
                            dynamic,
                            through: self.pointer_parameter_base(args[0]),
                        },
                    | super::super::ObjectSize::Unknown =>
                        super::super::ObjectSize::Constant(if mode >= 2 { 0 } else { u64::MAX }),
                    | size => size,
                };
                if let super::super::ObjectSize::Constant(size) = size {
                    let (bits, signed) = self.integer_type(result)?;
                    info.integer = Some(self.wide.convert(
                        Integer::int(i128::from(size)),
                        bits,
                        signed,
                    ));
                    info.ice = true;
                    info.constant = ConstantClass::Arithmetic;
                }
                _ = self
                    .function_extensions
                    .object_sizes
                    .insert(std::ptr::from_ref(e).addr(), size);
            },
            | CommonBuiltin::Overflow(operation, scalar, predicate) => {
                if let Some(arg) = self.wide_signed_product(operation, args) {
                    self.error(
                        SemanticErrorKind::OverflowBuiltinBitInt,
                        arg.source_vectors,
                        None,
                        None,
                    );
                    return Some(Self::expression_result(e, self.types.unknown()));
                }
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
                // GCC and Clang accept a bit-precise destination too.
                let valid = destination.is_some_and(|ty| {
                    matches!(
                        self.types.nodes[ty.index],
                        TypeKind::Scalar(s) if s.integer() && s != Scalar::Bool
                    ) || (matches!(self.types.nodes[ty.index], TypeKind::BitInt { .. })
                        && scalar.is_none())
                }) && destination.is_some_and(|ty| {
                    (predicate || !ty.qualifiers.contains(TypeQualifiers::CONST))
                        && scalar.is_none_or(|s| self.types.nodes[ty.index] == TypeKind::Scalar(s))
                });
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
                            && self.effects_pure(
                                args[2],
                                crate::translation_phases::semantic_analysis::EffectContext::Value,
                            )
                            && let Some((a, b)) = a.integer.zip(b.integer)
                            && let Some((bits, signed)) = self.integer_type(destination)
                        {
                            let bits = operand.bit_field.unwrap_or(bits);
                            let overflows = if a.wide() || b.wide() || bits > 128 {
                                self.wide.overflows(
                                    match operation {
                                        | OverflowOperation::Add => BinaryOperator::Addition,
                                        | OverflowOperation::Sub => BinaryOperator::Subtraction,
                                        | OverflowOperation::Mul => BinaryOperator::Multiplication,
                                    },
                                    a,
                                    b,
                                    bits,
                                    signed,
                                )
                            } else {
                                constant_overflow(operation, a, b, bits, signed)
                            };
                            info.integer = Some(Integer::int(i128::from(overflows)));
                            info.ice = true;
                            info.constant = ConstantClass::Arithmetic;
                        }
                    }
                }
            },
            // Lowering substitutes the pack in a copy of this function per
            // call, so it must know which bodies evaluate it.
            | CommonBuiltin::ArgumentPack(_) => self.argument_pack_use(),
            | CommonBuiltin::Atomic(_)
            | CommonBuiltin::Bit(..)
            | CommonBuiltin::GenericBit(_)
            | CommonBuiltin::Bswap(_)
            | CommonBuiltin::Expect(_)
            | CommonBuiltin::Aligned
            | CommonBuiltin::Prefetch
            | CommonBuiltin::Alloca
            | CommonBuiltin::Address(_)
            | CommonBuiltin::ExtractReturnAddress
            | CommonBuiltin::Unreachable
            | CommonBuiltin::Trap
            | CommonBuiltin::Setjmp
            | CommonBuiltin::Longjmp
            | CommonBuiltin::Varargs(_)
            | CommonBuiltin::Library
            | CommonBuiltin::Offsetof
            | CommonBuiltin::ReadTimestamp
            | CommonBuiltin::DebugTrap
            | CommonBuiltin::Annotation
            | CommonBuiltin::Semantic(_)
            | CommonBuiltin::Unsupported(_) => {},
        }
        Some(info)
    }

    /// Under the Clang identity, the first operand or result of a
    /// `__builtin_mul_overflow` that is a signed `_BitInt` of more than 128
    /// bits, which Clang refuses (its runtime multiplies at most 128 bits);
    /// an unsigned one is accepted, and GCC accepts both.
    /// Clang extension: the error "`__builtin_mul_overflow` does not support
    /// 'signed `_BitInt`' operands of more than 128 bits".
    fn wide_signed_product(
        &mut self,
        operation: OverflowOperation,
        args: &[&'tu Expression<'tu>],
    ) -> Option<&'tu Expression<'tu>> {
        if operation != OverflowOperation::Mul
            || self.context.configuration.bit_int_rules() != crate::configuration::RuleSet::Clang
        {
            return None;
        }
        args.iter().take(3).enumerate().find_map(|(index, &arg)| {
            let ty = self.converted(self.expression_info(arg));
            let ty = if index == 2 {
                match self.types.nodes[ty.index] {
                    | TypeKind::Pointer(target) => target,
                    | _ => return None,
                }
            } else {
                ty
            };
            matches!(
                self.types.nodes[ty.index],
                TypeKind::BitInt { width, signed: true } if width > 128
            )
            .then_some(arg)
        })
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
            // Immediates belong to x86 prototypes, checked by `x86_immediates`.
            | ConstantConstraint::None | ConstantConstraint::Immediate(..) => true,
            | ConstantConstraint::ConvertedIntegerRange(max) => self
                .common_integer_constant(argument, ty)
                .and_then(Integer::to_u64)
                .is_some_and(|v| v <= max),
            | ConstantConstraint::OriginalIntegerRange(max) => info
                .integer
                .filter(|_| info.ice)
                .and_then(Integer::to_u64)
                .is_some_and(|v| v <= max),
            | ConstantConstraint::ConvertedIntegerValue(value) => {
                let converted = self.common_integer_constant(argument, ty);
                if let Some((original, converted)) =
                    info.integer.filter(|_| info.ice).zip(converted)
                    && original.value != converted.value
                {
                    self.constant_conversion_warning(argument, info.ty, ty, original, converted);
                }
                converted
                    .and_then(Integer::to_u64)
                    .is_some_and(|v| v == value)
            },
            | ConstantConstraint::ConvertedPowerOfTwo => self
                .common_integer_constant(argument, ty)
                .and_then(Integer::to_u64)
                .is_some_and(u64::is_power_of_two),
            | ConstantConstraint::ConvertedProbability =>
                info.constant == ConstantClass::Arithmetic
                    && self.converted_floating(info, ty).is_some_and(|v| {
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

    /// Clang's `-Wconstant-conversion`: an integer constant argument whose
    /// conversion to the parameter type `to` changes its value, which wraps
    /// modulo 2^N on every target, an implementation-defined choice.
    /// C99: §6.3.1.3p3, p. 43; PDF p. 55.
    fn constant_conversion_warning(
        &mut self,
        argument: &'tu Expression<'tu>,
        from: TypeId,
        to: TypeId,
        original: Integer,
        converted: Integer,
    ) {
        let (TypeKind::Scalar(from), TypeKind::Scalar(to)) =
            (self.types.nodes[from.index], self.types.nodes[to.index])
        else {
            return;
        };
        let text = crate::diagnostics::format_in!(
            self.scratch,
            "implicit conversion from `{}` to `{}` changes value from {} to {}",
            from.spelling(),
            to.spelling(),
            original.value,
            converted.value
        );
        let text = self.context.string_cache.intern(text);
        self.group_diagnostic(
            crate::configuration::DiagnosticGroup::ConstantConversion,
            SemanticErrorKind::ConstantConversion(text),
            argument.source_vectors,
            None,
            None,
        );
    }

    /// Reports incompatible types before checking dependent value constraints.
    /// C99: §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84.
    pub(super) fn common_argument(&mut self, e: &'tu Expression<'tu>, ty: TypeId) -> bool {
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
        Some(self.wide.convert(value, bits, signed))
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
