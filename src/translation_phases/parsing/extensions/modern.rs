//! Syntax of the ISO additions to translation phase 7, parsed by
//! `grammar/modern.rs`.
//!
//! C11: generic selections §6.5.1.1, p. 78; PDF p. 96; static assertions
//! §6.7.10, p. 145; PDF p. 163. C99: the shared type-name and expression
//! children are §6.7.6, p. 122; PDF p. 134 and §6.5, pp. 67-94;
//! PDF pp. 79-106. Evaluation and type constraints belong to later analysis.

pub(crate) use crate::translation_phases::preprocessing::InterchangeFloat;
use crate::{
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::TypeName,
            syntax::Expression,
        },
        preprocessing::{
            KeywordTokenType,
            Token,
        },
    },
    util::arena_list::ArenaList,
};

/// A grammar operand distinguished before semantic analysis.
/// C11: §6.5.1.1, p. 78; PDF p. 96. Type operands in `_Generic` are C2y.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum SyntaxOperand<'tu> {
    Expression(&'tu Expression<'tu>),
    Type(&'tu TypeName<'tu>),
}

/// Parameterized and newly introduced ISO type specifiers.
/// C99: extends the type-specifier seam of §6.7.2, pp. 99-100; PDF pp. 111-112.
/// C11: atomic type specifiers §6.7.2.4 paragraph 1, p. 121; PDF p. 139.
/// C23: typeof specifiers §6.7.3.6 paragraph 1, p. 117; PDF p. 130.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ExtendedType<'tu> {
    Atomic(&'tu TypeName<'tu>),
    Typeof {
        operand:     SyntaxOperand<'tu>,
        unqualified: bool,
    },
    BitInt {
        width:      &'tu Expression<'tu>,
        signedness: Option<bool>,
    },
    Decimal32,
    Decimal64,
    Decimal128,
    Inferred,
    Int128 {
        signedness: Option<bool>,
    },
    AutoType,
    /// MSVC fixed-width integer spelling; target layout belongs to analysis.
    /// MSVC extension: Microsoft Learn, "__int8, __int16, __int32, __int64".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/int8-int16-int32-int64>
    MsInteger {
        width:      u8,
        signedness: Option<bool>,
    },
    Float128 {
        complex: bool,
    },
    /// A binary interchange or extended floating type, real or complex.
    /// C23: §H.5.1 paragraph 1, p. 560; PDF p. 573.
    InterchangeFloat {
        format:  InterchangeFloat,
        complex: bool,
    },
    /// `_Complex` with the integer type specifiers of its real type, in any
    /// order (`_Complex int`, `unsigned short _Complex`, `long _Complex
    /// long`). GNU extension: GCC manual, "Complex Numbers".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Complex.html>
    ComplexInteger(super::super::declaration_syntax::TypeSpecifiers<'tu>),
}

/// Specifier additions collected in reverse source order; immutable links avoid
/// enlarging common nodes. Inspection visits the links in source order. C99:
/// extends declaration-specifiers §6.7, p. 97; PDF p. 109. C11: alignment
/// specifiers §6.7.5 paragraph 1, p. 127; PDF p. 145.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct SpecifierExtension<'tu> {
    pub(crate) kind:           SpecifierExtensionKind<'tu>,
    pub(crate) next:           Option<&'tu Self>,
    pub(crate) source_vectors: SourceVectors,
}

/// Declaration specifier additions from later ISO revisions and vendor
/// dialects.
/// GNU extension: GCC manual, "Attribute Syntax".
/// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
/// MSVC extension: Microsoft Learn, "declspec".
/// <https://learn.microsoft.com/en-us/cpp/cpp/declspec>
/// C11: §6.7.5 paragraph 1, p. 127; PDF p. 145.
/// C11: §6.7.1 paragraph 1, p. 109; PDF p. 127.
/// C23: §6.7.2 paragraph 1, pp. 98-99; PDF pp. 111-112.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum SpecifierExtensionKind<'tu> {
    Alignment(SyntaxOperand<'tu>),
    Attributes(&'tu AttributeSpecifier<'tu>),
    ThreadLocal(ThreadSpelling),
    Constexpr,
    ExtensionMarker,
    /// MSVC declaration modifier, retaining exact spelling and provenance.
    /// MSVC extension: Microsoft Learn, "Microsoft-Specific Modifiers".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/microsoft-specific-modifiers>
    MsModifier(KeywordTokenType),
}

