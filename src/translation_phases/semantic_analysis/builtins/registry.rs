//! Semantic projection of neutral registry signatures and diagnostic
//! identities. C99: §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84.

use crate::{
    builtins::{
        BuiltinDescriptor,
        ConstraintDiagnostic,
        SignatureType,
    },
    translation_phases::{
        parsing::declaration_syntax::TypeQualifiers,
        semantic_analysis::{
            Analyzer,
            Scalar,
            SemanticErrorKind,
            TypeId,
            TypeKind,
        },
    },
    util::bump::ArenaVec,
};

impl Analyzer<'_, '_, '_> {
    pub(in crate::translation_phases::semantic_analysis) fn descriptor_type(
        &mut self,
        descriptor: &BuiltinDescriptor,
    ) -> TypeId {
        let signature = descriptor.signature;
        let result = self.signature_type(signature.result);
        let mut parameters = ArenaVec::new_in(self.types.tu);
        for &parameter in signature.parameters {
            parameters.push(self.signature_type(parameter));
        }
        self.types.intern(TypeKind::Function {
            result,
            parameters: parameters.leak(),
            prototype: signature.prototype,
            variadic: signature.variadic,
        })
    }

    pub(in crate::translation_phases::semantic_analysis) fn signature_type(
        &mut self,
        pattern: SignatureType,
    ) -> TypeId {
        match pattern {
            | SignatureType::Scalar(scalar) => self.types.scalar(scalar),
            | SignatureType::Size => self.types.scalar(self.types.target.size_t),
            | SignatureType::Unsigned(bits) => {
                let scalar = match bits {
                    | 16 => Scalar::UnsignedShort,
                    | 32 => Scalar::UnsignedInt,
                    | _ if self
                        .types
                        .target
                        .integer(Scalar::UnsignedLong)
                        .is_some_and(|(bits, _)| bits == 64) =>
                        Scalar::UnsignedLong,
                    | _ => Scalar::UnsignedLongLong,
                };
                self.types.scalar(scalar)
            },
            | SignatureType::Pointer(scalar, constant) => {
                let mut ty = self.types.scalar(scalar);
                if constant {
                    ty = ty.qualified(TypeQualifiers::CONST);
                }
                self.types.intern(TypeKind::Pointer(ty))
            },
            | SignatureType::VaList => self.va_list_parameter(),
            | SignatureType::VaListPointer => {
                let list = self.builtin_va_list();
                self.types.intern(TypeKind::Pointer(list))
            },
            | SignatureType::File => self.file_parameter(),
            | SignatureType::Vector(scalar, count) => self.signature_vector(scalar, count),
            | SignatureType::VectorPointer(scalar, count, constant) => {
                let mut ty = self.signature_vector(scalar, count);
                if constant {
                    ty = ty.qualified(TypeQualifiers::CONST);
                }
                self.types.intern(TypeKind::Pointer(ty))
            },
            | SignatureType::Opaque => self.types.unknown(),
        }
    }

    /// A GNU vector aligned to its size, as `vector_size` declares one.
    /// GNU extension: GCC manual, "Vector Extensions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    fn signature_vector(&mut self, scalar: Scalar, count: u8) -> TypeId {
        let element = self.types.scalar(scalar);
        let count = u64::from(count);
        let align = self
            .types
            .layout(element)
            .map_or(0, |layout| layout.size * count);
        self.types.intern(TypeKind::Vector {
            element,
            count,
            align,
        })
    }
}

pub(super) fn diagnostic(identity: ConstraintDiagnostic) -> SemanticErrorKind {
    match identity {
        | ConstraintDiagnostic::ArgumentType => SemanticErrorKind::InvalidArgumentType,
        | ConstraintDiagnostic::ObjectSizeMode => SemanticErrorKind::InvalidObjectSizeMode,
        | ConstraintDiagnostic::Alignment => SemanticErrorKind::InvalidBuiltinAlignment,
        | ConstraintDiagnostic::Probability => SemanticErrorKind::InvalidBuiltinProbability,
        | ConstraintDiagnostic::PrefetchReadWrite => SemanticErrorKind::InvalidPrefetchReadWrite,
        | ConstraintDiagnostic::PrefetchLocality => SemanticErrorKind::InvalidPrefetchLocality,
        | ConstraintDiagnostic::FrameLevel => SemanticErrorKind::InvalidFrameLevel,
        | ConstraintDiagnostic::VaList => SemanticErrorKind::InvalidVaList,
        | ConstraintDiagnostic::VaArgType => SemanticErrorKind::InvalidVaArgType,
        | ConstraintDiagnostic::Immediate => SemanticErrorKind::InvalidImmediate,
        | ConstraintDiagnostic::LongjmpValue => SemanticErrorKind::InvalidLongjmpValue,
        | ConstraintDiagnostic::AllocaMaximum => SemanticErrorKind::InvalidAllocaMaximum,
    }
}

/// Adapter from the parser's sole keyword classifier to neutral special forms.
pub(in crate::translation_phases::semantic_analysis) fn keyword_descriptor(
    keyword: crate::translation_phases::preprocessing::KeywordTokenType,
) -> Option<&'static BuiltinDescriptor> {
    use crate::{
        builtins::BuiltinId as I,
        translation_phases::preprocessing::KeywordTokenType as K,
    };
    Some(
        match keyword {
            | K::BuiltinVaStart => I::VaStart,
            | K::BuiltinVaEnd => I::VaEnd,
            | K::BuiltinVaCopy => I::VaCopy,
            | K::BuiltinVaArg => I::VaArg,
            | K::BuiltinOffsetof => I::Offsetof,
            | K::BuiltinTypesCompatible => I::TypesCompatible,
            | K::BuiltinChooseExpr => I::ChooseExpr,
            | K::BuiltinConvertVector => I::Convertvector,
            | K::BuiltinBitCast => I::BitCast,
            | _ => return None,
        }
        .descriptor(),
    )
}
