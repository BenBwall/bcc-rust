//! The feature vocabulary assigns each syntax or library capability its
//! language origin. Configuration uses that origin to compute feature sets.

use super::{
    CStandard,
    MsvcFeature,
};

/// Shared feature vocabulary. Reserved or unambiguous syntax is accepted
/// earlier as an extension; spellings that change valid programs are gated.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub(crate) enum Feature {
    Digraphs,
    LineComments,
    Trigraphs,
    UnicodeLiteralPrefixes,
    Utf8CharacterConstants,
    DigitSeparators,
    BitIntSuffixes,
    BinaryConstants,
    Elifdef,
    WarningDirective,
    Embed,
    HasInclude,
    HasEmbed,
    HasCAttribute,
    VaOpt,
    OctalPrefix,
    DelimitedEscapes,
    HexFloats,
    VariadicMacros,
    /// Omitting the argument for a variadic macro's ellipsis.
    /// C23: §6.10.5 paragraph 4, p. 178; PDF p. 191.
    OmittedVariadicArguments,
    EmptyMacroArguments,
    Inline,
    Restrict,
    Bool,
    Complex,
    Imaginary,
    ImplicitInt,
    ImplicitFunctionDeclaration,
    MixedDeclarations,
    ForDeclarations,
    /// Non-variable declarations in a for initializer.
    /// C23: §6.8.6.1 paragraphs 1-2, p. 155; PDF p. 168; relaxes
    /// C99 §6.8.5 paragraph 3, p. 135; PDF p. 147.
    ForNonVariableDeclarations,
    /// Qualified, static and prototype-star array parameters.
    /// C99: §6.7.5 paragraph 1, p. 114; PDF p. 126.
    ArrayParameterSyntax,
    DesignatedInitializers,
    CompoundLiterals,
    FlexibleArrayMembers,
    LongLong,
    TrailingEnumComma,
    /// Identifier-list function declarators, removed in C23.
    /// C99: §6.7.5 paragraph 1, p. 114; PDF p. 126. C23: §6.7.7.1
    /// paragraph 1, pp. 126-127; PDF pp. 139-140.
    OldStyleFunctionDeclarators,
    Func,
    StaticAssert,
    Generic,
    Alignas,
    Alignof,
    Noreturn,
    ThreadLocal,
    Atomic,
    AnonymousAggregates,
    /// Same-type, non-variably-modified typedef redefinition.
    /// C11: §6.7 paragraph 3, p. 108; PDF p. 126.
    TypedefRedefinition,
    C23Keywords,
    Attributes,
    BitInt,
    DecimalTypes,
    EmptyInitializers,
    EnumUnderlyingType,
    AutoTypeInference,
    Constexpr,
    Nullptr,
    WideEnumerators,
    Countof,
    IfSwitchDeclarations,
    NamedLoops,
    GenericTypeOperand,
    /// Inclusive case-label ranges, native in C2y and enabled earlier by GNU
    /// modes. C2y: WG14 N3370, <https://www.open-std.org/JTC1/SC22/WG14/www/docs/n3370.htm>;
    /// extends C99 §6.8.1 paragraph 1, p. 131; PDF p. 143.
    CaseRanges,
    GnuAttribute,
    GnuAsm,
    GnuTypeof,
    ExtensionMarker,
    StatementExpressions,
    BuiltinVaArg,
    BuiltinOffsetof,
    BuiltinTypesCompatible,
    BuiltinChooseExpr,
    LabelsAsValues,
    LocalLabels,
    OmittedConditionalOperand,
    ZeroLengthArrays,
    Int128,
    AutoType,
    GnuAlternateKeywords,
    /// Alignment of an expression rather than a type.
    /// GNU extension: GCC manual, Determining the Alignment of Functions,
    /// Types or Variables, <https://gcc.gnu.org/onlinedocs/gcc/Alignment.html>.
    AlignofExpression,
    /// Returning a void expression from a void function in GNU modes.
    /// GNU extension to C99 §6.8.6.4 paragraph 1, p. 139; PDF p. 151;
    /// GCC c/c-typeck.cc, `c_finish_return`, permits void expression operands.
    VoidExpressionReturn,
    RealImag,
    GnuDesignators,
    UnionCasts,
    EmptyStructs,
    ExtraSemicolons,
    NestedFunctions,
    ImaginaryConstants,
    DollarIdentifiers,
    NamedVariadicMacros,
    GnuVaArgs,
    IncludeNext,
    IdentDirective,
    Counter,
    HasAttribute,
    HasBuiltin,
    SignBitShifts,
    FlexibleArrayExtensions,
    ConstantFolding,
    MsDeclspec,
    MsIntTypes,
    MsCallingConventions,
    MsTypeQualifiers,
    MsInline,
    MsSeh,
    MsAsm,
    MsPragma,
    MsAnonymousStructs,
    MsVaArgs,
    Float128,
    VectorBuiltins,
    /// Evaluated comma in #if, extending C99 §6.6p3, p. 95; PDF p. 107.
    PreprocessorComma,
    /// Macro-generated defined, extending C99 §6.10.1p4, p. 148; PDF p. 160.
    MacroExpandedDefined,
    /// Path backslashes, extending C99 §6.4.7p3, pp. 64-65; PDF pp. 76-77.
    QuotedHeaderBackslash,
    /// C23 §6.9.2p5, p. 160; PDF p. 173 permits unnamed definition parameters.
    UnnamedDefinitionParameters,
}