/// How a thread storage-class specifier was written. Every spelling gives
/// thread storage duration, but `__thread` does not combine with the
/// standard ones, and diagnostics name the spelling the source used.
/// C11: §6.7.1 paragraph 1, p. 109; PDF p. 127. C23: `thread_local` is a
/// keyword, §6.4.1 paragraph 1, p. 53; PDF p. 66.
/// GNU extension: GCC manual, "Thread-Local Storage".
/// <https://gcc.gnu.org/onlinedocs/gcc/Thread-Local.html>
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ThreadSpelling {
    /// GNU `__thread`.
    Gnu,
    /// C11 `_Thread_local`.
    Reserved,
    /// C23 `thread_local`.
    Keyword,
}

impl ThreadSpelling {
    /// The specifier as written.
    pub(crate) const fn spelling(self) -> &'static str {
        match self {
            | Self::Gnu => "__thread",
            | Self::Reserved => "_Thread_local",
            | Self::Keyword => "thread_local",
        }
    }

    /// Whether two specifiers are the same one written twice rather than
    /// the GNU and the standard specifier, which do not combine.
    pub(crate) const fn repeats(self, other: Self) -> bool {
        matches!(self, Self::Gnu) == matches!(other, Self::Gnu)
    }
}

/// Shared attribute syntax for ISO, GNU, and MSVC grammar owners.
/// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
/// Balanced tokens retain spelling and provenance. Recognized GNU alignment
/// operands also retain expression syntax for semantic constant evaluation.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct AttributeSpecifier<'tu> {
    pub(crate) syntax:         AttributeSyntax,
    pub(crate) tokens:         &'tu [Token],
    /// Attribute-name token index and its parsed alignment operand.
    pub(crate) arguments:      ArenaList<'tu, AttributeArgument<'tu>>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

/// A recognized attribute's expression operand, associated with its name's
/// position in the retained token list. GNU extension: GCC aligned uses an
/// integer constant expression (C99 §6.6p6, p. 95; PDF p. 107).
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct AttributeArgument<'tu> {
    pub(crate) index:      usize,
    pub(crate) expression: &'tu Expression<'tu>,
}

/// ISO, GNU and Microsoft attribute introducer grammars.
/// GNU extension: GCC manual, "Attribute Syntax".
/// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
/// MSVC extension: Microsoft Learn, "declspec".
/// <https://learn.microsoft.com/en-us/cpp/cpp/declspec>
/// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum AttributeSyntax {
    Standard,
    Gnu,
    Msvc,
}

/// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct GenericSelection<'tu> {
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
    pub(crate) controlling:    SyntaxOperand<'tu>,
    pub(crate) associations:   ArenaList<'tu, GenericAssociation<'tu>>,
}

/// A type-named or default generic association and its result expression.
/// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct GenericAssociation<'tu> {
    pub(crate) type_name:  Option<&'tu TypeName<'tu>>,
    pub(crate) expression: &'tu Expression<'tu>,
}

/// C11: §6.7.10 paragraph 1, p. 145; PDF p. 163. C23 permits no message.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct StaticAssertion<'tu> {
    pub(crate) expression:     &'tu Expression<'tu>,
    pub(crate) message:        Option<Token>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

impl SyntaxOperand<'_> {
    pub(in crate::translation_phases::parsing) fn source(self) -> SourceVectors {
        match self {
            | Self::Expression(x) => x.source_vectors,
            | Self::Type(x) => x.source_vectors,
        }
    }
}
