//! Common GCC scalar intrinsics. Calls are checked after their operands on
//! the semantic work stack; lowering shares the name classifier.
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
    configuration::Feature,
    translation_phases::{
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::{
                Expression,
                ExpressionType,
                Identifier,
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
    /// GCC signatures are independent of the host's long and `size_t` widths.
    /// GNU extension: GCC manuals linked in this module's documentation.
    pub(in crate::translation_phases::semantic_analysis) fn common_builtin_type(
        &mut self,
        name: Identifier,
    ) -> Option<TypeId> {
        let builtin = common_builtin(self.context.string_cache.at(name.name))?;
        let scalar = match builtin {
            | CommonBuiltin::Float(FloatOperation::Copysign, scalar) =>
                scalar.unwrap_or(Scalar::Double),
            | CommonBuiltin::Bit(_, _) | CommonBuiltin::Constant | CommonBuiltin::Float(..) =>
                Scalar::Int,
            | CommonBuiltin::Bswap(bits) => self.bswap_scalar(bits),
            | CommonBuiltin::Expect(_) => Scalar::Long,
            | CommonBuiltin::ObjectSize(_) => self.types.target.size_t,
            | CommonBuiltin::Overflow(..) => Scalar::Bool,
            | CommonBuiltin::Complex => Scalar::ComplexDouble,
            | _ => Scalar::Void,
        };
        let mut result = self.types.scalar(scalar);
        if matches!(
            builtin,
            CommonBuiltin::Aligned
                | CommonBuiltin::Alloca
                | CommonBuiltin::Address(_)
                | CommonBuiltin::ExtractReturnAddress
        ) {
            result = self.types.intern(TypeKind::Pointer(result));
        }
        let void = self
            .types
            .scalar(Scalar::Void)
            .qualified(TypeQualifiers::CONST);
        let const_pointer = self.types.intern(TypeKind::Pointer(void));
        let size = self.types.scalar(self.types.target.size_t);
        let long = self.types.scalar(Scalar::Long);
        let int = self.types.scalar(Scalar::Int);
        let mut parameters = crate::util::bump::ArenaVec::new_in(self.types.tu);
        let mut prototype = true;
        let mut variadic = false;
        match builtin {
            | CommonBuiltin::Float(operation, scalar) =>
                if let Some(scalar) = scalar {
                    let ty = self.types.scalar(scalar);
                    parameters.push(ty);
                    if operation == FloatOperation::Copysign {
                        parameters.push(ty);
                    }
                } else {
                    prototype = false;
                },
            | CommonBuiltin::Bit(_, scalar) => parameters.push(self.types.scalar(scalar)),
            | CommonBuiltin::Bswap(_) | CommonBuiltin::ExtractReturnAddress =>
                parameters.push(result),
            | CommonBuiltin::Expect(probability) => {
                parameters.extend_from_slice(&[long, long]);
                if probability {
                    parameters.push(self.types.scalar(Scalar::Double));
                }
            },
            | CommonBuiltin::Aligned => {
                parameters.extend_from_slice(&[const_pointer, size]);
                variadic = true;
            },
            | CommonBuiltin::Prefetch => {
                parameters.push(const_pointer);
                variadic = true;
            },
            | CommonBuiltin::ObjectSize(_) => parameters.extend_from_slice(&[const_pointer, int]),
            | CommonBuiltin::Alloca => parameters.push(size),
            | CommonBuiltin::Address(_) => parameters.push(self.types.scalar(Scalar::UnsignedInt)),
            | CommonBuiltin::Overflow(_, Some(scalar), false) => {
                let integer = self.types.scalar(scalar);
                let pointer = self.types.intern(TypeKind::Pointer(integer));
                parameters.extend_from_slice(&[integer, integer, pointer]);
            },
            | CommonBuiltin::Constant
            | CommonBuiltin::Overflow(..)
            | CommonBuiltin::Varargs(_)
            | CommonBuiltin::Complex => prototype = false,
            | CommonBuiltin::Unreachable | CommonBuiltin::Trap => {},
        }
        Some(self.types.intern(TypeKind::Function {
            result,
            parameters: parameters.leak(),
            prototype,
            variadic,
        }))
    }

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
        let callee = super::super::selected_designator(callee, |e| Some(self.expression_info(e)));
        let ExpressionType::Identifier(name) = callee.kind else {
            return None;
        };
        let spelling = self
            .context
            .tu_arena()
            .alloc_str(self.context.string_cache.at(name.name));
        let builtin = common_builtin(spelling)?;
        if !self
            .expression_info(callee)
            .binding
            .is_some_and(|id| self.bindings[id].builtin)
        {
            return None;
        }
        if let CommonBuiltin::Varargs(keyword) = builtin {
            let count = if keyword
                == crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaEnd
            {
                1
            } else {
                2
            };
            if arguments.len() != count {
                self.error(
                    SemanticErrorKind::InvalidArgumentCount,
                    e.source_vectors,
                    None,
                    None,
                );
                return Some(Self::expression_result(e, self.types.unknown()));
            }
            let b = keyword_call(self.types.tu, e, keyword, arguments);
            return Some(self.type_builtin(e, b));
        }
        self.context
            .report_extension(Feature::GnuBuiltins, spelling, name.source_vectors);
        let args = arguments.as_slice();
        let (min, max) = match builtin {
            | CommonBuiltin::Float(FloatOperation::Copysign, _)
            | CommonBuiltin::Expect(false)
            | CommonBuiltin::ObjectSize(_)
            | CommonBuiltin::Complex => (2, 2),
            | CommonBuiltin::Float(FloatOperation::Fpclassify, _) => (6, 6),
            | CommonBuiltin::Bit(..)
            | CommonBuiltin::Bswap(_)
            | CommonBuiltin::Constant
            | CommonBuiltin::Alloca
            | CommonBuiltin::Address(_)
            | CommonBuiltin::ExtractReturnAddress
            | CommonBuiltin::Float(..) => (1, 1),
            | CommonBuiltin::Expect(true) | CommonBuiltin::Overflow(..) => (3, 3),
            | CommonBuiltin::Aligned => (2, 3),
            | CommonBuiltin::Prefetch => (1, 3),
            | _ => (0, 0),
        };
        if !(min..=max).contains(&args.len()) {
            self.error(
                SemanticErrorKind::InvalidArgumentCount,
                e.source_vectors,
                None,
                None,
            );
            return Some(Self::expression_result(e, self.types.unknown()));
        }
        let mut result = self.expression_info(callee).ty;
        if let TypeKind::Function { result: r, .. } = self.types.nodes[result.index] {
            result = r;
        }
        // Intrinsic bindings retain the implementation's signature.
        let intrinsic_type = self.common_builtin_type(name)?;
        if let TypeKind::Function { result: r, .. } = self.types.nodes[intrinsic_type.index] {
            result = r;
        }
        let mut info = Self::expression_result(e, result);
        match builtin {
            | CommonBuiltin::Float(operation, scalar) => {
                return Some(self.floating_builtin(e, args, operation, scalar));
            },
            | CommonBuiltin::Complex => return Some(self.complex_builtin(e, args)),
            | CommonBuiltin::Bit(_, scalar) => {
                let ty = self.types.scalar(scalar);
                _ = self.common_argument(args[0], ty);
            },
            | CommonBuiltin::Bswap(bits) => {
                let scalar = self.bswap_scalar(bits);
                let ty = self.types.scalar(scalar);
                _ = self.common_argument(args[0], ty);
            },
            | CommonBuiltin::Expect(probability) => {
                let ty = self.types.scalar(Scalar::Long);
                _ = self.common_argument(args[0], ty);
                _ = self.common_argument(args[1], ty);
                if probability {
                    let ty = self.types.scalar(Scalar::Double);
                    let compatible = self.common_argument(args[2], ty);
                    let p = self.expression_info(args[2]);
                    // GCC's probability parameter is double; check its
                    // converted value, including rounding
                    // onto either endpoint. C99 §6.3.1.5p2,
                    // p. 44; PDF p. 56.
                    let value = self
                        .floating_value(p)
                        .and_then(|value| self.round_floating(value, ty));
                    if compatible
                        && (p.constant != ConstantClass::Arithmetic
                            || value.is_none_or(|v| {
                                !matches!(
                                    v.real.compare(crate::float_parsing::LongDouble::ZERO),
                                    0 | 1
                                ) || !matches!(
                                    v.real
                                        .compare(crate::float_parsing::LongDouble::from_double(
                                            1.0
                                        )),
                                    -1 | 0
                                )
                            }))
                    {
                        self.error(
                            SemanticErrorKind::InvalidBuiltinProbability,
                            args[2].source_vectors,
                            Some(name.name),
                            None,
                        );
                    }
                }
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
                let void = self
                    .types
                    .scalar(Scalar::Void)
                    .qualified(TypeQualifiers::CONST);
                let ptr = self.types.intern(TypeKind::Pointer(void));
                _ = self.common_argument(args[0], ptr);
                let int = self.types.scalar(Scalar::Int);
                let compatible = self.common_argument(args[1], int);
                let value = self
                    .common_integer_constant(args[1], int)
                    .and_then(Integer::to_u64);
                if compatible && value.is_none_or(|v| v > 3) {
                    self.error(
                        SemanticErrorKind::InvalidObjectSizeMode,
                        args[1].source_vectors,
                        Some(name.name),
                        None,
                    );
                }
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
            | CommonBuiltin::Aligned | CommonBuiltin::Prefetch => {
                let void = self
                    .types
                    .scalar(Scalar::Void)
                    .qualified(TypeQualifiers::CONST);
                let ptr = self.types.intern(TypeKind::Pointer(void));
                _ = self.common_argument(args[0], ptr);
                for (i, &arg) in args.iter().enumerate().skip(1) {
                    let scalar = if builtin == CommonBuiltin::Aligned {
                        self.types.target.size_t
                    } else {
                        Scalar::Int
                    };
                    let ty = self.types.scalar(scalar);
                    let compatible = self.common_argument(arg, ty);
                    let operand = self.expression_info(arg);
                    let value = if builtin == CommonBuiltin::Aligned {
                        self.common_integer_constant(arg, ty)
                    } else {
                        // GCC prefetch's optional ICEs follow `...`, so they
                        // have no fixed int parameter to narrow them first.
                        // C99 §6.5.2.2p7, p. 72; PDF p. 84.
                        operand.integer.filter(|_| operand.ice)
                    }
                    .and_then(Integer::to_u64);
                    let valid = if builtin == CommonBuiltin::Aligned {
                        i == 2 || value.is_some_and(u64::is_power_of_two)
                    } else {
                        value.is_some_and(|v| v <= if i == 1 { 1 } else { 3 })
                    };
                    if compatible && !valid {
                        self.error(
                            if builtin == CommonBuiltin::Aligned {
                                SemanticErrorKind::InvalidBuiltinAlignment
                            } else if i == 1 {
                                SemanticErrorKind::InvalidPrefetchReadWrite
                            } else {
                                SemanticErrorKind::InvalidPrefetchLocality
                            },
                            arg.source_vectors,
                            Some(name.name),
                            None,
                        );
                    }
                }
            },
            | CommonBuiltin::Alloca => {
                let ty = self.types.scalar(self.types.target.size_t);
                _ = self.common_argument(args[0], ty);
            },
            | CommonBuiltin::Address(_) => {
                let ty = self.types.scalar(Scalar::UnsignedInt);
                if self.common_argument(args[0], ty)
                    && self.common_integer_constant(args[0], ty).is_none()
                {
                    self.error(
                        SemanticErrorKind::InvalidFrameLevel,
                        args[0].source_vectors,
                        Some(name.name),
                        None,
                    );
                }
            },
            | CommonBuiltin::ExtractReturnAddress => {
                _ = self.common_argument(args[0], result);
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
            | CommonBuiltin::Unreachable | CommonBuiltin::Trap | CommonBuiltin::Varargs(_) => {},
        }
        Some(info)
    }

    /// GCC's `uint64_t` is unsigned long on LP64 and unsigned long long on
    /// LLP64. GNU extension: GCC "Byte-Swapping Builtins"; GCC 15.2
    /// `BT_UINT64` uses the target's `UINT64_TYPE` in c-family/c-common.cc.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Byte-Swapping-Builtins.html>
    fn bswap_scalar(&self, bits: u32) -> Scalar {
        match bits {
            | 16 => Scalar::UnsignedShort,
            | 32 => Scalar::UnsignedInt,
            | _ if self
                .types
                .target
                .integer(Scalar::UnsignedLong)
                .is_some_and(|(bits, _)| bits == 64) =>
                Scalar::UnsignedLong,
            | _ => Scalar::UnsignedLongLong,
        }
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
        Some(value.cast(bits, signed))
    }
}

/// The operations whose GCC names share one documented signature family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CommonBuiltin {
    Varargs(crate::translation_phases::preprocessing::KeywordTokenType),
    Bit(BitOperation, Scalar),
    Float(FloatOperation, Option<Scalar>),
    Bswap(u32),
    Expect(bool),
    Aligned,
    Unreachable,
    Trap,
    Prefetch,
    Constant,
    ObjectSize(bool),
    Alloca,
    /// GNU frame (`false`) or return (`true`) address of a constant frame
    /// level. <https://gcc.gnu.org/onlinedocs/gcc/Return-Address.html>
    Address(bool),
    /// GNU: the return address as an address; the identity on x86-64.
    ExtractReturnAddress,
    Overflow(OverflowOperation, Option<Scalar>, bool),
    /// GNU: the complex value of a real and an imaginary part of one real
    /// floating type. <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    Complex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatOperation {
    Signbit,
    Isinf,
    Copysign,
    /// The type-generic classification macros of C99 §7.12.3, pp. 216-217;
    /// PDF pp. 228-229, as GCC and Clang builtins.
    Isnan,
    Isfinite,
    Isnormal,
    /// GNU: -1, 0 or 1 for negative infinity, non-infinity or infinity.
    IsinfSign,
    /// GNU: five `int` results (NaN, infinite, normal, subnormal, zero),
    /// then the classified operand. C99: §7.12.3.1, p. 216; PDF p. 228.
    Fpclassify,
}

impl FloatOperation {
    /// The index of the floating operand among the call's arguments.
    pub(crate) const fn operand_index(self) -> usize {
        if matches!(self, Self::Fpclassify) {
            5
        } else {
            0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BitOperation {
    Clz,
    Ctz,
    Popcount,
    Parity,
    Ffs,
    Clrsb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverflowOperation {
    Add,
    Sub,
    Mul,
}

/// Recognition is exact: an unsupported suffix must remain unsupported.
/// GNU extension: GCC manuals linked in this module's documentation.
pub(crate) fn common_builtin(name: &str) -> Option<CommonBuiltin> {
    use CommonBuiltin as B;

    use crate::translation_phases::preprocessing::KeywordTokenType as K;
    let name = name
        .strip_prefix("__builtin_")
        .or_else(|| (name == "alloca").then_some("alloca"))?;
    match name {
        | "va_start" => return Some(B::Varargs(K::BuiltinVaStart)),
        | "va_end" => return Some(B::Varargs(K::BuiltinVaEnd)),
        | "va_copy" => return Some(B::Varargs(K::BuiltinVaCopy)),
        | _ => {},
    }
    // The C99 classification macros exist only in their type-generic form.
    match name {
        | "isnan" => return Some(B::Float(FloatOperation::Isnan, None)),
        | "isfinite" => return Some(B::Float(FloatOperation::Isfinite, None)),
        | "isnormal" => return Some(B::Float(FloatOperation::Isnormal, None)),
        | "isinf_sign" => return Some(B::Float(FloatOperation::IsinfSign, None)),
        | "fpclassify" => return Some(B::Float(FloatOperation::Fpclassify, None)),
        | _ => {},
    }
    for (prefix, operation) in [
        ("signbit", FloatOperation::Signbit),
        ("isinf", FloatOperation::Isinf),
        ("copysign", FloatOperation::Copysign),
    ] {
        if let Some(suffix) = name.strip_prefix(prefix) {
            let scalar = match suffix {
                | "" if operation != FloatOperation::Copysign => None,
                | "" => Some(Scalar::Double),
                | "f" => Some(Scalar::Float),
                | "l" => Some(Scalar::LongDouble),
                | _ => continue,
            };
            return Some(B::Float(operation, scalar));
        }
    }
    for (prefix, op) in [
        ("clz", BitOperation::Clz),
        ("ctz", BitOperation::Ctz),
        ("popcount", BitOperation::Popcount),
        ("parity", BitOperation::Parity),
        ("ffs", BitOperation::Ffs),
        ("clrsb", BitOperation::Clrsb),
    ] {
        if let Some(suffix) = name.strip_prefix(prefix) {
            let signed = matches!(op, BitOperation::Ffs | BitOperation::Clrsb);
            let scalar = match (suffix, signed) {
                | ("", true) => Scalar::Int,
                | ("", false) => Scalar::UnsignedInt,
                | ("l", true) => Scalar::Long,
                | ("l", false) => Scalar::UnsignedLong,
                | ("ll", true) => Scalar::LongLong,
                | ("ll", false) => Scalar::UnsignedLongLong,
                | _ => continue,
            };
            return Some(B::Bit(op, scalar));
        }
    }
    for (prefix, op) in [
        ("add", OverflowOperation::Add),
        ("sub", OverflowOperation::Sub),
        ("mul", OverflowOperation::Mul),
    ] {
        if name.strip_suffix("_overflow") == Some(prefix) {
            return Some(B::Overflow(op, None, false));
        }
        if name.strip_suffix("_overflow_p") == Some(prefix) {
            return Some(B::Overflow(op, None, true));
        }
        for (signed, lead) in [(true, "s"), (false, "u")] {
            if let Some(tail) = name
                .strip_prefix(lead)
                .and_then(|n| n.strip_prefix(prefix))
                .and_then(|n| n.strip_suffix("_overflow"))
            {
                let scalar = match (tail, signed) {
                    | ("", true) => Scalar::Int,
                    | ("", false) => Scalar::UnsignedInt,
                    | ("l", true) => Scalar::Long,
                    | ("l", false) => Scalar::UnsignedLong,
                    | ("ll", true) => Scalar::LongLong,
                    | ("ll", false) => Scalar::UnsignedLongLong,
                    | _ => continue,
                };
                return Some(B::Overflow(op, Some(scalar), false));
            }
        }
    }
    Some(match name {
        | "bswap16" => B::Bswap(16),
        | "bswap32" => B::Bswap(32),
        | "bswap64" => B::Bswap(64),
        | "expect" => B::Expect(false),
        | "expect_with_probability" => B::Expect(true),
        | "assume_aligned" => B::Aligned,
        | "unreachable" => B::Unreachable,
        | "trap" => B::Trap,
        | "prefetch" => B::Prefetch,
        | "constant_p" => B::Constant,
        | "object_size" => B::ObjectSize(false),
        | "dynamic_object_size" => B::ObjectSize(true),
        | "alloca" => B::Alloca,
        | "frame_address" => B::Address(false),
        | "return_address" => B::Address(true),
        | "extract_return_addr" => B::ExtractReturnAddress,
        | "complex" => B::Complex,
        | _ => return None,
    })
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

/// Expression-only varargs calls share the header intrinsic operand checker.
/// C99: §7.15.1, pp. 249-251; PDF pp. 261-263.
pub(crate) fn keyword_call<'tu>(
    arena: &'tu crate::util::bump::Bump,
    expression: &'tu Expression<'tu>,
    keyword: crate::translation_phases::preprocessing::KeywordTokenType,
    arguments: ArenaList<'tu, &'tu Expression<'tu>>,
) -> &'tu crate::translation_phases::parsing::Builtin<'tu> {
    let mut operands = crate::util::bump::ArenaVec::new_in(arena);
    for &arg in arguments {
        operands.push(crate::translation_phases::parsing::SyntaxOperand::Expression(arg));
    }
    arena.alloc(crate::translation_phases::parsing::Builtin {
        keyword,
        operands: ArenaList::copy_from_slice(arena, &operands),
        members: ArenaList::empty(),
        source_vectors: expression.source_vectors,
        recovered: false,
    })
}