impl Feature {
    pub(crate) const fn origin(self) -> FeatureOrigin {
        match self {
            | Self::Digraphs => FeatureOrigin::Standard(CStandard::C95),
            | Self::LineComments
            | Self::HexFloats
            | Self::VariadicMacros
            | Self::EmptyMacroArguments
            | Self::Inline
            | Self::Restrict
            | Self::Bool
            | Self::Complex
            | Self::Imaginary
            | Self::MixedDeclarations
            | Self::ArrayParameterSyntax
            | Self::ForDeclarations
            | Self::DesignatedInitializers
            | Self::CompoundLiterals
            | Self::FlexibleArrayMembers
            | Self::LongLong
            | Self::TrailingEnumComma
            | Self::Func => FeatureOrigin::Standard(CStandard::C99),
            | Self::Trigraphs => FeatureOrigin::Standard(CStandard::C89),
            | Self::UnicodeLiteralPrefixes
            | Self::StaticAssert
            | Self::Generic
            | Self::Alignas
            | Self::Alignof
            | Self::Noreturn
            | Self::ThreadLocal
            | Self::Atomic
            | Self::TypedefRedefinition
            | Self::AnonymousAggregates => FeatureOrigin::Standard(CStandard::C11),
            | Self::Utf8CharacterConstants
            | Self::DigitSeparators
            | Self::BitIntSuffixes
            | Self::BinaryConstants
            | Self::Elifdef
            | Self::WarningDirective
            | Self::Embed
            | Self::HasInclude
            | Self::HasEmbed
            | Self::HasCAttribute
            | Self::OmittedVariadicArguments
            | Self::VaOpt
            | Self::ForNonVariableDeclarations
            | Self::C23Keywords
            | Self::Attributes
            | Self::BitInt
            | Self::DecimalTypes
            | Self::EmptyInitializers
            | Self::EnumUnderlyingType
            | Self::AutoTypeInference
            | Self::Constexpr
            | Self::Nullptr
            | Self::UnnamedDefinitionParameters
            | Self::WideEnumerators => FeatureOrigin::Standard(CStandard::C23),

            | Self::OctalPrefix
            | Self::DelimitedEscapes
            | Self::Countof
            | Self::IfSwitchDeclarations
            | Self::NamedLoops
            | Self::GenericTypeOperand
            | Self::CaseRanges => FeatureOrigin::Standard(CStandard::C2y),

            | Self::OldStyleFunctionDeclarators => FeatureOrigin::Removed {
                since:   CStandard::C89,
                removed: CStandard::C23,
            },
            // C99: Foreword paragraph 5, p. xii; PDF p. 10 lists "remove
            // implicit int" among the changes from C89.
            | Self::ImplicitInt | Self::ImplicitFunctionDeclaration => FeatureOrigin::Removed {
                since:   CStandard::C89,
                removed: CStandard::C99,
            },

            | Self::GnuAttribute
            | Self::GnuAsm
            | Self::GnuTypeof
            | Self::ExtensionMarker
            | Self::StatementExpressions
            | Self::BuiltinVaArg
            | Self::BuiltinOffsetof
            | Self::BuiltinTypesCompatible
            | Self::BuiltinChooseExpr
            | Self::LabelsAsValues
            | Self::LocalLabels
            | Self::OmittedConditionalOperand
            | Self::ZeroLengthArrays
            | Self::Int128
            | Self::AutoType
            | Self::VoidExpressionReturn
            | Self::AlignofExpression
            | Self::GnuAlternateKeywords
            | Self::RealImag
            | Self::GnuDesignators
            | Self::UnionCasts
            | Self::EmptyStructs
            | Self::ExtraSemicolons
            | Self::NestedFunctions
            | Self::ImaginaryConstants
            | Self::DollarIdentifiers
            | Self::NamedVariadicMacros
            | Self::GnuVaArgs
            | Self::IncludeNext
            | Self::IdentDirective
            | Self::Counter
            | Self::HasAttribute
            | Self::HasBuiltin
            | Self::SignBitShifts
            | Self::FlexibleArrayExtensions
            | Self::ConstantFolding
            | Self::Float128
            | Self::VectorBuiltins
            | Self::PreprocessorComma
            | Self::MacroExpandedDefined
            | Self::QuotedHeaderBackslash => FeatureOrigin::Gnu,

            | Self::MsDeclspec => FeatureOrigin::Msvc(MsvcFeature::Declspec),
            | Self::MsIntTypes => FeatureOrigin::Msvc(MsvcFeature::IntTypes),
            | Self::MsCallingConventions => FeatureOrigin::Msvc(MsvcFeature::CallingConventions),
            | Self::MsTypeQualifiers => FeatureOrigin::Msvc(MsvcFeature::TypeQualifiers),
            | Self::MsInline => FeatureOrigin::Msvc(MsvcFeature::Inline),
            | Self::MsSeh => FeatureOrigin::Msvc(MsvcFeature::Seh),
            | Self::MsAsm => FeatureOrigin::Msvc(MsvcFeature::Asm),
            | Self::MsPragma => FeatureOrigin::Msvc(MsvcFeature::Pragma),
            | Self::MsAnonymousStructs => FeatureOrigin::Msvc(MsvcFeature::AnonymousStructs),
            | Self::MsVaArgs => FeatureOrigin::Msvc(MsvcFeature::VaArgs),
        }
    }

    pub(super) const fn bit(self) -> u128 {
        1 << self as u8
    }
}

impl Feature {
    pub(crate) const ALL: &'static [Self] = &[
        Self::Digraphs,
        Self::LineComments,
        Self::Trigraphs,
        Self::UnicodeLiteralPrefixes,
        Self::Utf8CharacterConstants,
        Self::DigitSeparators,
        Self::BitIntSuffixes,
        Self::BinaryConstants,
        Self::Elifdef,
        Self::WarningDirective,
        Self::Embed,
        Self::HasInclude,
        Self::HasEmbed,
        Self::HasCAttribute,
        Self::VaOpt,
        Self::OctalPrefix,
        Self::DelimitedEscapes,
        Self::HexFloats,
        Self::VariadicMacros,
        Self::OmittedVariadicArguments,
        Self::EmptyMacroArguments,
        Self::Inline,
        Self::Restrict,
        Self::Bool,
        Self::Complex,
        Self::Imaginary,
        Self::ImplicitInt,
        Self::ImplicitFunctionDeclaration,
        Self::MixedDeclarations,
        Self::ForDeclarations,
        Self::ForNonVariableDeclarations,
        Self::ArrayParameterSyntax,
        Self::DesignatedInitializers,
        Self::CompoundLiterals,
        Self::FlexibleArrayMembers,
        Self::LongLong,
        Self::TrailingEnumComma,
        Self::OldStyleFunctionDeclarators,
        Self::Func,
        Self::StaticAssert,
        Self::Generic,
        Self::Alignas,
        Self::Alignof,
        Self::Noreturn,
        Self::ThreadLocal,
        Self::Atomic,
        Self::AnonymousAggregates,
        Self::TypedefRedefinition,
        Self::C23Keywords,
        Self::Attributes,
        Self::BitInt,
        Self::DecimalTypes,
        Self::EmptyInitializers,
        Self::EnumUnderlyingType,
        Self::AutoTypeInference,
        Self::Constexpr,
        Self::Nullptr,
        Self::WideEnumerators,
        Self::Countof,
        Self::IfSwitchDeclarations,
        Self::NamedLoops,
        Self::GenericTypeOperand,
        Self::CaseRanges,
        Self::GnuAttribute,
        Self::GnuAsm,
        Self::GnuTypeof,
        Self::ExtensionMarker,
        Self::StatementExpressions,
        Self::BuiltinVaArg,
        Self::BuiltinOffsetof,
        Self::BuiltinTypesCompatible,
        Self::BuiltinChooseExpr,
        Self::LabelsAsValues,
        Self::LocalLabels,
        Self::OmittedConditionalOperand,
        Self::ZeroLengthArrays,
        Self::Int128,
        Self::AutoType,
        Self::GnuAlternateKeywords,
        Self::AlignofExpression,
        Self::VoidExpressionReturn,
        Self::RealImag,
        Self::GnuDesignators,
        Self::UnionCasts,
        Self::EmptyStructs,
        Self::ExtraSemicolons,
        Self::NestedFunctions,
        Self::ImaginaryConstants,
        Self::DollarIdentifiers,
        Self::NamedVariadicMacros,
        Self::GnuVaArgs,
        Self::IncludeNext,
        Self::IdentDirective,
        Self::Counter,
        Self::HasAttribute,
        Self::HasBuiltin,
        Self::SignBitShifts,
        Self::FlexibleArrayExtensions,
        Self::ConstantFolding,
        Self::MsDeclspec,
        Self::MsIntTypes,
        Self::MsCallingConventions,
        Self::MsTypeQualifiers,
        Self::MsInline,
        Self::MsSeh,
        Self::MsAsm,
        Self::MsPragma,
        Self::MsAnonymousStructs,
        Self::MsVaArgs,
        Self::Float128,
        Self::VectorBuiltins,
        Self::PreprocessorComma,
        Self::MacroExpandedDefined,
        Self::QuotedHeaderBackslash,
        Self::UnnamedDefinitionParameters,
    ];
}

/// Native origin used for policy diagnostics, independent of acceptance.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum FeatureOrigin {
    Standard(CStandard),
    /// A standard feature that a later revision removed: native from
    /// `since` up to, but not including, `removed`, and an extension after.
    Removed {
        since:   CStandard,
        removed: CStandard,
    },
    Gnu,
    Msvc(MsvcFeature),
}

/// The `msvc` bit recording that the last MSVC umbrella flag enabled the
/// groups, above the `MsvcFeature` bits.
pub(super) const MSVC_COMPATIBILITY: u16 = 1 << 15;
