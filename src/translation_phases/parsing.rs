//! Non-recursive C language parser and its arena-backed syntax model.
//!
//! [`Parser`] is a stack machine: [`ParseFrame`] values own resumable grammar
//! productions, return typed [`ParseValue`] children, and ask the driver to
//! consume, push, reduce, reprocess, or recover through [`ParseAction`]. Each
//! delimiter belongs to one frame. Child frames begin on unconsumed lookahead,
//! and malformed input is synchronized by production-specific sets.
//!
//! Hard syntax diagnostics do not discard useful syntax. If a declaration can
//! be repaired, the parser yields [`ExternalDeclaration::RecoveredDeclaration`]
//! with its arena handle so later semantic analysis can continue. The distinct
//! status prevents repaired syntax from being mistaken for fully valid input.
//!
//! Phase 03 implements declarations, function definitions, compound blocks,
//! every C99 statement family, and the scope transitions needed for
//! typedef-sensitive block-item decisions. Expressions and initializers remain
//! typed deferred children that diagnose and synchronize at their owning
//! grammar boundaries for Phases 04 and 05.
//!
//! Standard references in this module cite WG14/N1256, ISO/IEC 9899:TC3
//! (C99 with Technical Corrigenda 1, 2, and 3). Each reference gives the
//! normative clause, the standard's printed page, and the one-based page in
//! the repository's `c-spec.pdf`.

use std::{
    collections::VecDeque,
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
};

use super::{
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceFileIndex,
    GetSourceVectors,
    SetPosition,
    SetSourceFileIndex,
    SourcePosition,
    SourceVectors,
    TranslationPhase,
    preprocessing::{
        CharacterTokenType,
        FloatTokenType,
        IntegerTokenType,
        KeywordTokenType,
        Preprocessor,
        Token,
        TokenType,
    },
};
use crate::{
    translation_phases::preprocessing::OperatorTokenType,
    util::{
        HashMap,
        HashSet,
        string_cache::StringCacheId,
        vector_slice::{
            UsizeExt,
            VectorSlice,
        },
    },
};

/// Owns parser input, control frames, syntax arenas, scopes, and diagnostics.
///
/// Calling [`TranslationPhase::next_item`] drives the machine until one
/// external declaration reduces or the preprocessed token stream ends.
///
/// C99: translation units and external declarations are specified by §6.9,
/// p. 140; PDF p. 152: a translation unit “consists of a sequence of external
/// declarations.” The diagnostic obligation is §5.1.1.3, p. 11; PDF p. 23.
pub(crate) struct Parser {
    /// Buffered parser-facing token stream.
    cursor: TokenCursor,
    /// Heap-backed grammar control stack; the final element is active.
    frames: Vec<ParseFrame>,
    /// Completed child value waiting for its parent frame.
    returned: Option<ParseValue>,
    /// Arenas owning every syntax node produced by this parser.
    syntax: SyntaxStore,
    /// Parser-visible ordinary-name classification used for typedef ambiguity.
    scopes: ScopeStack,
    /// Function-local label namespaces, independent of ordinary identifiers.
    label_scopes: Vec<LabelScope>,
    /// Active switch contexts used to associate `case` and `default` labels.
    switch_scopes: Vec<SwitchScope>,
    /// Delimiter depth and ownership while a synchronization scan is active.
    recovery: RecoveryState,
    /// Number of hard parser diagnostics emitted so far.
    hard_error_count: usize,
    /// Whether at least one external declaration has reduced successfully or
    /// through recovery.
    has_external_declaration: bool,
    /// Prevents repeated end-of-stream polling from diagnosing an empty
    /// translation unit more than once.
    reported_empty_translation_unit: bool,
    #[cfg(test)]
    /// Driver actions retained only for machine and recovery regressions.
    trace: Vec<FrameTraceEvent>,
}

impl GetPosition for Parser {
    fn position(&self, context: &Context) -> SourcePosition {
        self.cursor.preprocessor.position(context)
    }
}

impl SetPosition for Parser {
    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        self.cursor.preprocessor.set_position(context, position);
    }
}

impl GetSourceFileIndex for Parser {
    fn source_file_index(&self) -> u32 {
        self.cursor.preprocessor.source_file_index()
    }
}

impl SetSourceFileIndex for Parser {
    fn set_source_file_index(&mut self, context: &mut Context, source_file_index: u32) {
        self.cursor
            .preprocessor
            .set_source_file_index(context, source_file_index);
    }
}

/// declaration:
/// - declaration-specifiers init-declarator-list? ;
///
/// C99: §6.7, p. 97; PDF p. 109.
#[derive(Debug, PartialEq, Clone, Copy)]
#[expect(
    clippy::struct_field_names,
    reason = "The C grammar's declaration-specifiers term is the precise field name."
)]
pub(crate) struct Declaration {
    pub(crate) declaration_specifiers: DeclarationSpecifiers,
    /// init-declarator-list
    pub(crate) init_declarators:       VectorSlice<InitDeclarator>,
    pub(crate) source_vectors:         SourceVectors,
    /// This declaration-shaped prefix transferred to a function definition
    /// before a declaration semicolon was consumed.
    is_function_definition_head:       bool,
}

/// init-declarator:
/// - declarator
/// - declarator = initializer
///
/// C99: §6.7, p. 97; PDF p. 109.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct InitDeclarator {
    pub(crate) declarator:     Declarator,
    pub(crate) initializer:    Option<Initializer>,
    pub(crate) source_vectors: SourceVectors,
}

/// initializer:
/// - assignment-expression
/// - { initializer-list }
/// - { initializer-list , }
///
/// C99: §6.7.8, p. 125; PDF p. 137.
#[derive(Debug, PartialEq, Clone)]
#[expect(
    clippy::enum_variant_names,
    reason = "InitializerList is the C grammar production represented by this variant."
)]
pub(crate) enum Initializer {
    #[expect(
        dead_code,
        reason = "The expression frame will construct this variant."
    )]
    AssignmentExpression(ExpressionIndex),
    #[expect(
        dead_code,
        reason = "The initializer frame will construct this variant."
    )]
    InitializerList(VectorSlice<Initializer>),
    FutureChild(SourceVectors),
}

bitflags::bitflags! {
    #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Default)]
    /// type-qualifier:
    /// - const
    /// - restrict
    /// - volatile
    ///
    /// Represents all the type-qualifiers in a declaration. For example, the declaration `const volatile int foo;` would be represented as `TypeQualifiers::CONST | TypeQualifiers::VOLATILE`.
    ///
    /// C99: §6.7.3, p. 108; PDF p. 120.
    pub(crate) struct TypeQualifiers: u8 {
        const CONST = 1 << 0;
        const VOLATILE = 1 << 1;
        const RESTRICT = 1 << 2;
    }
}

/// type-specifier:
/// - void
/// - char
/// - short
/// - int
/// - long
/// - float
/// - double
/// - signed
/// - unsigned
/// - _Bool
/// - _Complex
/// - _Imaginary
/// - struct-or-union-specifier
/// - enum-specifier
/// - typedef-name
///
/// Core variants represent valid combinations of type specifiers. For example,
/// the declaration `unsigned int foo` is represented as
/// `TypeSpecifiers::UnsignedInt`. The `Imaginary` variants retain the existing
/// extension surface; strict C99 mode must diagnose them rather than treating
/// them as normative §6.7.2 alternatives.
///
/// C99: §6.7.2, pp. 99-100; PDF pp. 111-112. `_Imaginary` is listed as a
/// keyword by §6.4.1, p. 50; PDF p. 62, but is not a core type-specifier in
/// the normative §6.7.2 grammar. Its central constraint is: “At least one type
/// specifier shall be given”.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Default)]
pub(crate) enum TypeSpecifiers {
    #[default]
    Empty,
    Void,
    Char,
    SignedChar,
    UnsignedChar,
    Short,
    SignedShort,
    UnsignedShort,
    ShortInt,
    SignedShortInt,
    UnsignedShortInt,
    Int,
    SignedInt,
    UnsignedInt,
    Signed,
    Unsigned,
    Long,
    SignedLong,
    UnsignedLong,
    LongInt,
    SignedLongInt,
    UnsignedLongInt,
    LongLong,
    SignedLongLong,
    UnsignedLongLong,
    LongLongInt,
    SignedLongLongInt,
    UnsignedLongLongInt,
    Float,
    Double,
    LongDouble,
    Bool,
    Complex,
    ComplexFloat,
    ComplexDouble,
    ComplexLong,
    ComplexLongDouble,
    Imaginary,
    ImaginaryFloat,
    ImaginaryDouble,
    ImaginaryLong,
    ImaginaryLongDouble,
    StructOrUnion(StructOrUnionSpecifierIndex),
    Enum(EnumSpecifierIndex),
    TypedefName(Identifier),
}

impl Display for TypeSpecifiers {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | TypeSpecifiers::Empty => write!(f, "<no type specifier>"),
            | TypeSpecifiers::Char => write!(f, "char"),
            | TypeSpecifiers::SignedChar => write!(f, "signed char"),
            | TypeSpecifiers::UnsignedChar => write!(f, "unsigned char"),
            | TypeSpecifiers::Short => write!(f, "short"),
            | TypeSpecifiers::SignedShort => write!(f, "signed short"),
            | TypeSpecifiers::UnsignedShort => write!(f, "unsigned short"),
            | TypeSpecifiers::ShortInt => write!(f, "short int"),
            | TypeSpecifiers::SignedShortInt => write!(f, "signed short int"),
            | TypeSpecifiers::UnsignedShortInt => write!(f, "unsigned short int"),
            | TypeSpecifiers::Int => write!(f, "int"),
            | TypeSpecifiers::SignedInt => write!(f, "signed int"),
            | TypeSpecifiers::UnsignedInt => write!(f, "unsigned int"),
            | TypeSpecifiers::Signed => write!(f, "signed"),
            | TypeSpecifiers::Unsigned => write!(f, "unsigned"),
            | TypeSpecifiers::Long => write!(f, "long"),
            | TypeSpecifiers::SignedLong => write!(f, "signed long"),
            | TypeSpecifiers::UnsignedLong => write!(f, "unsigned long"),
            | TypeSpecifiers::LongInt => write!(f, "long int"),
            | TypeSpecifiers::SignedLongInt => write!(f, "signed long int"),
            | TypeSpecifiers::UnsignedLongInt => write!(f, "unsigned long int"),
            | TypeSpecifiers::LongLong => write!(f, "long long"),
            | TypeSpecifiers::SignedLongLong => write!(f, "signed long long"),
            | TypeSpecifiers::UnsignedLongLong => write!(f, "unsigned long long"),
            | TypeSpecifiers::LongLongInt => write!(f, "long long int"),
            | TypeSpecifiers::SignedLongLongInt => write!(f, "signed long long int"),
            | TypeSpecifiers::UnsignedLongLongInt => write!(f, "unsigned long long int"),
            | TypeSpecifiers::Float => write!(f, "float"),
            | TypeSpecifiers::Double => write!(f, "double"),
            | TypeSpecifiers::LongDouble => write!(f, "long double"),
            | TypeSpecifiers::Bool => write!(f, "_Bool"),
            | TypeSpecifiers::Complex => write!(f, "_Complex"),
            | TypeSpecifiers::ComplexFloat => write!(f, "_Complex float"),
            | TypeSpecifiers::ComplexDouble => write!(f, "_Complex double"),
            | TypeSpecifiers::ComplexLong => write!(f, "_Complex long"),
            | TypeSpecifiers::ComplexLongDouble => write!(f, "_Complex long double"),
            | TypeSpecifiers::Imaginary => write!(f, "_Imaginary"),
            | TypeSpecifiers::ImaginaryFloat => write!(f, "_Imaginary float"),
            | TypeSpecifiers::ImaginaryDouble => write!(f, "_Imaginary double"),
            | TypeSpecifiers::ImaginaryLong => write!(f, "_Imaginary long"),
            | TypeSpecifiers::ImaginaryLongDouble => write!(f, "_Imaginary long double"),
            | TypeSpecifiers::Void => write!(f, "void"),
            | TypeSpecifiers::StructOrUnion(_) => write!(f, "<struct-or-union-declarator>"),
            | TypeSpecifiers::Enum(_) => write!(f, "<enum-declarator>"),
            | TypeSpecifiers::TypedefName(_) => write!(f, "<typedef-name>"),
        }
    }
}

macro_rules! map_fn {
    ($map_fn_name:ident, $make_fn_name:ident, $is_fn_name:ident, $keyword_token_type:ident, $is_match_pattern:pat $(if $is_guard:expr)?, $($map_match_pattern:pat $(if $map_guard:expr)? => $map_result:expr $(,)?)+) => {
        fn $is_fn_name(self) -> bool {
            match self {
                $is_match_pattern $(if $is_guard)? => true,
                | _ => false,
            }
        }

        fn $map_fn_name(
            self,
        ) -> Result<Self, ParserErrorType> {
            match self {
                $($map_match_pattern $(if $map_guard)? => Ok($map_result),)+
                | type_specifier => {
                    Err(ParserErrorType::ConflictingTypeSpecifiers(
                            type_specifier,
                            TokenType::Keyword(KeywordTokenType::$keyword_token_type),
                        ))
                },
            }
        }

        fn $make_fn_name(
            &mut self,
            parser: &mut Parser,
            context: &mut Context,
            token: Token,
        ) {
            match self.$map_fn_name() {
                | Ok(mapped) => *self = mapped,
                | Err(error_type) => parser.report(context, error_type, Some(token)),
            }
        }
    }
}

impl TypeSpecifiers {
    map_fn!(map_signed, make_signed, is_signed, Signed,
        | TypeSpecifiers::SignedChar
        | TypeSpecifiers::SignedShort
        | TypeSpecifiers::SignedShortInt
        | TypeSpecifiers::SignedInt
        | TypeSpecifiers::Signed
        | TypeSpecifiers::SignedLong
        | TypeSpecifiers::SignedLongInt
        | TypeSpecifiers::SignedLongLong
        | TypeSpecifiers::SignedLongLongInt,

        | TypeSpecifiers::Empty => TypeSpecifiers::Signed,
        | TypeSpecifiers::Char => TypeSpecifiers::SignedChar,
        | TypeSpecifiers::Short => TypeSpecifiers::SignedShort,
        | TypeSpecifiers::ShortInt => TypeSpecifiers::SignedShortInt,
        | TypeSpecifiers::Int => TypeSpecifiers::SignedInt,
        | TypeSpecifiers::Long => TypeSpecifiers::SignedLong,
        | TypeSpecifiers::LongInt => TypeSpecifiers::SignedLongInt,
        | TypeSpecifiers::LongLong => TypeSpecifiers::SignedLongLong,
        | TypeSpecifiers::LongLongInt => TypeSpecifiers::SignedLongLongInt,);

    map_fn!(map_unsigned, make_unsigned, is_unsigned, Unsigned,
        | TypeSpecifiers::UnsignedChar
        | TypeSpecifiers::UnsignedShort
        | TypeSpecifiers::UnsignedShortInt
        | TypeSpecifiers::UnsignedInt
        | TypeSpecifiers::Unsigned
        | TypeSpecifiers::UnsignedLong
        | TypeSpecifiers::UnsignedLongInt
        | TypeSpecifiers::UnsignedLongLong
        | TypeSpecifiers::UnsignedLongLongInt,

        | TypeSpecifiers::Empty => TypeSpecifiers::Unsigned,
        | TypeSpecifiers::Char => TypeSpecifiers::UnsignedChar,
        | TypeSpecifiers::Short => TypeSpecifiers::UnsignedShort,
        | TypeSpecifiers::ShortInt => TypeSpecifiers::UnsignedShortInt,
        | TypeSpecifiers::Int => TypeSpecifiers::UnsignedInt,
        | TypeSpecifiers::Long => TypeSpecifiers::UnsignedLong,
        | TypeSpecifiers::LongInt => TypeSpecifiers::UnsignedLongInt,
        | TypeSpecifiers::LongLong => TypeSpecifiers::UnsignedLongLong,
        | TypeSpecifiers::LongLongInt => TypeSpecifiers::UnsignedLongLongInt,
    );

    map_fn!(map_int, make_int, is_int, Int,
        | TypeSpecifiers::Int
        | TypeSpecifiers::SignedInt
        | TypeSpecifiers::UnsignedInt
        | TypeSpecifiers::ShortInt
        | TypeSpecifiers::SignedShortInt
        | TypeSpecifiers::UnsignedShortInt
        | TypeSpecifiers::LongInt
        | TypeSpecifiers::SignedLongInt
        | TypeSpecifiers::UnsignedLongInt
        | TypeSpecifiers::LongLongInt
        | TypeSpecifiers::SignedLongLongInt
        | TypeSpecifiers::UnsignedLongLongInt,

        | TypeSpecifiers::Empty => TypeSpecifiers::Int,
        | TypeSpecifiers::Short => TypeSpecifiers::ShortInt,
        | TypeSpecifiers::Signed => TypeSpecifiers::SignedInt,
        | TypeSpecifiers::Unsigned => TypeSpecifiers::UnsignedInt,
        | TypeSpecifiers::Long => TypeSpecifiers::LongInt,
        | TypeSpecifiers::SignedLong => TypeSpecifiers::SignedLongInt,
        | TypeSpecifiers::UnsignedLong => TypeSpecifiers::UnsignedLongInt,
        | TypeSpecifiers::LongLong => TypeSpecifiers::LongLongInt,
        | TypeSpecifiers::SignedLongLong => TypeSpecifiers::SignedLongLongInt,
        | TypeSpecifiers::UnsignedLongLong => TypeSpecifiers::UnsignedLongLongInt,
    );

    map_fn!(map_short, make_short, is_short, Short,
        | TypeSpecifiers::Short
        | TypeSpecifiers::SignedShort
        | TypeSpecifiers::UnsignedShort
        | TypeSpecifiers::ShortInt
        | TypeSpecifiers::SignedShortInt
        | TypeSpecifiers::UnsignedShortInt,

        | TypeSpecifiers::Empty => TypeSpecifiers::Short,
        | TypeSpecifiers::Int => TypeSpecifiers::ShortInt,
        | TypeSpecifiers::Signed => TypeSpecifiers::SignedShort,
        | TypeSpecifiers::Unsigned => TypeSpecifiers::UnsignedShort,
    );

    map_fn!(map_long, make_long, is_long, Long,
        | TypeSpecifiers::Long
        | TypeSpecifiers::SignedLong
        | TypeSpecifiers::UnsignedLong
        | TypeSpecifiers::LongInt
        | TypeSpecifiers::SignedLongInt
        | TypeSpecifiers::UnsignedLongInt
        | TypeSpecifiers::LongLong
        | TypeSpecifiers::SignedLongLong
        | TypeSpecifiers::UnsignedLongLong
        | TypeSpecifiers::LongLongInt
        | TypeSpecifiers::SignedLongLongInt
        | TypeSpecifiers::UnsignedLongLongInt
        | TypeSpecifiers::LongDouble
        | TypeSpecifiers::ComplexLong
        | TypeSpecifiers::ComplexLongDouble
        | TypeSpecifiers::ImaginaryLong
        | TypeSpecifiers::ImaginaryLongDouble,

        | TypeSpecifiers::Empty => TypeSpecifiers::Long,
        | TypeSpecifiers::Int => TypeSpecifiers::LongInt,
        | TypeSpecifiers::Signed => TypeSpecifiers::SignedLong,
        | TypeSpecifiers::Unsigned => TypeSpecifiers::UnsignedLong,
        | TypeSpecifiers::Long => TypeSpecifiers::LongLong,
        | TypeSpecifiers::SignedLong => TypeSpecifiers::SignedLongLong,
        | TypeSpecifiers::UnsignedLong => TypeSpecifiers::UnsignedLongLong,
        | TypeSpecifiers::LongInt => TypeSpecifiers::LongLongInt,
        | TypeSpecifiers::SignedLongInt => TypeSpecifiers::SignedLongLongInt,
        | TypeSpecifiers::UnsignedLongInt => TypeSpecifiers::UnsignedLongLongInt,
        | TypeSpecifiers::Double => TypeSpecifiers::LongDouble,
        | TypeSpecifiers::Complex => TypeSpecifiers::ComplexLong,
        | TypeSpecifiers::ComplexDouble => TypeSpecifiers::ComplexLongDouble,
        | TypeSpecifiers::Imaginary => TypeSpecifiers::ImaginaryLong,
        | TypeSpecifiers::ImaginaryDouble => TypeSpecifiers::ImaginaryLongDouble,
    );

    map_fn!(map_char, make_char, is_char, Char,
        | TypeSpecifiers::Char
        | TypeSpecifiers::SignedChar
        | TypeSpecifiers::UnsignedChar,

        | TypeSpecifiers::Empty => TypeSpecifiers::Char,
        | TypeSpecifiers::Signed => TypeSpecifiers::SignedChar,
        | TypeSpecifiers::Unsigned => TypeSpecifiers::UnsignedChar,
    );

    map_fn!(map_float, make_float, is_float, Float,
        | TypeSpecifiers::Float
        | TypeSpecifiers::ComplexFloat
        | TypeSpecifiers::ImaginaryFloat,

        | TypeSpecifiers::Empty => TypeSpecifiers::Float,
        | TypeSpecifiers::Complex => TypeSpecifiers::ComplexFloat,
        | TypeSpecifiers::Imaginary => TypeSpecifiers::ImaginaryFloat,
    );

    map_fn!(map_double, make_double, is_double, Double,
        | TypeSpecifiers::ComplexDouble
        | TypeSpecifiers::ComplexLongDouble
        | TypeSpecifiers::ImaginaryDouble
        | TypeSpecifiers::ImaginaryLongDouble
        | TypeSpecifiers::LongDouble,

        | TypeSpecifiers::Empty => TypeSpecifiers::Double,
        | TypeSpecifiers::Complex => TypeSpecifiers::ComplexDouble,
        | TypeSpecifiers::ComplexLong => TypeSpecifiers::ComplexLongDouble,
        | TypeSpecifiers::Imaginary => TypeSpecifiers::ImaginaryDouble,
        | TypeSpecifiers::ImaginaryLong => TypeSpecifiers::ImaginaryLongDouble,
        | TypeSpecifiers::Long => TypeSpecifiers::LongDouble
    );

    map_fn!(map_void, make_void, is_void, Void,
        | TypeSpecifiers::Void,

        | TypeSpecifiers::Empty => TypeSpecifiers::Void,
    );

    map_fn!(map_bool, make_bool, is_bool, Bool,
        | TypeSpecifiers::Bool,

        | TypeSpecifiers::Empty => TypeSpecifiers::Bool,
    );

    map_fn!(map_complex, make_complex, is_complex, Complex,
        | TypeSpecifiers::Complex
        | TypeSpecifiers::ComplexFloat
        | TypeSpecifiers::ComplexDouble
        | TypeSpecifiers::ComplexLong
        | TypeSpecifiers::ComplexLongDouble,

        | TypeSpecifiers::Empty => TypeSpecifiers::Complex,
        | TypeSpecifiers::Float => TypeSpecifiers::ComplexFloat,
        | TypeSpecifiers::Double => TypeSpecifiers::ComplexDouble,
        | TypeSpecifiers::Long => TypeSpecifiers::ComplexLong,
        | TypeSpecifiers::LongDouble => TypeSpecifiers::ComplexLongDouble,
    );

    map_fn!(map_imaginary, make_imaginary, is_imaginary, Imaginary,
        | TypeSpecifiers::Imaginary
        | TypeSpecifiers::ImaginaryFloat
        | TypeSpecifiers::ImaginaryDouble
        | TypeSpecifiers::ImaginaryLong
        | TypeSpecifiers::ImaginaryLongDouble,

        | TypeSpecifiers::Empty => TypeSpecifiers::Imaginary,
        | TypeSpecifiers::Float => TypeSpecifiers::ImaginaryFloat,
        | TypeSpecifiers::Double => TypeSpecifiers::ImaginaryDouble,
        | TypeSpecifiers::Long => TypeSpecifiers::ImaginaryLong,
        | TypeSpecifiers::LongDouble => TypeSpecifiers::ImaginaryLongDouble,
    );

    fn is_long_long(self) -> bool {
        match self {
            | TypeSpecifiers::LongLong
            | TypeSpecifiers::SignedLongLong
            | TypeSpecifiers::UnsignedLongLong
            | TypeSpecifiers::LongLongInt
            | TypeSpecifiers::SignedLongLongInt
            | TypeSpecifiers::UnsignedLongLongInt => true,
            | _ => false,
        }
    }

    fn is_long_double(self) -> bool {
        match self {
            | TypeSpecifiers::LongDouble
            | TypeSpecifiers::ComplexLongDouble
            | TypeSpecifiers::ImaginaryLongDouble => true,
            | _ => false,
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Retained for the future semantic type builder.")
    )]
    fn is_struct_or_union(self) -> bool {
        match self {
            | TypeSpecifiers::StructOrUnion(_) => true,
            | _ => false,
        }
    }

    fn map_struct_or_union(
        self,
        index: StructOrUnionSpecifierIndex,
        token: Token,
    ) -> Result<Self, ParserErrorType> {
        match self {
            | TypeSpecifiers::Empty => Ok(TypeSpecifiers::StructOrUnion(index)),
            | type_specifiers => Err(ParserErrorType::ConflictingTypeSpecifiers(
                type_specifiers,
                token.kind,
            )),
        }
    }

    fn make_struct_or_union(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        index: StructOrUnionSpecifierIndex,
        token: Token,
    ) {
        match self.map_struct_or_union(index, token) {
            | Ok(mapped) => *self = mapped,
            | Err(error_type) => parser.report(context, error_type, Some(token)),
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Retained for the future semantic type builder.")
    )]
    fn is_enum(self) -> bool {
        match self {
            | TypeSpecifiers::Enum(_) => true,
            | _ => false,
        }
    }

    fn map_enum(self, index: EnumSpecifierIndex) -> Result<Self, ParserErrorType> {
        match self {
            | TypeSpecifiers::Empty => Ok(TypeSpecifiers::Enum(index)),
            | type_specifiers => Err(ParserErrorType::ConflictingTypeSpecifiers(
                type_specifiers,
                TokenType::Keyword(KeywordTokenType::Enum),
            )),
        }
    }

    fn make_enum(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        index: EnumSpecifierIndex,
        token: Token,
    ) {
        match self.map_enum(index) {
            | Ok(mapped) => *self = mapped,
            | Err(error_type) => parser.report(context, error_type, Some(token)),
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Retained for the future semantic type builder.")
    )]
    fn is_typedef_name(self) -> bool {
        match self {
            | TypeSpecifiers::TypedefName(_) => true,
            | _ => false,
        }
    }

    fn map_typedef_name(self, name: Identifier) -> Result<Self, ParserErrorType> {
        match self {
            | TypeSpecifiers::Empty => Ok(TypeSpecifiers::TypedefName(name)),
            | type_specifiers => Err(ParserErrorType::ConflictingTypeSpecifiers(
                type_specifiers,
                TokenType::Identifier,
            )),
        }
    }

    fn make_typedef_name(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        name: Identifier,
        token: Token,
    ) {
        match self.map_typedef_name(name) {
            | Ok(mapped) => *self = mapped,
            | Err(error_type) => parser.report(context, error_type, Some(token)),
        }
    }
}

/// struct-or-union-specifier:
/// - struct-or-union identifier? { struct-declaration-list }
/// - struct-or-union identifier
///
/// C99: §6.7.2.1, p. 101; PDF p. 113.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct StructOrUnionSpecifier {
    pub(crate) struct_or_union:         StructOrUnion,
    pub(crate) identifier:              Option<Identifier>,
    /// None indicates that the body is missing. An empty vector indicates an
    /// empty body. `struct Foo;` has no body. `struct Foo {};` has an empty
    /// body.
    pub(crate) struct_declaration_list: Option<VectorSlice<StructDeclaration>>,
    pub(crate) source_vectors:          SourceVectors,
}

/// struct-or-union:
/// - struct
/// - union
///
/// C99: §6.7.2.1, p. 101; PDF p. 113.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum StructOrUnion {
    Struct,
    Union,
}

/// struct-declaration:
/// - specifier-qualifier-list struct-declarator-list ;
///
/// `type_qualifiers` and `type_specifiers` are split into two fields.
///
/// C99: §6.7.2.1, p. 101; PDF p. 113.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct StructDeclaration {
    type_qualifiers:        TypeQualifiers,
    type_specifiers:        TypeSpecifiers,
    struct_declarator_list: VectorSlice<StructDeclarator>,
    source_vectors:         SourceVectors,
}

/// struct-declarator:
/// - declarator
/// - declarator? : constant-expression
///
/// C99: §6.7.2.1, p. 101; PDF p. 113.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct StructDeclarator {
    declarator:     Option<Declarator>,
    bitfield_width: Option<ConstantExpressionIndex>,
    source_vectors: SourceVectors,
}

/// enum-specifier:
/// - enum identifier? { enumerator-list }
/// - enum identifier? { enumerator-list , }
/// - enum identifier
///
/// C99: §6.7.2.2, p. 105; PDF p. 117.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct EnumSpecifier {
    pub(crate) name:             Option<Identifier>,
    pub(crate) enumeration_list: Option<VectorSlice<Enumerator>>,
    pub(crate) source_vectors:   SourceVectors,
}

/// enumerator:
/// - enumeration-constant
/// - enumeration-constant = constant-expression
///
/// C99: §6.7.2.2, p. 105; PDF p. 117.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Enumerator {
    pub(crate) name:           Identifier,
    pub(crate) expression:     Option<ConstantExpressionIndex>,
    pub(crate) source_vectors: SourceVectors,
}

/// function-specifier:
/// - inline
///
/// C99: §6.7.4, p. 113; PDF p. 125.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Default)]
pub(crate) struct FunctionSpecifiers {
    pub(crate) is_inline: bool,
}

/// declaration-specifiers:
/// - storage-class-specifier declaration-specifiers?
/// - type-specifier declaration-specifiers?
/// - type-qualifier declaration-specifiers?
/// - function-specifier declaration-specifiers?
///
/// each field in this struct contains all the specifiers of that type. For
/// example, the `type_qualifiers` field contains all the type qualifiers in the
/// declaration.
///
/// C99: §6.7, p. 97; PDF p. 109. The constituent specifier families are
/// specified by §6.7.1-§6.7.4, pp. 98-113; PDF pp. 110-125.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct DeclarationSpecifiers {
    pub(crate) storage_class:       StorageClass,
    pub(crate) type_qualifiers:     TypeQualifiers,
    pub(crate) type_specifiers:     TypeSpecifiers,
    pub(crate) function_specifiers: FunctionSpecifiers,
    pub(crate) source_vectors:      SourceVectors,
}

impl Default for DeclarationSpecifiers {
    fn default() -> Self {
        Self::new()
    }
}

impl DeclarationSpecifiers {
    fn new() -> Self {
        Self {
            storage_class:       StorageClass::Auto,
            type_qualifiers:     TypeQualifiers::empty(),
            type_specifiers:     TypeSpecifiers::Empty,
            function_specifiers: FunctionSpecifiers { is_inline: false },
            source_vectors:      VectorSlice::empty(),
        }
    }
}

/// pointer:
/// - type-qualifier-list?
/// - type-qualifier-list? pointer
///
/// Each element in the `type_qualifiers_list` represents the type qualifiers
/// for one level of indirection. For example, this declaration: `*const
/// *volatile *x` would be parsed as: `[TypeQualifiers::CONST,
/// TypeQualifiers::VOLATILE, TypeQualifiers::empty()]`
///
/// C99: §6.7.5, p. 114; PDF p. 126, and pointer derivation §6.7.5.1,
/// p. 115; PDF p. 127.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct PointerDeclarator {
    /// Each element represents the type qualifiers for one level of
    /// indirection.
    pub(crate) type_qualifiers_list: VectorSlice<TypeQualifiers>,
}

/// declarator:
/// - pointer? direct-declarator
///
/// abstract-declarator:
/// - pointer
/// - pointer? direct-abstract-declarator
///
/// Represents both declarators and abstract declarators.
///
/// C99: declarators are §6.7.5, p. 114; PDF p. 126. Abstract declarators are
/// §6.7.6, p. 122; PDF p. 134.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Declarator {
    pub(crate) pointer:        PointerDeclarator,
    pub(crate) kind:           VectorSlice<DirectDeclarator>,
    pub(crate) source_vectors: SourceVectors,
}

/// direct-declarator:
/// - identifier
/// - ( declarator )
/// - direct-declarator [ type-qualifier-list? assignment-expression? ]
/// - direct-declarator [ static type-qualifier-list? assignment-expression ]
/// - direct-declarator [ type-qualifier-list static assignment-expression ]
/// - direct-declarator [ type-qualifier-list? * ]
/// - direct-declarator ( parameter-type-list )
/// - direct-declarator ( identifier-list? )
///
/// direct-abstract-declarator:
/// - ( abstract-declarator )
/// - direct-abstract-declarator? [ type-qualifier-list? assignment-expression?
///   ]
/// - direct-abstract-declarator? [ static type-qualifier-list?
///   assignment-expression ]
/// - direct-abstract-declarator? [ type-qualifier-list static
///   assignment-expression ]
/// - direct-abstract-declarator? \[ * \]
/// - direct-abstract-declarator? ( parameter-type-list? )
///
/// Represents both direct-declarators and direct-abstract-declarators.
///
/// C99: direct declarators are §6.7.5, p. 114; PDF p. 126, with array and
/// function derivation in §6.7.5.2-§6.7.5.3, pp. 116-121; PDF pp. 128-133.
/// Direct abstract declarators are §6.7.6, p. 122; PDF p. 134.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum DirectDeclarator {
    Identifier(Identifier),
    Parenthesized(Declarator),
    KAndRStyleFunction {
        parameters: VectorSlice<Identifier>,
    },
    Array {
        type_qualifiers:       TypeQualifiers,
        is_static:             bool,
        is_pointer:            bool,
        assignment_expression: Option<ExpressionIndex>,
    },
    Function {
        parameter_list: VectorSlice<ParameterDeclaration>,
        is_variadic:    bool,
    },
}

/// parameter-declaration:
/// - declaration-specifiers declarator
/// - declaration-specifiers abstract-declarator?
///
/// C99: §6.7.5, p. 114; PDF p. 126, and function declarators §6.7.5.3,
/// pp. 118-121; PDF pp. 130-133.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ParameterDeclaration {
    declaration_specifiers: DeclarationSpecifiers,
    /// Could be a declarator or an abstract declarator or neither.
    declarator:             Option<Declarator>,
    source_vectors:         SourceVectors,
}

// The Phase 03 parser machine is implemented below the retained syntax model.

/// A type-name syntax node retained for the future type-name frame.
///
/// C99: §6.7.6, p. 122; PDF p. 134.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TypeName {
    /// Specifiers and qualifiers that establish the base type.
    pub(crate) declaration_specifiers: DeclarationSpecifiers,
    /// Optional abstract declarator deriving pointer, array, or function shape.
    pub(crate) declarator:             Option<Declarator>,
}

/// Buffered adapter from the preprocessor's iterator interface to parser
/// current-token and arbitrary-lookahead operations.
///
/// `consume` advances exactly one token. Lookahead never changes `current`, and
/// EOF is memoized so the preprocessor is not polled after completion.
///
/// C99: this cursor consumes the phase-7 token stream described by §5.1.1.2,
/// phases 6-7, pp. 9-10; PDF pp. 21-22. Token categories are specified by
/// §6.4, pp. 49-50; PDF pp. 61-62.
struct TokenCursor {
    /// Upstream producer of parser-facing tokens.
    preprocessor: Preprocessor,
    /// Token currently owned by the active parser frame.
    current:      Option<Token>,
    /// Tokens fetched beyond `current`, ordered nearest first.
    lookahead:    VecDeque<Token>,
    /// Whether the upstream preprocessor has returned EOF.
    reached_eof:  bool,
}

impl TokenCursor {
    /// Creates an empty cursor over `preprocessor`; no token is fetched
    /// eagerly.
    fn new(preprocessor: Preprocessor) -> Self {
        Self {
            preprocessor,
            current: None,
            lookahead: VecDeque::new(),
            reached_eof: false,
        }
    }

    /// Returns the current token, fetching it once if necessary.
    fn current(&mut self, context: &mut Context) -> Option<Token> {
        if self.current.is_none() && !self.reached_eof {
            self.current = self.preprocessor.next_item(context);
            self.reached_eof = self.current.is_none();
        }
        self.current
    }

    /// Returns the token immediately following `current` without consuming.
    fn following(&mut self, context: &mut Context) -> Option<Token> {
        self.lookahead(context, 0)
    }

    /// Returns zero-based lookahead beyond `current` without consuming.
    fn lookahead(&mut self, context: &mut Context, index: usize) -> Option<Token> {
        let _ = self.current(context)?;
        while self.lookahead.len() <= index && !self.reached_eof {
            let next = self.preprocessor.next_item(context);
            self.reached_eof = next.is_none();
            if let Some(next) = next {
                self.lookahead.push_back(next);
            }
        }
        self.lookahead.get(index).copied()
    }

    /// Advances by one token while preserving any buffered lookahead.
    fn consume(&mut self) {
        debug_assert!(self.current.is_some(), "cannot consume parser EOF");
        self.current = self.lookahead.pop_front();
    }
}

/// Owns arena storage for every syntax domain constructed by parser frames.
///
/// AST nodes refer to these vectors through typed handles and [`VectorSlice`]
/// ranges, keeping nested syntax compact and avoiding recursive ownership.
///
/// C99: the stored language syntax spans expressions through external
/// definitions, §6.5-§6.9, pp. 67-144; PDF pp. 79-156. Arena storage is an
/// implementation strategy, not a normative C concept.
#[derive(Debug, Default)]
struct SyntaxStore {
    #[expect(dead_code, reason = "Owned by the future type-name frame.")]
    type_names:                 Vec<TypeName>,
    declarations:               Vec<Declaration>,
    init_declarators:           Vec<InitDeclarator>,
    expressions:                Vec<Expression>,
    #[expect(dead_code, reason = "Owned by the future expression frame.")]
    expression_indices:         Vec<ExpressionIndex>,
    statements:                 Vec<Statement>,
    block_items:                Vec<BlockItem>,
    function_definitions:       Vec<FunctionDefinition>,
    declaration_indices:        Vec<DeclarationIndex>,
    type_qualifiers:            Vec<TypeQualifiers>,
    direct_declarators:         Vec<DirectDeclarator>,
    identifiers:                Vec<Identifier>,
    parameter_declarations:     Vec<ParameterDeclaration>,
    struct_or_union_specifiers: Vec<StructOrUnionSpecifier>,
    struct_declarations:        Vec<StructDeclaration>,
    struct_declarators:         Vec<StructDeclarator>,
    enum_specifiers:            Vec<EnumSpecifier>,
    enumerators:                Vec<Enumerator>,
}

/// Parser-visible classification in C's ordinary-identifier namespace.
///
/// C99: scopes and namespaces are §6.2.1-§6.2.3, pp. 29-31; PDF pp. 41-43.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum NameClass {
    /// Identifier currently denotes a typedef name.
    Typedef,
    /// Identifier denotes an object, function, parameter, or enumerator.
    Ordinary,
}

/// Kind of parser-visible scope whose lifetime is owned by one frame.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum ScopeKind {
    Function,
    FunctionPrototype,
    Block,
    ImplicitSelection,
    ImplicitIteration,
}

#[derive(Default)]
struct Scope {
    kind:     Option<ScopeKind>,
    bindings: HashMap<StringCacheId, NameClass>,
}

/// File, function, prototype, block, and implicit statement scopes used for
/// typedef-sensitive grammar choices.
///
/// C99: identifier scopes are §6.2.1, pp. 29-30; PDF pp. 41-42; distinct
/// namespaces are §6.2.3, p. 31; PDF p. 43. Function-prototype scope ends at
/// the function declarator under §6.2.1 paragraph 4, p. 30; PDF p. 42.
#[derive(Default)]
struct ScopeStack {
    /// Bindings visible for the translation unit.
    file_scope:    HashMap<StringCacheId, NameClass>,
    /// Nested scopes, with the innermost scope last.
    nested_scopes: Vec<Scope>,
    #[cfg(test)]
    trace:         Vec<ScopeTraceEvent>,
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct ScopeTraceEvent {
    kind:  ScopeKind,
    enter: bool,
}

impl ScopeStack {
    /// Tests the innermost visible ordinary-name binding for typedef status.
    fn is_typedef(&self, name: StringCacheId) -> bool {
        self.nested_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.bindings.get(&name))
            .or_else(|| self.file_scope.get(&name))
            == Some(&NameClass::Typedef)
    }

    /// Publishes a binding in the innermost active scope.
    fn publish(&mut self, name: StringCacheId, class: NameClass) {
        if let Some(scope) = self.nested_scopes.last_mut() {
            _ = scope.bindings.insert(name, class);
        } else {
            _ = self.file_scope.insert(name, class);
        }
    }

    fn depth(&self) -> usize {
        self.nested_scopes.len()
    }

    /// Opens a nested scope with an explicit grammar lifetime.
    fn enter_scope(&mut self, kind: ScopeKind) {
        #[cfg(test)]
        self.trace.push(ScopeTraceEvent { kind, enter: true });
        self.nested_scopes.push(Scope {
            kind:     Some(kind),
            bindings: HashMap::default(),
        });
    }

    /// Restores exactly the depth recorded by the owning frame.
    fn restore_depth(&mut self, depth: usize) {
        debug_assert!(
            depth <= self.nested_scopes.len(),
            "a frame cannot restore below the scope depth at which it started"
        );
        while self.nested_scopes.len() > depth {
            let scope = self.nested_scopes.pop().expect("scope depth was checked");
            let kind = scope.kind.expect("nested scopes have a kind");
            #[cfg(test)]
            self.trace.push(ScopeTraceEvent { kind, enter: false });
            #[cfg(not(test))]
            let _ = kind;
        }
    }
}

#[derive(Debug, Default)]
struct LabelScope {
    definitions: HashSet<StringCacheId>,
    references:  HashSet<StringCacheId>,
}

#[derive(Debug, Default)]
struct SwitchScope {
    has_default: bool,
}

/// Stable identity for a frame family, used by traces, recovery, and internal
/// invariant diagnostics instead of free-form strings.
///
/// C99: frame families partition the clause-6 grammar described using the
/// notation of §6.1, p. 29; PDF p. 41. Frame identity itself is an
/// implementation mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseFrameKind {
    /// Translation-unit entry for one external declaration.
    ExternalDeclaration,
    /// Declaration shell and init-declarator list.
    Declaration,
    /// Declaration-specifier or specifier-qualifier sequence.
    DeclarationSpecifiers,
    /// Named, abstract, or maybe-abstract declarator.
    Declarator,
    /// Prototype or K&R parameter list.
    ParameterList,
    /// Struct or union tag specifier and optional member body.
    StructOrUnionSpecifier,
    /// Enum tag specifier and optional enumerator body.
    EnumSpecifier,
    /// Function definition continuation after a completed declaration head.
    FunctionDefinition,
    /// Brace-delimited ordered block-item sequence.
    CompoundStatement,
    /// One labeled, compound, expression, selection, iteration, or jump
    /// statement.
    Statement,
    /// Typed placeholder for a grammar child implemented in a later phase.
    FutureChild(FutureChildKind),
}

impl ParseFrameKind {
    /// Returns the stable kebab-case label used in traces and diagnostics.
    fn label(self) -> &'static str {
        match self {
            | Self::ExternalDeclaration => "external-declaration",
            | Self::Declaration => "declaration",
            | Self::DeclarationSpecifiers => "declaration-specifiers",
            | Self::Declarator => "declarator",
            | Self::ParameterList => "parameter-list",
            | Self::StructOrUnionSpecifier => "struct-or-union-specifier",
            | Self::EnumSpecifier => "enum-specifier",
            | Self::FunctionDefinition => "function-definition",
            | Self::CompoundStatement => "compound-statement",
            | Self::Statement => "statement",
            | Self::FutureChild(kind) => kind.label(),
        }
    }
}

/// Instruction returned by the active frame to the parser driver.
///
/// C99: these actions implement the §6.1 grammar notation without Rust call
/// recursion and support the translation-limit requirements of §5.2.4.1,
/// pp. 20-21; PDF pp. 32-33.
#[derive(Debug)]
enum ParseAction {
    /// Consume the current token and keep the active frame.
    Consume,
    /// Suspend the active frame and push an unstarted child.
    Push(ParseFrame),
    /// Complete the active frame and return a typed value to its parent.
    Reduce(ParseValue),
    /// Keep the current token and run the active frame again after a state
    /// change.
    Reprocess,
    /// Scan to a production-specific boundary before resuming the active frame.
    Recover(SynchronizationSet),
}

/// Typed value returned by a completed child frame.
///
/// C99: values correspond to completed nonterminals from §6.7-§6.9,
/// pp. 97-144; PDF pp. 109-156. Typed returns are an implementation mechanism.
#[derive(Debug, Clone, Copy)]
enum ParseValue {
    /// Result of a declaration-specifier child.
    DeclarationSpecifiers(DeclarationSpecifiers),
    /// Declarator result; `None` records a recoverable missing declarator.
    Declarator(Option<Declarator>),
    /// Completed function or K&R parameter-list suffix.
    ParameterList(ParameterListResult),
    /// Arena handle for a completed struct or union specifier.
    StructOrUnionSpecifier(StructOrUnionSpecifierIndex),
    /// Completed enum specifier and its recovery handoff.
    EnumSpecifier(EnumSpecifierResult),
    /// Arena handle for a completed declaration.
    Declaration(DeclarationIndex),
    FunctionDefinition(FunctionDefinitionIndex),
    CompoundStatement(StatementIndex),
    Statement(StatementIndex),
    /// External item ready to be yielded by the translation-phase seam.
    ExternalDeclaration(ExternalDeclaration),
    /// Recovered placeholder for a child implemented in a later phase.
    FutureChild(FutureChildResult),
}

/// Parameter-list child result before it is appended to a declarator frame.
///
/// C99: parameter-type-list, parameter-list, and identifier-list are specified
/// by §6.7.5, p. 114; PDF p. 126, with semantics in §6.7.5.3,
/// pp. 118-121; PDF pp. 130-133.
#[derive(Debug, Clone, Copy)]
struct ParameterListResult {
    /// Function suffix constructed from the parsed list.
    direct_declarator: DirectDeclarator,
    /// Exact source provenance owned by the parameter-list frame.
    source_vectors:    SourceVectors,
}

/// Enum child result before its type specifier is merged into the parent.
#[derive(Debug, Clone, Copy)]
struct EnumSpecifierResult {
    /// Arena handle for the completed enum specifier.
    index: EnumSpecifierIndex,
    /// Whether recovery stopped before a following declaration.
    stopped_before_declaration: bool,
}

/// Result of diagnosing and synchronizing a deferred grammar child.
///
/// C99: a syntax violation requires a diagnostic under §5.1.1.3, p. 11; PDF
/// p. 23. Synchronization and placeholders are implementation mechanisms.
#[derive(Debug, Clone, Copy)]
struct FutureChildResult {
    /// Deferred grammar family that was encountered.
    kind:           FutureChildKind,
    /// Source consumed while synchronizing that child.
    source_vectors: SourceVectors,
}

/// Sum type for every grammar frame currently implemented by the parser.
///
/// C99: the represented grammar families currently cover declarations through
/// external definitions, §6.7-§6.9, pp. 97-144; PDF pp. 109-156.
#[derive(Debug)]
enum ParseFrame {
    ExternalDeclaration(ExternalDeclarationFrame),
    Declaration(DeclarationFrame),
    DeclarationSpecifiers(DeclarationSpecifiersFrame),
    Declarator(DeclaratorFrame),
    ParameterList(ParameterListFrame),
    StructOrUnionSpecifier(StructOrUnionSpecifierFrame),
    EnumSpecifier(EnumSpecifierFrame),
    FunctionDefinition(FunctionDefinitionFrame),
    CompoundStatement(CompoundStatementFrame),
    Statement(StatementFrame),
    FutureChild(FutureChildFrame),
}

/// One driver response paired with the identity of the frame that produced it.
///
/// C99: this is an implementation of the grammar notation in §6.1, p. 29;
/// PDF p. 41, not a normative C data type.
struct FrameStep {
    frame_kind: ParseFrameKind,
    action:     ParseAction,
}

/// A production-specific synchronization policy and the frame allowed to
/// resume after the scan.
///
/// C99: recovery serves the diagnostic requirement in §5.1.1.3, p. 11; PDF
/// p. 23. The standard does not prescribe synchronization sets.
#[derive(Debug, Clone, Copy)]
struct SynchronizationSet {
    kind:   SynchronizationKind,
    target: ParseFrameKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpressionTerminator {
    Semicolon,
    ForSemicolon,
    ClosingParenthesis,
    Colon,
}

/// Selects the grammar-specific boundary rules used during recovery.
///
/// C99: boundaries are derived from the productions in §6.7-§6.9,
/// pp. 97-144; PDF pp. 109-156; recovery itself is implementation-defined.
#[derive(Debug, Clone, Copy)]
enum SynchronizationKind {
    /// Stop before a declarator separator, terminator, or enclosing brace.
    Declaration,
    /// Stop before a block statement keyword as well as declaration boundaries.
    BlockDeclaration,
    /// Stop before the function body of an old-style definition.
    OldStyleParameter,
    /// Stop before a `for` header's closing parenthesis as well as declaration
    /// separators.
    ForInitializer,
    /// Stop before an initializer separator or declaration boundary.
    Initializer,
    /// Stop before the owning `]` or an enclosing declaration boundary.
    ArrayBound,
    /// Stop before a prototype parameter separator or enclosing boundary.
    Parameter,
    /// Stop before an identifier-list separator or enclosing boundary.
    KAndRParameter,
    /// Stop before the `)` that must follow `...` or an enclosing boundary.
    VariadicParameterList,
    /// Consume a comma-introduced parameter that illegally follows `...`.
    VariadicTrailingParameter,
    /// Stop before a member separator or enclosing struct boundary.
    StructMember,
    /// Stop before an enumerator separator or enclosing enum boundary.
    EnumeratorValue,
    /// Stop before a caller-owned statement-expression delimiter.
    StatementExpression(ExpressionTerminator),
    /// Stop before a statement boundary while retaining the enclosing `}`.
    Statement,
}

/// Parser-owned state for the currently active synchronization scan.
///
/// Keeping delimiter depth here makes recovery ownership inspectable and keeps
/// the state resumable if the token source becomes asynchronous in a later
/// phase. `active` is `None` whenever normal frame execution is in progress.
///
/// C99: §5.1.1.3, p. 11; PDF p. 23 requires diagnostics but leaves recovery
/// strategy to the implementation.
#[derive(Debug, Default)]
struct RecoveryState {
    active: Option<ActiveRecovery>,
}

/// Delimiter depth at which a conditional question mark was consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DelimiterDepth {
    parentheses: usize,
    brackets:    usize,
    braces:      usize,
}

/// Delimiter depth and policy for one active recovery scan.
///
/// C99: delimiter ownership follows the productions of §6.5-§6.9,
/// pp. 67-144; PDF pp. 79-156. Depth tracking is an implementation mechanism.
#[derive(Debug)]
struct ActiveRecovery {
    set:         SynchronizationSet,
    parentheses: usize,
    brackets:    usize,
    braces:      usize,
    questions:   Vec<DelimiterDepth>,
    last_token:  Option<TokenType>,
}

impl RecoveryState {
    /// Starts a scan owned by `set.target` with balanced delimiter depth.
    fn begin(&mut self, set: SynchronizationSet) {
        debug_assert!(self.active.is_none(), "recovery scans cannot nest");
        self.active = Some(ActiveRecovery {
            set,
            parentheses: 0,
            brackets: 0,
            braces: 0,
            questions: Vec::new(),
            last_token: None,
        });
    }

    /// Returns the active scan; callers use this to decide whether to stop.
    fn active(&self) -> &ActiveRecovery {
        self.active.as_ref().expect("a recovery scan is active")
    }

    /// Records one consumed token for balanced recovery.
    fn consume(&mut self, token: TokenType) {
        let state = self.active.as_mut().expect("a recovery scan is active");
        match token {
            | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => {
                state.parentheses += 1;
            },
            | TokenType::Operator(OperatorTokenType::ClosingParenthesis) if state.parentheses > 0 =>
            {
                state.parentheses -= 1;
                Self::discard_closed_questions(state);
            },
            | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                state.brackets += 1;
            },
            | TokenType::Operator(OperatorTokenType::ClosingSquareBracket) if state.brackets > 0 =>
            {
                state.brackets -= 1;
                Self::discard_closed_questions(state);
            },
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                state.braces += 1;
            },
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) if state.braces > 0 => {
                state.braces -= 1;
                Self::discard_closed_questions(state);
            },
            | TokenType::Operator(OperatorTokenType::QuestionMark) => {
                state.questions.push(DelimiterDepth {
                    parentheses: state.parentheses,
                    brackets:    state.brackets,
                    braces:      state.braces,
                });
            },
            | TokenType::Operator(OperatorTokenType::Colon) => {
                let depth = DelimiterDepth {
                    parentheses: state.parentheses,
                    brackets:    state.brackets,
                    braces:      state.braces,
                };
                if let Some(index) = state
                    .questions
                    .iter()
                    .rposition(|question| *question == depth)
                {
                    _ = state.questions.remove(index);
                }
            },
            | _ => {},
        }
        state.last_token = Some(token);
    }

    fn discard_closed_questions(state: &mut ActiveRecovery) {
        let depth = DelimiterDepth {
            parentheses: state.parentheses,
            brackets:    state.brackets,
            braces:      state.braces,
        };
        state.questions.retain(|question| {
            question.parentheses <= depth.parentheses
                && question.brackets <= depth.brackets
                && question.braces <= depth.braces
        });
    }

    /// Completes the active scan and restores normal parser execution.
    fn finish(&mut self) {
        drop(self.active.take().expect("a recovery scan is active"));
    }
}

/// Grammar child whose implementation is intentionally deferred beyond Phase
/// 02 while its parent production and recovery seam remain explicit.
///
/// C99: array bounds are §6.7.5.2, pp. 116-117; PDF pp. 128-129; bit-field
/// widths are §6.7.2.1, p. 101; PDF p. 113; enumerator values are §6.7.2.2,
/// p. 105; PDF p. 117; initializers are §6.7.8, pp. 125-130; PDF pp. 137-142;
/// function bodies are §6.9.1, pp. 141-142; PDF pp. 153-154.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FutureChildKind {
    /// Assignment expression between `[` and `]` in an array declarator.
    ArrayBoundExpression,
    /// Constant expression following `:` in a bit-field declaration.
    BitFieldWidthExpression,
    /// Constant expression following `=` in an enumerator.
    EnumeratorValueExpression,
    /// Object initializer following `=` in an init-declarator.
    Initializer,
    /// General expression in a statement position.
    StatementExpression,
    /// Constant expression in a `case` label.
    StatementConstantExpression,
}

impl FutureChildKind {
    /// Returns the grammar-position label used for this deferred child.
    fn label(self) -> &'static str {
        match self {
            | Self::ArrayBoundExpression => "array-bound-expression",
            | Self::BitFieldWidthExpression => "bit-field-width-expression",
            | Self::EnumeratorValueExpression => "enumerator-value-expression",
            | Self::Initializer => "initializer",
            | Self::StatementExpression => "statement-expression",
            | Self::StatementConstantExpression => "statement-constant-expression",
        }
    }

    fn not_implemented_error(self) -> ParserErrorType {
        match self {
            | Self::ArrayBoundExpression => ParserErrorType::ArrayBoundExpressionNotImplemented,
            | Self::BitFieldWidthExpression =>
                ParserErrorType::BitFieldWidthExpressionNotImplemented,
            | Self::EnumeratorValueExpression =>
                ParserErrorType::EnumeratorValueExpressionNotImplemented,
            | Self::Initializer => ParserErrorType::InitializerNotImplemented,
            | Self::StatementExpression | Self::StatementConstantExpression =>
                ParserErrorType::StatementExpressionNotImplemented,
        }
    }
}

/// Placeholder frame that emits one stable unsupported-child diagnostic and
/// synchronizes at the boundary chosen by [`FutureChildKind`].
///
/// C99: the deferred productions are cited by [`FutureChildKind`]. Reporting
/// them rather than silently accepting them follows §5.1.1.3, p. 11; PDF p. 23.
#[derive(Debug, Clone, Copy)]
struct FutureChildFrame {
    /// Deferred grammar family represented by this frame.
    kind:                     FutureChildKind,
    /// Whether the diagnostic/recovery step has run.
    phase:                    FutureChildPhase,
    /// Provenance consumed by recovery for the parent placeholder node.
    recovered_source_vectors: Option<SourceVectors>,
    /// Caller-selected stop token for statement expressions.
    expression_terminator:    Option<ExpressionTerminator>,
    /// Optional caller-selected synchronization policy for deferred children.
    recovery_kind:            Option<SynchronizationKind>,
}

/// Lifecycle of a deferred child frame.
///
/// C99: implementation state for the deferred productions cited by
/// [`FutureChildKind`]; the standard does not prescribe parser states.
#[derive(Debug, Clone, Copy)]
enum FutureChildPhase {
    /// Emit the unsupported-child diagnostic and request recovery.
    Start,
    /// Return the recovered source to the parent.
    Recovered,
}

impl FutureChildFrame {
    fn new(kind: FutureChildKind) -> Self {
        Self {
            kind,
            phase: FutureChildPhase::Start,
            recovered_source_vectors: None,
            expression_terminator: None,
            recovery_kind: None,
        }
    }

    fn with_recovery(kind: FutureChildKind, recovery_kind: SynchronizationKind) -> Self {
        Self {
            recovery_kind: Some(recovery_kind),
            ..Self::new(kind)
        }
    }

    fn statement(kind: FutureChildKind, terminator: ExpressionTerminator) -> Self {
        debug_assert!(
            matches!(
                kind,
                FutureChildKind::StatementExpression | FutureChildKind::StatementConstantExpression
            ),
            "statement recovery requires an expression-shaped deferred child"
        );
        Self {
            kind,
            phase: FutureChildPhase::Start,
            recovered_source_vectors: None,
            expression_terminator: Some(terminator),
            recovery_kind: None,
        }
    }
}

/// Root frame that converts one declaration child into a valid or explicitly
/// recovered external-declaration item.
///
/// C99: external-declaration is specified by §6.9, p. 140; PDF p. 152.
#[derive(Debug, Clone, Copy)]
struct ExternalDeclarationFrame {
    /// Current root-frame transition.
    phase:                ExternalDeclarationPhase,
    /// Hard-error count at entry, used only to classify the yielded AST.
    starting_error_count: usize,
}

/// Transitions for one external declaration.
///
/// C99: §6.9, p. 140; PDF p. 152. The phase split is an implementation detail.
#[derive(Debug, Clone, Copy)]
enum ExternalDeclarationPhase {
    /// Push the declaration child without consuming its first token.
    Start,
    /// Classify the completed declaration from diagnostics emitted since entry.
    AwaitDeclaration,
    /// Receive a function definition selected from the completed declaration
    /// head.
    AwaitFunctionDefinition,
}

impl ExternalDeclarationFrame {
    fn new(starting_error_count: usize) -> Self {
        Self {
            phase: ExternalDeclarationPhase::Start,
            starting_error_count,
        }
    }
}

/// Parses a declaration shell around declaration specifiers, comma-separated
/// declarators, optional deferred initializers, and the terminating semicolon.
///
/// C99: declaration, init-declarator-list, and init-declarator are §6.7,
/// p. 97; PDF p. 109.
#[derive(Debug, Clone, Copy)]
struct DeclarationFrame {
    /// Current declaration transition.
    phase: DeclarationPhase,
    /// Specifiers shared by every init-declarator in this declaration.
    declaration_specifiers: Option<DeclarationSpecifiers>,
    /// Initial arena length used to build this declaration's final slice.
    init_declarator_start: u32,
    /// Provenance accumulated across specifiers, declarators, and separators.
    source_vectors: Option<SourceVectors>,
    /// Most recently stored init-declarator, used to attach an initializer.
    last_init_index: Option<u32>,
    /// Provenance of `=` retained while the initializer child runs.
    initializer_source: Option<SourceVectors>,
    /// Grammar context controlling function-definition handoff and `}`
    /// ownership.
    context: DeclarationContext,
    is_function_definition_head: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclarationContext {
    External,
    Block,
    ForInitializer,
    OldStyleParameter,
}

/// State transitions for [`DeclarationFrame`].
///
/// C99: §6.7, p. 97; PDF p. 109. The phases encode that production
/// iteratively.
#[derive(Debug, Clone, Copy)]
enum DeclarationPhase {
    /// Push declaration specifiers.
    Start,
    /// Receive specifiers and decide whether a declarator follows.
    AwaitSpecifiers,
    /// Receive one named declarator.
    AwaitDeclarator,
    /// Resume at a separator after recovering a missing declarator.
    AfterMissingDeclarator,
    /// Classify the token following a completed declarator.
    AfterDeclarator,
    /// Push the deferred initializer child after consuming `=`.
    PushInitializer,
    /// Attach a recovered initializer placeholder.
    AwaitInitializer,
    /// Push another declarator after consuming `,`.
    BeforeNextDeclarator,
    /// Store the declaration and return its arena handle.
    Finish,
}

impl DeclarationFrame {
    fn new(init_declarator_start: u32, context: DeclarationContext) -> Self {
        Self {
            phase: DeclarationPhase::Start,
            declaration_specifiers: None,
            init_declarator_start,
            source_vectors: None,
            last_init_index: None,
            initializer_source: None,
            context,
            is_function_definition_head: false,
        }
    }
}

/// Grammar context controlling which specifier families are legal.
///
/// C99: declaration-specifiers are §6.7, p. 97; PDF p. 109, while
/// specifier-qualifier-list is §6.7.2.1, p. 101; PDF p. 113.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpecifierMode {
    /// Full declaration specifiers, including storage and function specifiers.
    Declaration,
    /// Struct member/type-name specifiers: types and qualifiers only.
    SpecifierQualifier,
}

/// Accumulates one declaration-specifier or specifier-qualifier sequence.
///
/// C99: §6.7, p. 97; PDF p. 109; §6.7.2.1, p. 101; PDF p. 113.
#[derive(Debug, Clone, Copy)]
struct DeclarationSpecifiersFrame {
    /// Current collection/child-wait transition.
    phase:                  DeclarationSpecifiersPhase,
    /// Grammar context limiting legal specifier families.
    mode:                   SpecifierMode,
    /// Accumulated normalized specifier result.
    specifiers:             DeclarationSpecifiers,
    /// Whether at least one legal specifier has been consumed.
    consumed:               bool,
    /// Whether a storage-class specifier has already appeared.
    storage_seen:           bool,
    /// Owning tag keyword retained while its child frame runs.
    pending_type_specifier: Option<Token>,
    /// Provenance accumulated across the complete specifier sequence.
    source_vectors:         Option<SourceVectors>,
}

/// Child-wait states used while collecting declaration specifiers.
///
/// C99: the child alternatives are type-specifiers from §6.7.2,
/// pp. 99-100; PDF pp. 111-112.
#[derive(Debug, Clone, Copy)]
enum DeclarationSpecifiersPhase {
    /// Consume primitive, storage, qualifier, function, and typedef specifiers.
    Collect,
    /// Receive the struct/union specifier pushed by its keyword.
    AwaitStructOrUnion,
    /// Receive the enum specifier pushed by its keyword.
    AwaitEnum,
}

impl DeclarationSpecifiersFrame {
    fn new(mode: SpecifierMode) -> Self {
        Self {
            phase: DeclarationSpecifiersPhase::Collect,
            mode,
            specifiers: DeclarationSpecifiers::new(),
            consumed: false,
            storage_seen: false,
            pending_type_specifier: None,
            source_vectors: None,
        }
    }
}

/// Whether a declarator requires, forbids, or optionally accepts a name.
///
/// C99: named declarators are §6.7.5, pp. 114-121; PDF pp. 126-133; abstract
/// declarators are §6.7.6, p. 122; PDF p. 134.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclaratorMode {
    /// Ordinary declarator requiring an identifier base.
    Named,
    /// Abstract declarator used where no identifier may be declared.
    Abstract,
    /// Parameter declarator that may be named or abstract.
    MaybeAbstract,
}

#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "The booleans retain independent C array/declarator grammar facts."
)]
/// Iteratively parses named, abstract, or maybe-abstract declarators.
///
/// C99: declarator and direct-declarator are §6.7.5, pp. 114-121; PDF
/// pp. 126-133. Abstract forms are §6.7.6, p. 122; PDF p. 134.
struct DeclaratorFrame {
    /// Current pointer/base/suffix transition.
    phase: DeclaratorPhase,
    /// Whether the declarator requires or permits an identifier.
    mode: DeclaratorMode,
    /// Qualifiers for completed pointer levels, outermost first.
    pointer_qualifiers: Vec<TypeQualifiers>,
    /// Direct base and suffixes accumulated before arena insertion.
    direct_declarators: Vec<DirectDeclarator>,
    /// Qualifiers being collected for the current pointer level.
    current_qualifiers: TypeQualifiers,
    /// Whether at least one pointer level has been parsed.
    has_pointer_level: bool,
    /// Whether an identifier or parenthesized base has been parsed.
    has_direct_declarator: bool,
    /// Qualifiers accumulated for the active array suffix.
    array_qualifiers: TypeQualifiers,
    /// Whether array qualifiers occurred before `static`.
    array_qualifiers_before_static: bool,
    /// Whether the active array suffix contains `static`.
    array_is_static: bool,
    /// Whether the active array suffix uses the `[*]` form.
    array_is_pointer: bool,
    /// Provenance accumulated across every declarator component.
    source_vectors: Option<SourceVectors>,
}

/// State transitions for pointer, base, and suffix portions of a declarator.
///
/// C99: pointer, array, and function-derived declarator productions are
/// §6.7.5.1-§6.7.5.3, pp. 115-121; PDF pp. 127-133.
#[derive(Debug, Clone, Copy)]
enum DeclaratorPhase {
    /// Decide whether the declarator begins with a pointer or direct base.
    PointerOrBase,
    /// Collect qualifiers for the current pointer level.
    PointerQualifiers,
    /// Parse the identifier or opening parenthesis of the direct base.
    Base,
    /// Push a nested declarator after `(`.
    PushNested,
    /// Disambiguate an abstract `(` as grouping or a function suffix.
    ClassifyAbstractParenthesis,
    /// Receive a nested parenthesized declarator.
    AwaitNested,
    /// Require the `)` owned by a parenthesized declarator.
    ExpectNestedClose(Declarator),
    /// Consume zero or more array/function suffixes.
    Suffix,
    /// Parse array qualifiers, `static`, `*`, or a deferred bound.
    Array,
    /// Require the closing bracket of an expression-free array suffix.
    ArrayExpectClose,
    /// Receive and close a deferred array-bound expression.
    AwaitArrayBound,
    /// Decide whether a function suffix is empty, K&R, or prototype-style.
    FunctionStart,
    /// Receive a parameter-list child.
    AwaitParameterList,
    /// Store qualifiers/direct parts and return the declarator.
    Finish,
}

impl DeclaratorFrame {
    fn new(mode: DeclaratorMode) -> Self {
        Self {
            phase: DeclaratorPhase::PointerOrBase,
            mode,
            pointer_qualifiers: Vec::new(),
            direct_declarators: Vec::new(),
            current_qualifiers: TypeQualifiers::empty(),
            has_pointer_level: false,
            has_direct_declarator: false,
            array_qualifiers: TypeQualifiers::empty(),
            array_qualifiers_before_static: false,
            array_is_static: false,
            array_is_pointer: false,
            source_vectors: None,
        }
    }
}

/// Parses either a prototype parameter list or an allowed K&R identifier list
/// and owns the suffix's closing parenthesis.
///
/// C99: parameter-type-list, parameter-list, parameter-declaration, and
/// identifier-list are §6.7.5, p. 114; PDF p. 126; function declarator rules
/// are §6.7.5.3, pp. 118-121; PDF pp. 130-133.
#[derive(Debug)]
struct ParameterListFrame {
    /// Current prototype/K&R transition.
    phase: ParameterListPhase,
    /// Whether this syntactic position permits a K&R identifier list.
    allow_k_and_r: bool,
    /// Prototype parameters accumulated before arena insertion.
    parameters: Vec<ParameterDeclaration>,
    /// K&R identifiers accumulated before arena insertion.
    identifiers: Vec<Identifier>,
    /// Specifiers retained while an optional parameter declarator runs.
    pending_specifiers: Option<DeclarationSpecifiers>,
    /// Specifier provenance retained for parameter-source construction.
    pending_source: Option<SourceVectors>,
    /// Whether `...` terminated the prototype parameter list.
    is_variadic: bool,
    /// Whether variadic recovery may unwind at a later declaration starter.
    can_unwind_variadic_recovery: bool,
    /// Provenance accumulated across the entire parenthesized suffix.
    source_vectors: Option<SourceVectors>,
    /// Scope depth restored by every parameter-list exit.
    entry_scope_depth: Option<usize>,
}

/// State transitions for prototype and K&R parameter-list forms.
///
/// C99: §6.7.5 and §6.7.5.3, pp. 114 and 118-121; PDF pp. 126 and 130-133.
#[derive(Debug, Clone, Copy)]
enum ParameterListPhase {
    /// Enter prototype scope and select K&R versus prototype syntax.
    Start,
    /// Consume one K&R parameter identifier.
    KAndRIdentifier,
    /// Require `,` or `)` after a K&R identifier.
    KAndRSeparator,
    /// Push declaration specifiers for one prototype parameter.
    PrototypeParameter,
    /// Receive parameter specifiers and decide whether a declarator follows.
    AwaitSpecifiers,
    /// Receive the optional named or abstract parameter declarator.
    AwaitDeclarator,
    /// Require `,` or `)` after a prototype parameter.
    PrototypeSeparator,
    /// Parse `...` or the next parameter after a comma.
    AfterComma,
    /// Require `)` immediately after `...`.
    ExpectCloseAfterEllipsis,
    /// Leave scope and return a K&R function suffix.
    FinishKAndR,
    /// Leave scope and return a prototype function suffix.
    FinishPrototype,
}

impl ParameterListFrame {
    fn new(allow_k_and_r: bool) -> Self {
        Self {
            phase: ParameterListPhase::Start,
            allow_k_and_r,
            parameters: Vec::new(),
            identifiers: Vec::new(),
            pending_specifiers: None,
            pending_source: None,
            is_variadic: false,
            can_unwind_variadic_recovery: false,
            source_vectors: None,
            entry_scope_depth: None,
        }
    }
}

/// Parses a struct-or-union specifier, including its optional tag and member
/// declaration list.
///
/// C99: structure and union specifiers, member declarations, and bit-fields
/// are §6.7.2.1, pp. 101-104; PDF pp. 113-116.
#[derive(Debug)]
struct StructOrUnionSpecifierFrame {
    /// Current tag/member transition.
    phase: StructOrUnionPhase,
    /// Keyword-selected aggregate kind.
    kind: Option<StructOrUnion>,
    /// Optional tag identifier.
    identifier: Option<Identifier>,
    /// Completed member declarations before arena insertion.
    declarations: Vec<StructDeclaration>,
    /// Declarators belonging to the member declaration in progress.
    member_declarators: Vec<StructDeclarator>,
    /// Specifiers shared by the member declarators in progress.
    member_specifiers: Option<DeclarationSpecifiers>,
    /// Named declarator waiting for an optional bit-field width.
    member_declarator: Option<Declarator>,
    /// Whether `{` was consumed, distinguishing a reference from a definition.
    body_started: bool,
    /// Provenance accumulated across the complete tag specifier.
    source_vectors: Option<SourceVectors>,
    /// Provenance accumulated for the member declaration in progress.
    member_source: Option<SourceVectors>,
    /// Provenance for the member declarator/bit-field currently being built.
    current_member_declarator_source: Option<SourceVectors>,
}

/// State transitions for a struct/union tag and member body.
///
/// C99: §6.7.2.1, pp. 101-104; PDF pp. 113-116.
#[derive(Debug, Clone, Copy)]
enum StructOrUnionPhase {
    /// Consume and classify the `struct` or `union` keyword.
    Start,
    /// Parse an optional tag or anonymous opening brace.
    NameOrBody,
    /// Decide whether a named tag also has a body.
    AfterName,
    /// Parse `}` or begin another member declaration.
    MemberStart,
    /// Receive member specifiers and select named/unnamed declarator syntax.
    AwaitMemberSpecifiers,
    /// Push a member declarator unless an unnamed bit-field starts with `:`.
    PushMemberDeclarator,
    /// Receive the optional member declarator.
    AwaitMemberDeclarator,
    /// Decide whether a bit-field width follows the member declarator.
    AfterMemberDeclarator,
    /// Push the deferred constant-expression bit-field width.
    PushBitFieldWidth,
    /// Receive the recovered bit-field width placeholder.
    AwaitBitFieldWidth,
    /// Require `,` or `;` after one struct declarator.
    AfterStructDeclarator,
    /// Store the completed body and return the tag-specifier handle.
    FinishBody,
}

impl StructOrUnionSpecifierFrame {
    fn new() -> Self {
        Self {
            phase: StructOrUnionPhase::Start,
            kind: None,
            identifier: None,
            declarations: Vec::new(),
            member_declarators: Vec::new(),
            member_specifiers: None,
            member_declarator: None,
            body_started: false,
            source_vectors: None,
            member_source: None,
            current_member_declarator_source: None,
        }
    }
}

/// Parses an enum specifier, including its optional tag, enumerators, trailing
/// comma, and deferred explicit values.
///
/// C99: enumeration specifiers and enumerators are §6.7.2.2,
/// pp. 105-107; PDF pp. 117-119.
#[derive(Debug)]
struct EnumSpecifierFrame {
    /// Current tag/enumerator transition.
    phase: EnumPhase,
    /// Optional enum tag.
    name: Option<Identifier>,
    /// Completed enumerators before arena insertion.
    enumerators: Vec<Enumerator>,
    /// Enumerator name waiting for an optional explicit value.
    current_enumerator: Option<Identifier>,
    /// Whether `{` was consumed, distinguishing a reference from a definition.
    body_started: bool,
    /// Whether malformed-body recovery stopped before an outer declaration.
    stopped_before_declaration: bool,
    /// Provenance accumulated across the complete enum specifier.
    source_vectors: Option<SourceVectors>,
    /// Provenance for the enumerator currently being built.
    current_enumerator_source: Option<SourceVectors>,
}

/// State transitions for an enum tag and enumerator list.
///
/// C99: §6.7.2.2, pp. 105-107; PDF pp. 117-119.
#[derive(Debug, Clone, Copy)]
enum EnumPhase {
    /// Consume the `enum` keyword.
    Start,
    /// Parse an optional tag or anonymous opening brace.
    NameOrBody,
    /// Decide whether a named tag also has a body.
    AfterName,
    /// Parse an enumerator name or the body's closing brace.
    EnumeratorOrClose,
    /// Decide whether `=` introduces an explicit value.
    AfterEnumeratorName,
    /// Push the deferred constant-expression value.
    PushEnumeratorValue,
    /// Receive the recovered enumerator-value placeholder.
    AwaitEnumeratorValue,
    /// Require `,` or `}` after one enumerator.
    AfterEnumerator,
    /// Store the completed body and return the enum-specifier handle.
    FinishBody,
}

impl EnumSpecifierFrame {
    fn new() -> Self {
        Self {
            phase: EnumPhase::Start,
            name: None,
            enumerators: Vec::new(),
            current_enumerator: None,
            body_started: false,
            stopped_before_declaration: false,
            source_vectors: None,
            current_enumerator_source: None,
        }
    }
}

#[expect(
    clippy::missing_assert_message,
    reason = "Frame phases assert the typed driver protocol, whose mismatch already identifies \
              the invariant."
)]
impl FunctionDefinitionFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | FunctionDefinitionPhase::Start => {
                debug_assert!(returned.is_none());
                let declaration = parser.syntax.declarations[self.head.0 as usize];
                let declarator = parser
                    .declaration_head_declarator(self.head)
                    .expect("a function definition has one uninitialized declarator");
                self.source_vectors = Some(declaration.source_vectors);
                self.entry_scope_depth = Some(parser.scopes.depth());
                parser.scopes.enter_scope(ScopeKind::Function);
                parser.label_scopes.push(LabelScope::default());

                if let Some(suffix) = parser.function_suffix(declarator) {
                    match suffix {
                        | DirectDeclarator::Function { parameter_list, .. } => {
                            if parameter_list.length == 0 {
                                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                                return ParseAction::Reprocess;
                            }
                            let start = parameter_list.start_index as usize;
                            let end = start + parameter_list.length as usize;
                            let mut names = Vec::new();
                            for parameter in &parser.syntax.parameter_declarations[start..end] {
                                parser.collect_type_specifier_bindings(
                                    parameter.declaration_specifiers.type_specifiers,
                                    &mut names,
                                );
                                if let Some(name) = parameter
                                    .declarator
                                    .and_then(|declarator| parser.declarator_identifier(declarator))
                                    .map(|identifier| identifier.name)
                                {
                                    names.push(name);
                                }
                            }
                            for name in names {
                                parser.scopes.publish(name, NameClass::Ordinary);
                            }
                        },
                        | DirectDeclarator::KAndRStyleFunction { parameters } => {
                            if parameters.length == 0 {
                                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                                return ParseAction::Reprocess;
                            }
                            let start = parameters.start_index as usize;
                            let end = start + parameters.length as usize;
                            let names = parser.syntax.identifiers[start..end]
                                .iter()
                                .map(|identifier| identifier.name)
                                .collect::<Vec<_>>();
                            for name in names {
                                parser.scopes.publish(name, NameClass::Ordinary);
                            }
                        },
                        | _ => unreachable!("function suffix helper returns only function forms"),
                    }
                }
                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                ParseAction::Reprocess
            },
            | FunctionDefinitionPhase::DeclarationOrBody => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    self.phase = FunctionDefinitionPhase::AwaitBody;
                    ParseAction::Push(ParseFrame::CompoundStatement(CompoundStatementFrame::new(
                        parser.hard_error_count,
                        true,
                    )))
                } else if parser.declaration_is_old_style_function_head(self.head)
                    && token.is_some_and(|token| parser.declaration_starter(token))
                {
                    self.phase = FunctionDefinitionPhase::AwaitDeclaration;
                    ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                        parser.syntax.init_declarators.len().to_u32(),
                        DeclarationContext::OldStyleParameter,
                    )))
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedFunctionBody(token.map(|token| token.kind)),
                        token,
                    );
                    if token.is_none() {
                        let body = parser.syntax.statements.len().to_u32();
                        parser.syntax.statements.push(Statement {
                            kind:           StatementType::Compound {
                                items: VectorSlice::empty(),
                            },
                            source_vectors: SourceVectors::default(),
                            recovered:      true,
                        });
                        self.body = Some(StatementIndex(body));
                        self.phase = FunctionDefinitionPhase::Finish;
                        ParseAction::Reprocess
                    } else {
                        self.phase = FunctionDefinitionPhase::AwaitBody;
                        ParseAction::Push(ParseFrame::CompoundStatement(
                            CompoundStatementFrame::new(parser.hard_error_count, true),
                        ))
                    }
                }
            },
            | FunctionDefinitionPhase::AwaitDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("old-style declaration returned an unexpected value: {returned:?}");
                };
                let source = parser.syntax.declarations[declaration.0 as usize].source_vectors;
                self.source_vectors = Some(
                    self.source_vectors
                        .map_or(source, |existing| context.merge_vectors(existing, source)),
                );
                self.old_style_declarations.push(declaration);
                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                ParseAction::Reprocess
            },
            | FunctionDefinitionPhase::AwaitBody => {
                let Some(ParseValue::CompoundStatement(body)) = returned else {
                    panic!("function body returned an unexpected value: {returned:?}");
                };
                let source = parser.statement_source(body);
                self.source_vectors = Some(
                    self.source_vectors
                        .map_or(source, |existing| context.merge_vectors(existing, source)),
                );
                self.body = Some(body);
                self.phase = FunctionDefinitionPhase::Finish;
                ParseAction::Reprocess
            },
            | FunctionDefinitionPhase::Finish => {
                debug_assert!(returned.is_none());
                let head = parser.syntax.declarations[self.head.0 as usize];
                let declarator = parser
                    .declaration_head_declarator(self.head)
                    .expect("function definition head remains available");
                let declaration_start = parser.syntax.declaration_indices.len().to_u32();
                parser
                    .syntax
                    .declaration_indices
                    .append(&mut self.old_style_declarations);
                let recovered = parser.hard_error_count > self.starting_error_count;
                let index = parser.syntax.function_definitions.len().to_u32();
                parser.syntax.function_definitions.push(FunctionDefinition {
                    declaration_specifiers: head.declaration_specifiers,
                    declarator,
                    old_style_declarations: VectorSlice::new(
                        declaration_start,
                        parser.syntax.declaration_indices.len().to_u32(),
                    ),
                    body: self.body.expect("function definition has a body node"),
                    source_vectors: self.source_vectors.unwrap_or_default(),
                    recovered,
                });
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("function definition entered function scope"),
                );
                drop(
                    parser
                        .label_scopes
                        .pop()
                        .expect("function definition owns a label namespace"),
                );
                ParseAction::Reduce(ParseValue::FunctionDefinition(FunctionDefinitionIndex(
                    index,
                )))
            },
        }
    }
}

#[expect(
    clippy::missing_assert_message,
    reason = "Frame phases assert the typed driver protocol, whose mismatch already identifies \
              the invariant."
)]
impl CompoundStatementFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | CompoundStatementPhase::Start => {
                debug_assert!(returned.is_none());
                self.entry_scope_depth = Some(parser.scopes.depth());
                parser.scopes.enter_scope(ScopeKind::Block);
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening brace exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    if self.function_body {
                        let name = context.string_cache.intern("__func__");
                        parser.scopes.publish(name, NameClass::Ordinary);
                    }
                    self.phase = CompoundStatementPhase::ItemOrClose;
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedOpeningCurlyBraceInCompoundStatement(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = CompoundStatementPhase::ItemOrClose;
                    ParseAction::Reprocess
                }
            },
            | CompoundStatementPhase::ItemOrClose => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing brace exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = CompoundStatementPhase::Finish;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingCurlyBraceInCompoundStatement(None),
                        None,
                    );
                    self.phase = CompoundStatementPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    let is_label = token.is_some_and(|token| token.kind == TokenType::Identifier)
                        && is_operator(parser.cursor.following(context), OperatorTokenType::Colon);
                    if !is_label && token.is_some_and(|token| parser.declaration_starter(token)) {
                        self.phase = CompoundStatementPhase::AwaitDeclaration;
                        ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                            parser.syntax.init_declarators.len().to_u32(),
                            DeclarationContext::Block,
                        )))
                    } else {
                        self.phase = CompoundStatementPhase::AwaitStatement;
                        ParseAction::Push(ParseFrame::Statement(StatementFrame::new(
                            parser.hard_error_count,
                            None,
                        )))
                    }
                }
            },
            | CompoundStatementPhase::AwaitDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("block declaration returned an unexpected value: {returned:?}");
                };
                let source = parser.syntax.declarations[declaration.0 as usize].source_vectors;
                self.source_vectors = Some(
                    self.source_vectors
                        .map_or(source, |existing| context.merge_vectors(existing, source)),
                );
                self.items.push(BlockItem::Declaration(declaration));
                self.phase = CompoundStatementPhase::ItemOrClose;
                ParseAction::Reprocess
            },
            | CompoundStatementPhase::AwaitStatement => {
                let Some(ParseValue::Statement(statement)) = returned else {
                    panic!("block statement returned an unexpected value: {returned:?}");
                };
                let source = parser.statement_source(statement);
                self.source_vectors = Some(
                    self.source_vectors
                        .map_or(source, |existing| context.merge_vectors(existing, source)),
                );
                self.items.push(BlockItem::Statement(statement));
                self.phase = CompoundStatementPhase::ItemOrClose;
                ParseAction::Reprocess
            },
            | CompoundStatementPhase::Finish => {
                debug_assert!(returned.is_none());
                let item_start = parser.syntax.block_items.len().to_u32();
                parser.syntax.block_items.append(&mut self.items);
                let index = parser.syntax.statements.len().to_u32();
                parser.syntax.statements.push(Statement {
                    kind:           StatementType::Compound {
                        items: VectorSlice::new(
                            item_start,
                            parser.syntax.block_items.len().to_u32(),
                        ),
                    },
                    source_vectors: self.source_vectors.unwrap_or_default(),
                    recovered:      parser.hard_error_count > self.starting_error_count,
                });
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("compound statement entered block scope"),
                );
                ParseAction::Reduce(ParseValue::CompoundStatement(StatementIndex(index)))
            },
        }
    }
}

#[expect(
    clippy::missing_assert_message,
    clippy::too_many_lines,
    reason = "The single iterative statement grammar dispatcher keeps all phase transitions and \
              delimiter ownership visible in one frame implementation."
)]
impl StatementFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        if self.entry_scope_depth.is_none() {
            self.entry_scope_depth = Some(parser.scopes.depth());
            if let Some(kind) = self.implicit_scope {
                parser.scopes.enter_scope(kind);
            }
        }

        match self.phase {
            | StatementPhase::Start => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    self.phase = StatementPhase::AwaitCompound;
                    return ParseAction::Push(ParseFrame::CompoundStatement(
                        CompoundStatementFrame::new(parser.hard_error_count, false),
                    ));
                }
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::Finish(StatementType::Null);
                    return ParseAction::Consume;
                }
                if let Some(token) = token
                    && token.kind == TokenType::Identifier
                    && is_operator(parser.cursor.following(context), OperatorTokenType::Colon)
                {
                    let identifier = Identifier::new(token.contents);
                    if let Some(labels) = parser.label_scopes.last_mut() {
                        _ = labels.definitions.insert(identifier.name);
                    }
                    self.merge_token(parser, context, token);
                    self.phase = StatementPhase::IdentifierLabelColon(identifier);
                    return ParseAction::Consume;
                }
                if let Some(token) = token
                    && let TokenType::Keyword(keyword) = token.kind
                {
                    match keyword {
                        | KeywordTokenType::Return => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::ReturnStart;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Break => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::SimpleJumpSemicolon(SimpleJump::Break);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Continue => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::SimpleJumpSemicolon(SimpleJump::Continue);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Goto => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::GotoIdentifier;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Case => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::CaseExpression;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Default => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::DefaultColon;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::If => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitSelection);
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::HeaderOpening(HeaderKind::If);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Switch => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitSelection);
                            parser.switch_scopes.push(SwitchScope::default());
                            self.owns_switch_scope = true;
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::HeaderOpening(HeaderKind::Switch);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::While => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitIteration);
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::HeaderOpening(HeaderKind::While);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Do => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitIteration);
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::DoPushBody;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::For => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitIteration);
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::ForOpening;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Sizeof => {},
                        | _ if parser.declaration_starter(token) => {
                            parser.report(
                                context,
                                ParserErrorType::ExpectedStatement(Some(token.kind)),
                                Some(token),
                            );
                            self.phase = StatementPhase::Recovered;
                            return ParseAction::Recover(SynchronizationSet {
                                kind:   SynchronizationKind::Statement,
                                target: ParseFrameKind::Statement,
                            });
                        },
                        | _ => {},
                    }
                }
                let stray_else = token
                    .is_some_and(|token| token.kind == TokenType::Keyword(KeywordTokenType::Else));
                if token.is_none()
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                    || stray_else
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatement(token.map(|token| token.kind)),
                        token,
                    );
                    if stray_else && !self.leave_else_unconsumed {
                        self.merge_token(parser, context, token.expect("else token exists"));
                        self.phase = StatementPhase::Finish(StatementType::Null);
                        return ParseAction::Consume;
                    }
                    return self.finish(parser, StatementType::Null);
                }
                self.phase = StatementPhase::AwaitExpression;
                ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::statement(
                    FutureChildKind::StatementExpression,
                    ExpressionTerminator::Semicolon,
                )))
            },
            | StatementPhase::AwaitCompound => {
                let Some(ParseValue::CompoundStatement(statement)) = returned else {
                    panic!("compound statement returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, statement);
                self.finish_existing(parser, statement)
            },
            | StatementPhase::AwaitExpression => {
                let slot = Self::future_slot(returned, FutureChildKind::StatementExpression);
                self.merge_slot(parser, context, slot);
                self.phase = StatementPhase::ExpressionSemicolon(slot);
                ParseAction::Reprocess
            },
            | StatementPhase::ExpressionSemicolon(slot) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "expression statement");
                self.phase = StatementPhase::Finish(StatementType::Expression(slot));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ReturnStart => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::Finish(StatementType::Return(None));
                    ParseAction::Consume
                } else if Self::at_expression_boundary(token, ExpressionTerminator::Semicolon)
                    || token.is_some_and(|token| parser.declaration_starter(token))
                {
                    self.phase = StatementPhase::ReturnSemicolon(None);
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitReturnExpression;
                    ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::statement(
                        FutureChildKind::StatementExpression,
                        ExpressionTerminator::Semicolon,
                    )))
                }
            },
            | StatementPhase::AwaitReturnExpression => {
                let slot = Self::future_slot(returned, FutureChildKind::StatementExpression);
                self.merge_slot(parser, context, slot);
                self.phase = StatementPhase::ReturnSemicolon(Some(slot));
                ParseAction::Reprocess
            },
            | StatementPhase::ReturnSemicolon(expression) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "return statement");
                self.phase = StatementPhase::Finish(StatementType::Return(expression));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::SimpleJumpSemicolon(jump) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "jump statement");
                self.phase = StatementPhase::Finish(match jump {
                    | SimpleJump::Break => StatementType::Break,
                    | SimpleJump::Continue => StatementType::Continue,
                });
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::GotoIdentifier => {
                debug_assert!(returned.is_none());
                let identifier = if let Some(token) = token
                    && token.kind == TokenType::Identifier
                {
                    let identifier = Identifier::new(token.contents);
                    if let Some(labels) = parser.label_scopes.last_mut() {
                        _ = labels.references.insert(identifier.name);
                    }
                    self.merge_token(parser, context, token);
                    self.phase = StatementPhase::GotoSemicolon(identifier);
                    return ParseAction::Consume;
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedGotoLabel(token.map(|token| token.kind)),
                        token,
                    );
                    Identifier::new(context.string_cache.intern("<missing-label>"))
                };
                self.phase = StatementPhase::GotoSemicolon(identifier);
                ParseAction::Reprocess
            },
            | StatementPhase::GotoSemicolon(identifier) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "goto statement");
                self.phase = StatementPhase::Finish(StatementType::Goto(identifier));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::IdentifierLabelColon(identifier) => {
                debug_assert!(returned.is_none());
                self.own_colon_or_report(parser, context, token, "identifier label");
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Identifier(identifier));
                if is_operator(token, OperatorTokenType::Colon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::CaseExpression => {
                debug_assert!(returned.is_none());
                if Self::at_expression_boundary(token, ExpressionTerminator::Colon) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatementExpression(
                            "case label",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::CaseColon(Self::missing_slot(parser, context));
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitCaseExpression;
                    ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::statement(
                        FutureChildKind::StatementConstantExpression,
                        ExpressionTerminator::Colon,
                    )))
                }
            },
            | StatementPhase::AwaitCaseExpression => {
                let slot =
                    Self::future_slot(returned, FutureChildKind::StatementConstantExpression);
                self.merge_slot(parser, context, slot);
                self.phase = StatementPhase::CaseColon(slot);
                ParseAction::Reprocess
            },
            | StatementPhase::CaseColon(expression) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(parser.cursor.following(context), OperatorTokenType::Colon)
                {
                    self.own_colon_or_report(parser, context, token, "case label");
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    return ParseAction::Consume;
                }
                self.own_colon_or_report(parser, context, token, "case label");
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Case(expression));
                if is_operator(token, OperatorTokenType::Colon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DefaultColon => {
                debug_assert!(returned.is_none());
                self.own_colon_or_report(parser, context, token, "default label");
                if parser
                    .switch_scopes
                    .last()
                    .is_some_and(|switch| switch.has_default)
                {
                    parser.report(context, ParserErrorType::DuplicateDefaultLabel, token);
                } else if let Some(switch) = parser.switch_scopes.last_mut() {
                    switch.has_default = true;
                }
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Default);
                if is_operator(token, OperatorTokenType::Colon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::PushLabeled(prefix) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::AwaitLabeled(prefix);
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    None,
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::AwaitLabeled(prefix) => {
                let Some(ParseValue::Statement(statement)) = returned else {
                    panic!("labeled child returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, statement);
                self.finish(
                    parser,
                    match prefix {
                        | LabelPrefix::Identifier(identifier) =>
                            StatementType::Label(identifier, statement),
                        | LabelPrefix::Case(expression) =>
                            StatementType::Case(expression, statement),
                        | LabelPrefix::Default => StatementType::Default(statement),
                    },
                )
            },
            | StatementPhase::HeaderOpening(kind) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    self.merge_token(parser, context, token.expect("opening parenthesis exists"));
                    self.phase = StatementPhase::HeaderExpression(kind);
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedOpeningParenthesisInStatement(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::HeaderExpression(kind);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::HeaderExpression(kind) => {
                debug_assert!(returned.is_none());
                if Self::at_expression_boundary(token, ExpressionTerminator::ClosingParenthesis) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatementExpression(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase =
                        StatementPhase::HeaderClosing(kind, Self::missing_slot(parser, context));
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitHeaderExpression(kind);
                    ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::statement(
                        FutureChildKind::StatementExpression,
                        ExpressionTerminator::ClosingParenthesis,
                    )))
                }
            },
            | StatementPhase::AwaitHeaderExpression(kind) => {
                let slot = Self::future_slot(returned, FutureChildKind::StatementExpression);
                self.merge_slot(parser, context, slot);
                self.phase = StatementPhase::HeaderClosing(kind, slot);
                ParseAction::Reprocess
            },
            | StatementPhase::HeaderClosing(kind, expression) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, context, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::PushHeaderBody(kind, expression);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(context),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::PushHeaderBody(kind, expression);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::PushHeaderBody(kind, expression) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::AwaitHeaderBody(kind, expression);
                let frame = if kind == HeaderKind::If {
                    StatementFrame::for_if_then(parser.hard_error_count, Some(kind.scope_kind()))
                } else {
                    StatementFrame::with_else_ownership(
                        parser.hard_error_count,
                        Some(kind.scope_kind()),
                        self.leave_else_unconsumed,
                    )
                };
                ParseAction::Push(ParseFrame::Statement(frame))
            },
            | StatementPhase::AwaitHeaderBody(kind, expression) => {
                let Some(ParseValue::Statement(body)) = returned else {
                    panic!("selection/iteration body returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, body);
                match kind {
                    | HeaderKind::If => {
                        self.phase = StatementPhase::IfAfterThen(expression, body);
                        ParseAction::Reprocess
                    },
                    | HeaderKind::Switch => self.finish(
                        parser,
                        StatementType::Switch {
                            condition_expression: expression,
                            body_statement:       body,
                        },
                    ),
                    | HeaderKind::While => self.finish(
                        parser,
                        StatementType::While {
                            condition_expression: expression,
                            body_statement:       body,
                        },
                    ),
                }
            },
            | StatementPhase::IfAfterThen(expression, then_statement) => {
                debug_assert!(returned.is_none());
                if token
                    .is_some_and(|token| token.kind == TokenType::Keyword(KeywordTokenType::Else))
                {
                    self.merge_token(parser, context, token.expect("else token exists"));
                    self.phase = StatementPhase::PushElse(expression, then_statement);
                    ParseAction::Consume
                } else {
                    self.finish(
                        parser,
                        StatementType::If {
                            condition_expression: expression,
                            then_statement,
                            else_statement: None,
                        },
                    )
                }
            },
            | StatementPhase::PushElse(expression, then_statement) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::AwaitElse(expression, then_statement);
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    Some(ScopeKind::ImplicitSelection),
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::AwaitElse(expression, then_statement) => {
                let Some(ParseValue::Statement(else_statement)) = returned else {
                    panic!("else child returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, else_statement);
                self.finish(
                    parser,
                    StatementType::If {
                        condition_expression: expression,
                        then_statement,
                        else_statement: Some(else_statement),
                    },
                )
            },
            | StatementPhase::DoPushBody => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::DoAwaitBody;
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    Some(ScopeKind::ImplicitIteration),
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::DoAwaitBody => {
                let Some(ParseValue::Statement(body)) = returned else {
                    panic!("do body returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, body);
                self.phase = StatementPhase::DoWhileKeyword(body);
                ParseAction::Reprocess
            },
            | StatementPhase::DoWhileKeyword(body) => {
                debug_assert!(returned.is_none());
                if token
                    .is_some_and(|token| token.kind == TokenType::Keyword(KeywordTokenType::While))
                {
                    self.merge_token(parser, context, token.expect("while token exists"));
                    self.phase = StatementPhase::DoOpening(body);
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedWhileAfterDoBody(token.map(|token| token.kind)),
                        token,
                    );
                    self.phase = StatementPhase::DoOpening(body);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DoOpening(body) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    self.merge_token(parser, context, token.expect("opening parenthesis exists"));
                    self.phase = StatementPhase::DoExpression(body);
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedOpeningParenthesisInStatement(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::DoExpression(body);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DoExpression(body) => {
                debug_assert!(returned.is_none());
                if Self::at_expression_boundary(token, ExpressionTerminator::ClosingParenthesis) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatementExpression(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase =
                        StatementPhase::DoClosing(body, Self::missing_slot(parser, context));
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::DoAwaitExpression(body);
                    ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::statement(
                        FutureChildKind::StatementExpression,
                        ExpressionTerminator::ClosingParenthesis,
                    )))
                }
            },
            | StatementPhase::DoAwaitExpression(body) => {
                let expression = Self::future_slot(returned, FutureChildKind::StatementExpression);
                self.merge_slot(parser, context, expression);
                self.phase = StatementPhase::DoClosing(body, expression);
                ParseAction::Reprocess
            },
            | StatementPhase::DoClosing(body, expression) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, context, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::DoSemicolon(body, expression);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(context),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::DoSemicolon(body, expression);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DoSemicolon(body, expression) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "do-while statement");
                self.phase = StatementPhase::Finish(StatementType::DoWhile {
                    condition_expression: expression,
                    body_statement:       body,
                });
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForOpening => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    self.merge_token(parser, context, token.expect("opening parenthesis exists"));
                    self.phase = StatementPhase::ForInitializer;
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedOpeningParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::ForInitializer;
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForInitializer => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::ForCondition(None);
                    ParseAction::Consume
                } else if token.is_none()
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.own_semicolon_or_report(parser, context, token, "for initializer");
                    self.phase = StatementPhase::ForCondition(None);
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    self.phase = StatementPhase::AwaitForInitializerDeclaration;
                    ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                        parser.syntax.init_declarators.len().to_u32(),
                        DeclarationContext::ForInitializer,
                    )))
                } else {
                    self.phase = StatementPhase::AwaitForInitializerExpression;
                    ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::statement(
                        FutureChildKind::StatementExpression,
                        ExpressionTerminator::ForSemicolon,
                    )))
                }
            },
            | StatementPhase::AwaitForInitializerExpression => {
                let expression = Self::future_slot(returned, FutureChildKind::StatementExpression);
                self.merge_slot(parser, context, expression);
                self.phase = StatementPhase::ForInitializerSemicolon(expression);
                ParseAction::Reprocess
            },
            | StatementPhase::AwaitForInitializerDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("for declaration returned an unexpected value: {returned:?}");
                };
                let source = parser.syntax.declarations[declaration.0 as usize].source_vectors;
                self.source_vectors = Some(
                    self.source_vectors
                        .map_or(source, |existing| context.merge_vectors(existing, source)),
                );
                self.phase =
                    StatementPhase::ForCondition(Some(ForInitializer::Declaration(declaration)));
                ParseAction::Reprocess
            },
            | StatementPhase::ForInitializerSemicolon(expression) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "for initializer");
                self.phase =
                    StatementPhase::ForCondition(Some(ForInitializer::Expression(expression)));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForCondition(initializer) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::ForIteration(initializer, None);
                    ParseAction::Consume
                } else if token.is_none()
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.own_semicolon_or_report(parser, context, token, "for condition");
                    self.phase = StatementPhase::ForIteration(initializer, None);
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitForCondition(initializer);
                    ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::statement(
                        FutureChildKind::StatementExpression,
                        ExpressionTerminator::ForSemicolon,
                    )))
                }
            },
            | StatementPhase::AwaitForCondition(initializer) => {
                let expression = Self::future_slot(returned, FutureChildKind::StatementExpression);
                self.merge_slot(parser, context, expression);
                self.phase = StatementPhase::ForConditionSemicolon(initializer, expression);
                ParseAction::Reprocess
            },
            | StatementPhase::ForConditionSemicolon(initializer, condition) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "for condition");
                self.phase = StatementPhase::ForIteration(initializer, Some(condition));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForIteration(initializer, condition) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, context, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::ForPushBody(initializer, condition, None);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(context),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else if token.is_none() {
                    self.phase = StatementPhase::ForClosing(initializer, condition, None);
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitForIteration(initializer, condition);
                    ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::statement(
                        FutureChildKind::StatementExpression,
                        ExpressionTerminator::ClosingParenthesis,
                    )))
                }
            },
            | StatementPhase::AwaitForIteration(initializer, condition) => {
                let expression = Self::future_slot(returned, FutureChildKind::StatementExpression);
                self.merge_slot(parser, context, expression);
                self.phase = StatementPhase::ForClosing(initializer, condition, Some(expression));
                ParseAction::Reprocess
            },
            | StatementPhase::ForClosing(initializer, condition, iteration) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, context, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::ForPushBody(initializer, condition, iteration);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(context),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::ForPushBody(initializer, condition, iteration);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForPushBody(initializer, condition, iteration) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::ForAwaitBody(initializer, condition, iteration);
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    Some(ScopeKind::ImplicitIteration),
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::ForAwaitBody(initializer, condition, iteration) => {
                let Some(ParseValue::Statement(body)) = returned else {
                    panic!("for body returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, body);
                self.finish(
                    parser,
                    StatementType::For {
                        initializer,
                        condition_expression: condition,
                        iteration_expression: iteration,
                        body_statement: body,
                    },
                )
            },
            | StatementPhase::Recovered => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::Finish(StatementType::Null);
                    ParseAction::Consume
                } else {
                    self.finish(parser, StatementType::Null)
                }
            },
            | StatementPhase::Finish(kind) => {
                debug_assert!(returned.is_none());
                self.finish(parser, kind)
            },
        }
    }

    fn enter_construct_scope(parser: &mut Parser, kind: ScopeKind) {
        parser.scopes.enter_scope(kind);
    }

    fn at_expression_boundary(token: Option<Token>, terminator: ExpressionTerminator) -> bool {
        token.is_none_or(|token| {
            SynchronizationKind::StatementExpression(terminator).stops_before(token.kind)
        })
    }

    fn missing_slot(parser: &Parser, context: &mut Context) -> ExpressionSlot {
        let position = parser.position(context);
        let source_file_index = parser.source_file_index();
        ExpressionSlot::Missing(context.create_source_vectors(position, source_file_index, 0))
    }

    fn merge_token(&mut self, parser: &Parser, context: &mut Context, token: Token) {
        parser.merge_source(context, &mut self.source_vectors, token);
    }

    fn merge_statement(
        &mut self,
        parser: &Parser,
        context: &mut Context,
        statement: StatementIndex,
    ) {
        let source = parser.statement_source(statement);
        self.source_vectors = Some(
            self.source_vectors
                .map_or(source, |existing| context.merge_vectors(existing, source)),
        );
    }

    fn future_slot(returned: Option<ParseValue>, kind: FutureChildKind) -> ExpressionSlot {
        let Some(ParseValue::FutureChild(FutureChildResult {
            kind: returned_kind,
            source_vectors,
        })) = returned
        else {
            panic!("statement expression returned an unexpected value: {returned:?}");
        };
        assert_eq!(returned_kind, kind);
        ExpressionSlot::FutureChild(source_vectors)
    }

    fn merge_slot(&mut self, parser: &Parser, context: &mut Context, slot: ExpressionSlot) {
        let source = match slot {
            | ExpressionSlot::Parsed(index) =>
                parser.syntax.expressions[index.0 as usize].source_vectors,
            | ExpressionSlot::FutureChild(source) | ExpressionSlot::Missing(source) => source,
        };
        if source.length > 0 {
            self.source_vectors = Some(
                self.source_vectors
                    .map_or(source, |existing| context.merge_vectors(existing, source)),
            );
        }
    }

    fn own_semicolon_or_report(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        position: &'static str,
    ) {
        if is_operator(token, OperatorTokenType::Semicolon) {
            self.merge_token(parser, context, token.expect("semicolon exists"));
        } else {
            parser.report(
                context,
                ParserErrorType::ExpectedSemicolonInStatement(
                    position,
                    token.map(|token| token.kind),
                ),
                token,
            );
        }
    }

    fn own_colon_or_report(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        position: &'static str,
    ) {
        if is_operator(token, OperatorTokenType::Colon) {
            self.merge_token(parser, context, token.expect("colon exists"));
        } else {
            parser.report(
                context,
                ParserErrorType::ExpectedColonInLabel(position, token.map(|token| token.kind)),
                token,
            );
        }
    }

    fn finish_existing(&mut self, parser: &mut Parser, statement: StatementIndex) -> ParseAction {
        self.restore_scopes(parser);
        ParseAction::Reduce(ParseValue::Statement(statement))
    }

    fn finish(&mut self, parser: &mut Parser, kind: StatementType) -> ParseAction {
        let index = parser.syntax.statements.len().to_u32();
        parser.syntax.statements.push(Statement {
            kind,
            source_vectors: self.source_vectors.unwrap_or_default(),
            recovered: parser.hard_error_count > self.starting_error_count,
        });
        self.restore_scopes(parser);
        ParseAction::Reduce(ParseValue::Statement(StatementIndex(index)))
    }

    fn restore_scopes(&mut self, parser: &mut Parser) {
        if self.owns_switch_scope {
            let _switch_scope = parser
                .switch_scopes
                .pop()
                .expect("switch statement owns its switch scope");
            self.owns_switch_scope = false;
        }
        parser.scopes.restore_depth(
            self.entry_scope_depth
                .expect("statement frame recorded its entry depth"),
        );
    }
}

impl HeaderKind {
    fn name(self) -> &'static str {
        match self {
            | Self::If => "if statement",
            | Self::Switch => "switch statement",
            | Self::While => "while statement",
        }
    }

    fn scope_kind(self) -> ScopeKind {
        match self {
            | Self::If | Self::Switch => ScopeKind::ImplicitSelection,
            | Self::While => ScopeKind::ImplicitIteration,
        }
    }
}

#[derive(Debug)]
struct FunctionDefinitionFrame {
    phase:                  FunctionDefinitionPhase,
    head:                   DeclarationIndex,
    old_style_declarations: Vec<DeclarationIndex>,
    body:                   Option<StatementIndex>,
    source_vectors:         Option<SourceVectors>,
    starting_error_count:   usize,
    entry_scope_depth:      Option<usize>,
}

#[derive(Debug, Clone, Copy)]
enum FunctionDefinitionPhase {
    Start,
    DeclarationOrBody,
    AwaitDeclaration,
    AwaitBody,
    Finish,
}

impl FunctionDefinitionFrame {
    fn new(head: DeclarationIndex, starting_error_count: usize) -> Self {
        Self {
            phase: FunctionDefinitionPhase::Start,
            head,
            old_style_declarations: Vec::new(),
            body: None,
            source_vectors: None,
            starting_error_count,
            entry_scope_depth: None,
        }
    }
}

#[derive(Debug)]
struct CompoundStatementFrame {
    phase:                CompoundStatementPhase,
    items:                Vec<BlockItem>,
    source_vectors:       Option<SourceVectors>,
    starting_error_count: usize,
    entry_scope_depth:    Option<usize>,
    function_body:        bool,
}

#[derive(Debug, Clone, Copy)]
enum CompoundStatementPhase {
    Start,
    ItemOrClose,
    AwaitDeclaration,
    AwaitStatement,
    Finish,
}

impl CompoundStatementFrame {
    fn new(starting_error_count: usize, function_body: bool) -> Self {
        Self {
            phase: CompoundStatementPhase::Start,
            items: Vec::new(),
            source_vectors: None,
            starting_error_count,
            entry_scope_depth: None,
            function_body,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderKind {
    If,
    Switch,
    While,
}

#[derive(Debug, Clone, Copy)]
enum LabelPrefix {
    Identifier(Identifier),
    Case(ExpressionSlot),
    Default,
}

#[derive(Debug, Clone, Copy)]
enum SimpleJump {
    Break,
    Continue,
}

#[derive(Debug, Clone, Copy)]
enum StatementPhase {
    Start,
    AwaitCompound,
    AwaitExpression,
    ExpressionSemicolon(ExpressionSlot),
    ReturnStart,
    AwaitReturnExpression,
    ReturnSemicolon(Option<ExpressionSlot>),
    SimpleJumpSemicolon(SimpleJump),
    GotoIdentifier,
    GotoSemicolon(Identifier),
    IdentifierLabelColon(Identifier),
    CaseExpression,
    AwaitCaseExpression,
    CaseColon(ExpressionSlot),
    DefaultColon,
    PushLabeled(LabelPrefix),
    AwaitLabeled(LabelPrefix),
    HeaderOpening(HeaderKind),
    HeaderExpression(HeaderKind),
    AwaitHeaderExpression(HeaderKind),
    HeaderClosing(HeaderKind, ExpressionSlot),
    PushHeaderBody(HeaderKind, ExpressionSlot),
    AwaitHeaderBody(HeaderKind, ExpressionSlot),
    IfAfterThen(ExpressionSlot, StatementIndex),
    PushElse(ExpressionSlot, StatementIndex),
    AwaitElse(ExpressionSlot, StatementIndex),
    DoPushBody,
    DoAwaitBody,
    DoWhileKeyword(StatementIndex),
    DoOpening(StatementIndex),
    DoExpression(StatementIndex),
    DoAwaitExpression(StatementIndex),
    DoClosing(StatementIndex, ExpressionSlot),
    DoSemicolon(StatementIndex, ExpressionSlot),
    ForOpening,
    ForInitializer,
    AwaitForInitializerExpression,
    AwaitForInitializerDeclaration,
    ForInitializerSemicolon(ExpressionSlot),
    ForCondition(Option<ForInitializer>),
    AwaitForCondition(Option<ForInitializer>),
    ForConditionSemicolon(Option<ForInitializer>, ExpressionSlot),
    ForIteration(Option<ForInitializer>, Option<ExpressionSlot>),
    AwaitForIteration(Option<ForInitializer>, Option<ExpressionSlot>),
    ForClosing(
        Option<ForInitializer>,
        Option<ExpressionSlot>,
        Option<ExpressionSlot>,
    ),
    ForPushBody(
        Option<ForInitializer>,
        Option<ExpressionSlot>,
        Option<ExpressionSlot>,
    ),
    ForAwaitBody(
        Option<ForInitializer>,
        Option<ExpressionSlot>,
        Option<ExpressionSlot>,
    ),
    Recovered,
    Finish(StatementType),
}

#[derive(Debug)]
struct StatementFrame {
    phase:                 StatementPhase,
    source_vectors:        Option<SourceVectors>,
    starting_error_count:  usize,
    entry_scope_depth:     Option<usize>,
    implicit_scope:        Option<ScopeKind>,
    owns_switch_scope:     bool,
    leave_else_unconsumed: bool,
}

impl StatementFrame {
    fn new(starting_error_count: usize, implicit_scope: Option<ScopeKind>) -> Self {
        Self {
            phase: StatementPhase::Start,
            source_vectors: None,
            starting_error_count,
            entry_scope_depth: None,
            implicit_scope,
            owns_switch_scope: false,
            leave_else_unconsumed: false,
        }
    }

    fn with_else_ownership(
        starting_error_count: usize,
        implicit_scope: Option<ScopeKind>,
        leave_else_unconsumed: bool,
    ) -> Self {
        Self {
            leave_else_unconsumed,
            ..Self::new(starting_error_count, implicit_scope)
        }
    }

    fn for_if_then(starting_error_count: usize, implicit_scope: Option<ScopeKind>) -> Self {
        Self::with_else_ownership(starting_error_count, implicit_scope, true)
    }
}

#[cfg(test)]
/// One observable driver action used to prove ownership and progress in tests.
///
/// C99: implementation instrumentation for the iterative grammar strategy;
/// deep nesting requirements are §5.2.4.1, pp. 20-21; PDF pp. 32-33.
#[derive(Debug, Clone, Copy)]
struct FrameTraceEvent {
    frame:  &'static str,
    action: &'static str,
    token:  Option<TokenType>,
    depth:  usize,
}

impl Parser {
    /// Creates an idle parser over a preprocessor token source.
    ///
    /// C99: the input is the translation unit produced after phase 7 under
    /// §5.1.1.1-§5.1.1.2, pp. 9-10; PDF pp. 21-22.
    pub(crate) fn new(preprocessor: Preprocessor) -> Self {
        Self {
            cursor: TokenCursor::new(preprocessor),
            frames: Vec::new(),
            returned: None,
            syntax: SyntaxStore::default(),
            scopes: ScopeStack::default(),
            label_scopes: Vec::new(),
            switch_scopes: Vec::new(),
            recovery: RecoveryState::default(),
            hard_error_count: 0,
            has_external_declaration: false,
            reported_empty_translation_unit: false,
            #[cfg(test)]
            trace: Vec::new(),
        }
    }

    /// Returns the complete arena-backed syntax store for diagnostic output.
    ///
    /// External declarations contain compact handles, so the CLI prints this
    /// view after the item stream to make those handles manually inspectable
    /// without exposing parser storage as part of the parser interface.
    pub(crate) fn syntax_debug(&self) -> impl Debug + '_ {
        &self.syntax
    }

    /// Runs owned frame actions until one external declaration reduces or EOF
    /// is observed between declarations.
    ///
    /// C99: translation-unit is a nonempty sequence of external-declaration
    /// values under §6.9, p. 140; PDF p. 152.
    fn drive(&mut self, context: &mut Context) -> Option<ExternalDeclaration> {
        loop {
            if matches!(self.returned, Some(ParseValue::ExternalDeclaration(_))) {
                let Some(ParseValue::ExternalDeclaration(external)) = self.returned.take() else {
                    unreachable!("the returned value was just checked")
                };
                self.has_external_declaration = true;
                return Some(external);
            }
            debug_assert!(
                self.returned.is_none() || !self.frames.is_empty(),
                "a child value must have a parent frame"
            );

            if self.frames.is_empty() {
                if self.cursor.current(context).is_none() {
                    if !self.has_external_declaration && !self.reported_empty_translation_unit {
                        self.reported_empty_translation_unit = true;
                        self.report(context, ParserErrorType::EmptyTranslationUnit, None);
                    }
                    return None;
                }
                self.frames.push(ParseFrame::ExternalDeclaration(
                    ExternalDeclarationFrame::new(self.hard_error_count),
                ));
            }

            let token = self.cursor.current(context);
            let mut frame = self.frames.pop().expect("parser frame stack is nonempty");
            let returned = self.returned.take();
            let FrameStep { frame_kind, action } = frame.step(self, context, token, returned);

            #[cfg(test)]
            self.trace.push(FrameTraceEvent {
                frame:  frame_kind.label(),
                action: action.name(),
                token:  token.map(|token| token.kind),
                depth:  self.frames.len() + 1,
            });

            match action {
                | ParseAction::Consume => {
                    self.frames.push(frame);
                    if token.is_some() {
                        self.cursor.consume();
                    } else {
                        self.report(
                            context,
                            ParserErrorType::ParserFrameConsumedAtEndOfInput(frame_kind),
                            None,
                        );
                    }
                },
                | ParseAction::Push(child) => {
                    self.frames.push(frame);
                    self.frames.push(child);
                },
                | ParseAction::Reduce(value) => {
                    self.returned = Some(value);
                },
                | ParseAction::Reprocess => {
                    self.frames.push(frame);
                },
                | ParseAction::Recover(set) => {
                    debug_assert!(
                        set.target == frame_kind,
                        "a recovery set must target the frame that requested it"
                    );

                    let recovered_source_vectors = {
                        #[cfg(test)]
                        {
                            let depth = self.frames.len() + 1;
                            self.recover(context, set, depth)
                        }
                        #[cfg(not(test))]
                        {
                            self.recover(context, set)
                        }
                    };
                    frame.merge_recovered_sources(context, recovered_source_vectors);
                    self.frames.push(frame);
                },
            }
        }
    }

    /// Consumes malformed input until the active synchronization policy says
    /// its owning frame can safely resume.
    ///
    /// C99: continued translation after a required diagnostic is permitted by
    /// §5.1.1.3 paragraph 1 and footnote 8, p. 11; PDF p. 23. The exact
    /// synchronization algorithm is implementation-defined.
    fn recover(
        &mut self,
        context: &mut Context,
        set: SynchronizationSet,
        #[cfg(test)] depth: usize,
    ) -> Option<SourceVectors> {
        let mut source_vectors = None;
        let mut consumed_tokens = 0_usize;
        self.recovery.begin(set);

        while let Some(token) = self.cursor.current(context) {
            let state = self.recovery.active();
            let recovery_set = state.set;
            let at_top_level = state.parentheses == 0 && state.brackets == 0 && state.braces == 0;
            let delimiter_depth = DelimiterDepth {
                parentheses: state.parentheses,
                brackets:    state.brackets,
                braces:      state.braces,
            };
            let colon_matches_conditional = token.kind
                == TokenType::Operator(OperatorTokenType::Colon)
                && state
                    .questions
                    .iter()
                    .rev()
                    .any(|question| *question == delimiter_depth);
            let at_unambiguous_owning_delimiter = !colon_matches_conditional
                && recovery_set.kind.stops_before_despite_unbalanced_child(
                    token.kind,
                    state.parentheses,
                    state.brackets,
                    state.braces,
                );
            let stops_at_initial_declaration = matches!(
                (recovery_set.kind, recovery_set.target),
                |(
                    SynchronizationKind::Declaration
                    | SynchronizationKind::BlockDeclaration
                    | SynchronizationKind::OldStyleParameter
                    | SynchronizationKind::Parameter,
                    _,
                )| (
                    SynchronizationKind::StructMember,
                    ParseFrameKind::StructOrUnionSpecifier
                ) | (
                    SynchronizationKind::EnumeratorValue,
                    ParseFrameKind::EnumSpecifier
                )
            );
            let stops_at_declaration_after_malformed_prefix = matches!(
                recovery_set.kind,
                SynchronizationKind::Initializer
                    | SynchronizationKind::ArrayBound
                    | SynchronizationKind::VariadicParameterList
                    | SynchronizationKind::StructMember
                    | SynchronizationKind::EnumeratorValue
                    | SynchronizationKind::StatementExpression(_)
            );
            let at_next_declaration = (stops_at_initial_declaration
                || consumed_tokens > 0 && stops_at_declaration_after_malformed_prefix)
                && at_top_level
                && self.declaration_starter(token);
            let at_next_k_and_r_identifier =
                matches!(recovery_set.kind, SynchronizationKind::KAndRParameter)
                    && at_top_level
                    && token.kind == TokenType::Identifier
                    && !self.scopes.is_typedef(token.contents);
            let at_next_enumerator =
                matches!(recovery_set.kind, SynchronizationKind::EnumeratorValue)
                    && at_top_level
                    && recovery_set.target == ParseFrameKind::EnumSpecifier
                    && token.kind == TokenType::Identifier;
            let at_next_identifier_label = matches!(
                recovery_set.kind,
                SynchronizationKind::StatementExpression(ExpressionTerminator::Semicolon)
            ) && at_top_level
                && token.kind == TokenType::Identifier
                && is_operator(self.cursor.following(context), OperatorTokenType::Colon);
            let at_statement_body_brace = matches!(
                recovery_set.kind,
                SynchronizationKind::StatementExpression(ExpressionTerminator::ClosingParenthesis)
            ) && at_top_level
                && token.kind == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                && state.last_token
                    != Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis));
            if at_unambiguous_owning_delimiter
                || at_next_declaration
                || at_next_k_and_r_identifier
                || at_next_enumerator
                || at_next_identifier_label
                || at_statement_body_brace
                || at_top_level
                    && !colon_matches_conditional
                    && recovery_set.kind.stops_before(token.kind)
            {
                break;
            }

            self.recovery.consume(token.kind);

            #[cfg(test)]
            self.trace.push(FrameTraceEvent {
                frame: set.target.label(),
                action: "recover-consume",
                token: Some(token.kind),
                depth,
            });
            self.merge_source(context, &mut source_vectors, token);
            self.cursor.consume();
            consumed_tokens += 1;
        }
        self.recovery.finish();
        source_vectors
    }

    /// Emits one structured diagnostic and records whether it was a hard error.
    ///
    /// C99: syntax-rule and constraint violations require at least one
    /// diagnostic under §5.1.1.3, p. 11; PDF p. 23: implementations must
    /// “produce at least one diagnostic message”.
    fn report(&mut self, context: &mut Context, error_type: ParserErrorType, token: Option<Token>) {
        if error_type.severity() == ErrorSeverity::Error {
            self.hard_error_count += 1;
        }
        let source_vectors = token.map_or_else(
            || context.create_source_vectors(self.position(context), self.source_file_index(), 0),
            |token| token.source_vectors,
        );
        context.parser_error(ParserError {
            error_type,
            source_vectors,
        });
    }

    /// Adds a token's provenance to an optional accumulated source range.
    ///
    /// C99: diagnostics should identify the violation where possible under
    /// §5.1.1.3 and footnote 8, p. 11; PDF p. 23. `SourceVectors` is the
    /// implementation's macro/include provenance mechanism.
    #[expect(
        clippy::unused_self,
        reason = "Source accumulation is a parser-machine operation used by every frame."
    )]
    fn merge_source(
        &self,
        context: &mut Context,
        existing: &mut Option<SourceVectors>,
        token: Token,
    ) {
        *existing = Some(existing.map_or(token.source_vectors, |source_vectors| {
            context.merge_vectors(source_vectors, token.source_vectors)
        }));
    }

    /// Reports whether `token` can begin declaration specifiers in the current
    /// typedef environment.
    ///
    /// C99: declaration-specifiers are §6.7, p. 97; PDF p. 109; typedef-name
    /// is a type-specifier under §6.7.2, p. 99; PDF p. 111.
    fn declaration_starter(&self, token: Token) -> bool {
        match token.kind {
            | TokenType::Keyword(
                KeywordTokenType::Auto
                | KeywordTokenType::Char
                | KeywordTokenType::Complex
                | KeywordTokenType::Const
                | KeywordTokenType::Double
                | KeywordTokenType::Enum
                | KeywordTokenType::Extern
                | KeywordTokenType::Float
                | KeywordTokenType::Imaginary
                | KeywordTokenType::Inline
                | KeywordTokenType::Int
                | KeywordTokenType::Long
                | KeywordTokenType::Register
                | KeywordTokenType::Restrict
                | KeywordTokenType::Short
                | KeywordTokenType::Signed
                | KeywordTokenType::Static
                | KeywordTokenType::Struct
                | KeywordTokenType::Typedef
                | KeywordTokenType::Union
                | KeywordTokenType::Unsigned
                | KeywordTokenType::Void
                | KeywordTokenType::Volatile
                | KeywordTokenType::Bool,
            ) => true,
            | TokenType::Identifier => self.scopes.is_typedef(token.contents),
            | _ => false,
        }
    }

    /// Resolves the declaration-specifier/declarator ambiguity after a visible
    /// typedef name using buffered lookahead.
    ///
    /// C99: typedef-name is §6.7.7, pp. 123-124; PDF pp. 135-136, and its
    /// declarator ambiguity is constrained by §6.7.5.3 paragraph 11,
    /// p. 119; PDF p. 131.
    fn typedef_name_continues_specifiers(&mut self, context: &mut Context) -> bool {
        let Some(following) = self.cursor.following(context) else {
            return false;
        };
        following.kind == TokenType::Identifier
            || is_operator(Some(following), OperatorTokenType::Asterisk)
            || self.parenthesized_declarator_follows_typedef(context)
            || self.declaration_starter(following)
    }

    /// Detects the parenthesized-pointer shape that forces a typedef spelling
    /// to remain a specifier rather than become the declarator name.
    ///
    /// C99: parenthesized direct-declarator and pointer are §6.7.5,
    /// p. 114; PDF p. 126; typedef-name is §6.7.7, pp. 123-124;
    /// PDF pp. 135-136.
    fn parenthesized_declarator_follows_typedef(&mut self, context: &mut Context) -> bool {
        let mut index = 0;
        while is_operator(
            self.cursor.lookahead(context, index),
            OperatorTokenType::OpeningParenthesis,
        ) {
            index += 1;
        }
        is_operator(
            self.cursor.lookahead(context, index),
            OperatorTokenType::Asterisk,
        )
    }

    /// Finds the identifier declared by nested parenthesized direct
    /// declarators.
    ///
    /// C99: declarator binding is specified by §6.7.5 paragraph 4,
    /// p. 114; PDF p. 126.
    fn declarator_identifier(&self, declarator: Declarator) -> Option<Identifier> {
        let mut declarator = declarator;
        loop {
            let start = declarator.kind.start_index as usize;
            let end = start + declarator.kind.length as usize;
            let mut nested = None;
            for direct in &self.syntax.direct_declarators[start..end] {
                match *direct {
                    | DirectDeclarator::Identifier(identifier) => return Some(identifier),
                    | DirectDeclarator::Parenthesized(child) => nested = Some(child),
                    | _ => {},
                }
            }
            declarator = nested?;
        }
    }

    /// Reports whether the identifier binding produced by `declarator` is a
    /// function rather than an object or pointer.
    ///
    /// C99: derived declarator interpretation is §6.7.5 paragraph 4,
    /// p. 114; PDF p. 126; function declarators are §6.7.5.3,
    /// pp. 118-121; PDF pp. 130-133.
    fn declarator_declares_function(&self, declarator: Declarator) -> bool {
        self.declarator_function_binding(declarator) == Some(true)
    }

    /// Walks parenthesized declarators to find the function suffix bound to the
    /// declared identifier rather than a function type returned by it.
    ///
    /// C99: parenthesized declarator binding follows §6.7.5 paragraph 4,
    /// p. 114; PDF p. 126.
    fn declarator_function_binding(&self, declarator: Declarator) -> Option<bool> {
        let mut declarator = declarator;
        let mut binding = None;

        loop {
            let start = declarator.kind.start_index as usize;
            let end = start + declarator.kind.length as usize;
            let direct = &self.syntax.direct_declarators[start..end];

            let local_binding = direct.get(1).map(|suffix| {
                matches!(
                    suffix,
                    DirectDeclarator::Function { .. } | DirectDeclarator::KAndRStyleFunction { .. }
                )
            });
            binding = local_binding
                .or_else(|| (declarator.pointer.type_qualifiers_list.length > 0).then_some(false))
                .or(binding);

            let Some(DirectDeclarator::Parenthesized(nested)) = direct.first() else {
                return binding;
            };
            declarator = *nested;
        }
    }

    fn declaration_head_declarator(&self, declaration: DeclarationIndex) -> Option<Declarator> {
        let declaration = self.syntax.declarations.get(declaration.0 as usize)?;
        if declaration.init_declarators.length != 1 {
            return None;
        }
        let init = self
            .syntax
            .init_declarators
            .get(declaration.init_declarators.start_index as usize)?;
        init.initializer.is_none().then_some(init.declarator)
    }

    fn declaration_is_function_head(&self, declaration: DeclarationIndex) -> bool {
        self.syntax.declarations[declaration.0 as usize].is_function_definition_head
            && self
                .declaration_head_declarator(declaration)
                .is_some_and(|declarator| self.declarator_declares_function(declarator))
    }

    fn declaration_is_old_style_function_head(&self, declaration: DeclarationIndex) -> bool {
        self.declaration_head_declarator(declaration)
            .and_then(|declarator| self.function_suffix(declarator))
            .is_some_and(|suffix| matches!(suffix, DirectDeclarator::KAndRStyleFunction { .. }))
    }

    fn function_suffix(&self, mut declarator: Declarator) -> Option<DirectDeclarator> {
        let mut suffix = None;
        loop {
            let start = declarator.kind.start_index as usize;
            let end = start + declarator.kind.length as usize;
            let direct = &self.syntax.direct_declarators[start..end];
            if let Some(candidate) = direct.get(1).copied()
                && matches!(
                    candidate,
                    DirectDeclarator::Function { .. } | DirectDeclarator::KAndRStyleFunction { .. }
                )
            {
                suffix = Some(candidate);
            }
            let Some(DirectDeclarator::Parenthesized(nested)) = direct.first() else {
                return suffix;
            };
            declarator = *nested;
        }
    }

    fn collect_type_specifier_bindings(
        &self,
        type_specifiers: TypeSpecifiers,
        names: &mut Vec<StringCacheId>,
    ) {
        let mut pending = vec![type_specifiers];
        while let Some(type_specifiers) = pending.pop() {
            match type_specifiers {
                | TypeSpecifiers::Enum(index) => {
                    if let Some(enumeration_list) =
                        self.syntax.enum_specifiers[index.0 as usize].enumeration_list
                    {
                        let start = enumeration_list.start_index as usize;
                        let end = start + enumeration_list.length as usize;
                        names.extend(
                            self.syntax.enumerators[start..end]
                                .iter()
                                .map(|enumerator| enumerator.name.name),
                        );
                    }
                },
                | TypeSpecifiers::StructOrUnion(index) => {
                    if let Some(declarations) = self.syntax.struct_or_union_specifiers
                        [index.0 as usize]
                        .struct_declaration_list
                    {
                        let start = declarations.start_index as usize;
                        let end = start + declarations.length as usize;
                        pending.extend(
                            self.syntax.struct_declarations[start..end]
                                .iter()
                                .map(|declaration| declaration.type_specifiers),
                        );
                    }
                },
                | _ => {},
            }
        }
    }

    fn statement_source(&self, index: StatementIndex) -> SourceVectors {
        self.syntax.statements[index.0 as usize].source_vectors
    }
}

impl ParseAction {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        match self {
            | Self::Consume => "consume",
            | Self::Push(_) => "push",
            | Self::Reduce(_) => "reduce",
            | Self::Reprocess => "reprocess",
            | Self::Recover(_) => "recover",
        }
    }
}

impl SynchronizationKind {
    /// Returns whether a balanced recovery scan must stop before `token`.
    fn stops_before(self, token: TokenType) -> bool {
        match self {
            | Self::Declaration => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::BlockDeclaration =>
                is_statement_keyword(token)
                    || matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Comma
                                | OperatorTokenType::Semicolon
                                | OperatorTokenType::ClosingCurlyBrace
                        )
                    ),
            | Self::OldStyleParameter => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::OpeningCurlyBrace
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::ForInitializer => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::Initializer => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::ArrayBound => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::ClosingSquareBracket
                        | OperatorTokenType::Semicolon
                )
            ),
            | Self::Parameter | Self::KAndRParameter => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                        | OperatorTokenType::Semicolon
                )
            ),
            | Self::VariadicParameterList | Self::VariadicTrailingParameter => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                        | OperatorTokenType::Semicolon
                )
            ),
            | Self::StructMember => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::EnumeratorValue => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::StatementExpression(terminator) =>
                is_statement_keyword(token)
                    || match terminator {
                        | ExpressionTerminator::Semicolon => matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                        | ExpressionTerminator::ForSemicolon => matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingParenthesis
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                        | ExpressionTerminator::ClosingParenthesis => matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::ClosingParenthesis
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                        | ExpressionTerminator::Colon => matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Colon
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                    },
            | Self::Statement => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                )
            ),
        }
    }

    /// Returns whether an enclosing delimiter is unambiguously owned by the
    /// caller even when malformed child delimiters remain unbalanced.
    fn stops_before_despite_unbalanced_child(
        self,
        token: TokenType,
        parentheses: usize,
        brackets: usize,
        braces: usize,
    ) -> bool {
        match self {
            | Self::Declaration | Self::Initializer =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    ),
            | Self::BlockDeclaration =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    ),
            | Self::OldStyleParameter =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                    || parentheses == 0
                        && brackets == 0
                        && braces == 0
                        && token == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace),
            | Self::ForInitializer =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                    || parentheses == 0
                        && token == TokenType::Operator(OperatorTokenType::ClosingParenthesis),
            | Self::EnumeratorValue =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                    || parentheses == 0
                        && token == TokenType::Operator(OperatorTokenType::ClosingParenthesis),
            | Self::StructMember =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                    || parentheses == 0
                        && token == TokenType::Operator(OperatorTokenType::ClosingParenthesis),
            | Self::ArrayBound =>
                braces == 0 && token == TokenType::Operator(OperatorTokenType::Semicolon)
                    || brackets == 0
                        && token == TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                    || parentheses == 0
                        && token == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    || braces == 0
                        && token == TokenType::Operator(OperatorTokenType::ClosingCurlyBrace),
            | Self::Parameter
            | Self::KAndRParameter
            | Self::VariadicParameterList
            | Self::VariadicTrailingParameter =>
                braces == 0 && token == TokenType::Operator(OperatorTokenType::Semicolon)
                    || parentheses == 0
                        && token == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    || braces == 0
                        && token == TokenType::Operator(OperatorTokenType::ClosingCurlyBrace),
            | Self::StatementExpression(terminator) => match terminator {
                | ExpressionTerminator::Semicolon =>
                    braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                | ExpressionTerminator::ForSemicolon =>
                    braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                            )
                        )
                        || parentheses == 0
                            && token == TokenType::Operator(OperatorTokenType::ClosingParenthesis),
                | ExpressionTerminator::ClosingParenthesis =>
                    parentheses == 0
                        && braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::ClosingParenthesis
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                | ExpressionTerminator::Colon =>
                    parentheses == 0
                        && brackets == 0
                        && braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Colon
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
            },
            | Self::Statement =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    ),
        }
    }
}

/// Tests whether an optional token is a particular C punctuator.
///
/// C99: punctuators are §6.4.6, pp. 63-64; PDF pp. 75-76.
fn is_operator(token: Option<Token>, operator: OperatorTokenType) -> bool {
    token.is_some_and(|token| token.kind == TokenType::Operator(operator))
}

/// Returns whether a token begins a statement production rather than an
/// expression that the deferred expression frame could consume.
fn is_statement_keyword(token: TokenType) -> bool {
    matches!(
        token,
        TokenType::Keyword(
            KeywordTokenType::Break
                | KeywordTokenType::Case
                | KeywordTokenType::Continue
                | KeywordTokenType::Default
                | KeywordTokenType::Do
                | KeywordTokenType::Else
                | KeywordTokenType::For
                | KeywordTokenType::Goto
                | KeywordTokenType::If
                | KeywordTokenType::Return
                | KeywordTokenType::Switch
                | KeywordTokenType::While
        )
    )
}

impl ParseFrame {
    /// Adds tokens consumed by recovery to the syntax object owned by this
    /// frame; frames without an owned syntax range intentionally ignore them.
    fn merge_recovered_sources(&mut self, context: &mut Context, recovered: Option<SourceVectors>) {
        let Some(recovered) = recovered else {
            return;
        };
        let destination = match self {
            | Self::ExternalDeclaration(_) | Self::DeclarationSpecifiers(_) => return,
            | Self::Declaration(frame) => &mut frame.source_vectors,
            | Self::Declarator(frame) => &mut frame.source_vectors,
            | Self::ParameterList(frame) => &mut frame.source_vectors,
            | Self::StructOrUnionSpecifier(frame) => &mut frame.source_vectors,
            | Self::EnumSpecifier(frame) => &mut frame.source_vectors,
            | Self::FunctionDefinition(frame) => &mut frame.source_vectors,
            | Self::CompoundStatement(frame) => &mut frame.source_vectors,
            | Self::Statement(frame) => &mut frame.source_vectors,
            | Self::FutureChild(frame) => &mut frame.recovered_source_vectors,
        };
        *destination = Some(destination.map_or(recovered, |existing| {
            context.merge_vectors(existing, recovered)
        }));
    }

    /// Dispatches one transition to the concrete active frame and attaches its
    /// typed identity for tracing, recovery validation, and invariants.
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> FrameStep {
        match self {
            | Self::ExternalDeclaration(frame) => FrameStep {
                frame_kind: ParseFrameKind::ExternalDeclaration,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::Declaration(frame) => FrameStep {
                frame_kind: ParseFrameKind::Declaration,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::DeclarationSpecifiers(frame) => FrameStep {
                frame_kind: ParseFrameKind::DeclarationSpecifiers,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::Declarator(frame) => FrameStep {
                frame_kind: ParseFrameKind::Declarator,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::ParameterList(frame) => FrameStep {
                frame_kind: ParseFrameKind::ParameterList,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::StructOrUnionSpecifier(frame) => FrameStep {
                frame_kind: ParseFrameKind::StructOrUnionSpecifier,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::EnumSpecifier(frame) => FrameStep {
                frame_kind: ParseFrameKind::EnumSpecifier,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::FunctionDefinition(frame) => FrameStep {
                frame_kind: ParseFrameKind::FunctionDefinition,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::CompoundStatement(frame) => FrameStep {
                frame_kind: ParseFrameKind::CompoundStatement,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::Statement(frame) => FrameStep {
                frame_kind: ParseFrameKind::Statement,
                action:     frame.step(parser, context, token, returned),
            },
            | Self::FutureChild(frame) => FrameStep {
                frame_kind: ParseFrameKind::FutureChild(frame.kind),
                action:     frame.step(parser, context, token, returned),
            },
        }
    }
}

impl FutureChildFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        debug_assert!(
            returned.is_none(),
            "future-child frames cannot receive child values"
        );
        match self.phase {
            | FutureChildPhase::Start => {
                parser.report(context, self.kind.not_implemented_error(), token);
                self.phase = FutureChildPhase::Recovered;
                let kind = match self.kind {
                    | FutureChildKind::ArrayBoundExpression => SynchronizationKind::ArrayBound,
                    | FutureChildKind::BitFieldWidthExpression => SynchronizationKind::StructMember,
                    | FutureChildKind::EnumeratorValueExpression =>
                        SynchronizationKind::EnumeratorValue,
                    | FutureChildKind::Initializer => self
                        .recovery_kind
                        .unwrap_or(SynchronizationKind::Initializer),
                    | FutureChildKind::StatementExpression
                    | FutureChildKind::StatementConstantExpression =>
                        SynchronizationKind::StatementExpression(
                            self.expression_terminator
                                .expect("statement expressions have a caller stop token"),
                        ),
                };
                ParseAction::Recover(SynchronizationSet {
                    kind,
                    target: ParseFrameKind::FutureChild(self.kind),
                })
            },
            | FutureChildPhase::Recovered =>
                ParseAction::Reduce(ParseValue::FutureChild(FutureChildResult {
                    kind:           self.kind,
                    source_vectors: self.recovered_source_vectors.take().unwrap_or_default(),
                })),
        }
    }
}

impl ExternalDeclarationFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        _context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | ExternalDeclarationPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = ExternalDeclarationPhase::AwaitDeclaration;
                ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                    parser.syntax.init_declarators.len().to_u32(),
                    DeclarationContext::External,
                )))
            },
            | ExternalDeclarationPhase::AwaitDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("declaration frame returned an unexpected value: {returned:?}");
                };
                let is_definition = parser.declaration_is_function_head(declaration)
                    && (is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                        || parser.declaration_is_old_style_function_head(declaration)
                            && token.is_some_and(|token| parser.declaration_starter(token)));
                if is_definition {
                    self.phase = ExternalDeclarationPhase::AwaitFunctionDefinition;
                    return ParseAction::Push(ParseFrame::FunctionDefinition(
                        FunctionDefinitionFrame::new(declaration, self.starting_error_count),
                    ));
                }
                if parser.hard_error_count > self.starting_error_count {
                    return ParseAction::Reduce(ParseValue::ExternalDeclaration(
                        ExternalDeclaration::RecoveredDeclaration(declaration),
                    ));
                }
                ParseAction::Reduce(ParseValue::ExternalDeclaration(
                    ExternalDeclaration::Declaration(declaration),
                ))
            },
            | ExternalDeclarationPhase::AwaitFunctionDefinition => {
                let Some(ParseValue::FunctionDefinition(definition)) = returned else {
                    panic!("function-definition frame returned an unexpected value: {returned:?}");
                };
                let recovered = parser.syntax.function_definitions[definition.0 as usize].recovered;
                ParseAction::Reduce(ParseValue::ExternalDeclaration(if recovered {
                    ExternalDeclaration::RecoveredFunctionDefinition(definition)
                } else {
                    ExternalDeclaration::FunctionDefinition(definition)
                }))
            },
        }
    }
}

impl DeclarationFrame {
    fn recovery_kind(&self) -> SynchronizationKind {
        match self.context {
            | DeclarationContext::Block => SynchronizationKind::BlockDeclaration,
            | DeclarationContext::ForInitializer => SynchronizationKind::ForInitializer,
            | DeclarationContext::External => SynchronizationKind::Declaration,
            | DeclarationContext::OldStyleParameter => SynchronizationKind::OldStyleParameter,
        }
    }

    fn initializer_recovery_kind(&self) -> SynchronizationKind {
        match self.context {
            | DeclarationContext::Block => SynchronizationKind::BlockDeclaration,
            | DeclarationContext::ForInitializer => SynchronizationKind::ForInitializer,
            | DeclarationContext::External => SynchronizationKind::Initializer,
            | DeclarationContext::OldStyleParameter => SynchronizationKind::OldStyleParameter,
        }
    }

    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | DeclarationPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Specifiers are a child production because tag specifiers may
                // suspend again for complete struct/union/enum bodies.
                self.phase = DeclarationPhase::AwaitSpecifiers;
                ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                    DeclarationSpecifiersFrame::new(SpecifierMode::Declaration),
                ))
            },
            | DeclarationPhase::AwaitSpecifiers => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    panic!("specifier frame returned an unexpected value: {returned:?}");
                };
                self.declaration_specifiers = Some(specifiers);
                self.source_vectors = Some(specifiers.source_vectors);
                // A bare `;` completes the grammar's optional
                // init-declarator-list. A typedef is the exception: it must
                // still introduce a name, so retain the tree but diagnose it.
                if is_operator(token, OperatorTokenType::Semicolon) {
                    if specifiers.storage_class == StorageClass::Typedef {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedDeclaratorInTypedef(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                    }
                    if let Some(token) = token {
                        parser.merge_source(context, &mut self.source_vectors, token);
                    }
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Consume
                } else {
                    self.phase = DeclarationPhase::AwaitDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        DeclaratorMode::Named,
                    )))
                }
            },
            | DeclarationPhase::AwaitDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("declarator frame returned an unexpected value: {returned:?}");
                };
                let Some(declarator) = declarator else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclaratorInDeclaration(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::AfterMissingDeclarator;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   self.recovery_kind(),
                        target: ParseFrameKind::Declaration,
                    });
                };

                let source_vectors = declarator.source_vectors;
                self.source_vectors =
                    Some(self.source_vectors.map_or(source_vectors, |existing| {
                        context.merge_vectors(existing, source_vectors)
                    }));
                let init_index = parser.syntax.init_declarators.len().to_u32();
                parser.syntax.init_declarators.push(InitDeclarator {
                    declarator,
                    initializer: None,
                    source_vectors,
                });
                self.last_init_index = Some(init_index);

                // C scope begins immediately after the declarator, before its
                // initializer or a later comma-separated declarator. Publish
                // now so typedef shadowing affects the very next token.
                if let Some(identifier) = parser.declarator_identifier(declarator) {
                    let class = if self
                        .declaration_specifiers
                        .is_some_and(|specifiers| specifiers.storage_class == StorageClass::Typedef)
                    {
                        NameClass::Typedef
                    } else {
                        NameClass::Ordinary
                    };
                    parser.scopes.publish(identifier.name, class);
                }
                self.phase = DeclarationPhase::AfterDeclarator;
                ParseAction::Reprocess
            },
            | DeclarationPhase::AfterMissingDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Recovery stops before owning separators. Consume them here;
                // leave any unrelated token untouched for the parent frame.
                if is_operator(token, OperatorTokenType::Comma) {
                    let token = token.expect("comma token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclarationPhase::BeforeNextDeclarator;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    let token = token.expect("semicolon token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    self.phase = DeclarationPhase::Finish;
                    if self.context == DeclarationContext::External {
                        let token = token.expect("closing-curly-brace token exists");
                        parser.merge_source(context, &mut self.source_vectors, token);
                        ParseAction::Consume
                    } else {
                        ParseAction::Reprocess
                    }
                } else {
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                }
            },
            | DeclarationPhase::AfterDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                let has_sole_uninitialized_function_declarator =
                    parser.syntax.init_declarators.len().to_u32() == self.init_declarator_start + 1
                        && self
                            .last_init_index
                            .and_then(|index| parser.syntax.init_declarators.get(index as usize))
                            .is_some_and(|init| {
                                init.initializer.is_none()
                                    && parser.declarator_declares_function(init.declarator)
                            });
                let is_old_style_function_declarator = self
                    .last_init_index
                    .and_then(|index| parser.syntax.init_declarators.get(index as usize))
                    .is_some_and(|init| {
                        parser
                            .function_suffix(init.declarator)
                            .is_some_and(|suffix| {
                                matches!(suffix, DirectDeclarator::KAndRStyleFunction { .. })
                            })
                    });
                let starts_function_definition = self.context == DeclarationContext::External
                    && has_sole_uninitialized_function_declarator
                    && (is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                        || is_old_style_function_declarator
                            && token.is_some_and(|token| parser.declaration_starter(token)));
                // The same prefix can continue as another init-declarator, an
                // initializer, a completed declaration, or a function body.
                // Declarator binding decides whether `{` is legal here.
                if is_operator(token, OperatorTokenType::Comma) {
                    if let Some(token) = token {
                        parser.merge_source(context, &mut self.source_vectors, token);
                    }
                    self.phase = DeclarationPhase::BeforeNextDeclarator;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    if let Some(token) = token {
                        parser.merge_source(context, &mut self.source_vectors, token);
                    }
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Equals) {
                    self.initializer_source = token.map(|token| token.source_vectors);
                    if let Some(token) = token {
                        parser.merge_source(context, &mut self.source_vectors, token);
                    }
                    self.phase = DeclarationPhase::PushInitializer;
                    ParseAction::Consume
                } else if starts_function_definition {
                    self.is_function_definition_head = true;
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if self.context == DeclarationContext::ForInitializer
                    && is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if self.context == DeclarationContext::OldStyleParameter
                    && is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    if self.context == DeclarationContext::External {
                        if let Some(token) = token {
                            parser.merge_source(context, &mut self.source_vectors, token);
                        }
                        ParseAction::Consume
                    } else {
                        ParseAction::Reprocess
                    }
                } else if token.is_some_and(|token| {
                    parser.declaration_starter(token)
                        || self.context == DeclarationContext::Block
                            && is_statement_keyword(token.kind)
                }) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(None),
                        None,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::AfterDeclarator;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   self.recovery_kind(),
                        target: ParseFrameKind::Declaration,
                    })
                }
            },
            | DeclarationPhase::PushInitializer => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = DeclarationPhase::AwaitInitializer;
                ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::with_recovery(
                    FutureChildKind::Initializer,
                    self.initializer_recovery_kind(),
                )))
            },
            | DeclarationPhase::AwaitInitializer => {
                let Some(ParseValue::FutureChild(FutureChildResult {
                    kind: FutureChildKind::Initializer,
                    source_vectors,
                })) = returned
                else {
                    panic!("initializer seam returned an unexpected value: {returned:?}");
                };
                // Even while initializer parsing is deferred, attach its
                // recovered provenance to both the child placeholder and the
                // containing init-declarator/declaration.
                let initializer_source = self
                    .initializer_source
                    .map_or(source_vectors, |equals_source| {
                        context.merge_vectors(equals_source, source_vectors)
                    });
                self.source_vectors =
                    Some(self.source_vectors.map_or(source_vectors, |existing| {
                        context.merge_vectors(existing, source_vectors)
                    }));
                if let Some(index) = self.last_init_index {
                    let init_declarator = &mut parser.syntax.init_declarators[index as usize];
                    init_declarator.initializer =
                        Some(Initializer::FutureChild(initializer_source));
                    init_declarator.source_vectors =
                        context.merge_vectors(init_declarator.source_vectors, initializer_source);
                }
                self.phase = DeclarationPhase::AfterDeclarator;
                ParseAction::Reprocess
            },
            | DeclarationPhase::BeforeNextDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = DeclarationPhase::AwaitDeclarator;
                ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                    DeclaratorMode::Named,
                )))
            },
            | DeclarationPhase::Finish => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Arena insertion is the reduction boundary: all child slices
                // and source ranges are stable before the handle is returned.
                let index = parser.syntax.declarations.len().to_u32();
                let source_vectors = self.source_vectors.unwrap_or_default();
                parser.syntax.declarations.push(Declaration {
                    declaration_specifiers: self
                        .declaration_specifiers
                        .expect("a declaration cannot finish without specifiers"),
                    init_declarators: VectorSlice::new(
                        self.init_declarator_start,
                        parser.syntax.init_declarators.len().to_u32(),
                    ),
                    source_vectors,
                    is_function_definition_head: self.is_function_definition_head,
                });
                ParseAction::Reduce(ParseValue::Declaration(DeclarationIndex(index)))
            },
        }
    }
}

impl DeclarationSpecifiersFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | DeclarationSpecifiersPhase::AwaitStructOrUnion => {
                let Some(ParseValue::StructOrUnionSpecifier(index)) = returned else {
                    panic!("struct specifier returned an unexpected value: {returned:?}");
                };
                let token = self
                    .pending_type_specifier
                    .take()
                    .expect("struct-or-union child follows its keyword");
                // The child consumed the entire tag specifier. Merge it into
                // the normalized type set, then reprocess the untouched token
                // that follows the child.
                self.specifiers
                    .type_specifiers
                    .make_struct_or_union(parser, context, index, token);
                if let Some(source_vectors) = parser
                    .syntax
                    .struct_or_union_specifiers
                    .get(index.0 as usize)
                    .map(|specifier| specifier.source_vectors)
                {
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }));
                }
                self.consumed = true;
                self.phase = DeclarationSpecifiersPhase::Collect;
                return ParseAction::Reprocess;
            },
            | DeclarationSpecifiersPhase::AwaitEnum => {
                let Some(ParseValue::EnumSpecifier(EnumSpecifierResult {
                    index,
                    stopped_before_declaration,
                })) = returned
                else {
                    panic!("enum specifier returned an unexpected value: {returned:?}");
                };
                let token = self
                    .pending_type_specifier
                    .take()
                    .expect("enum child follows its keyword");
                // As above, the enum child owns its delimiters and returns on
                // the first token belonging to this specifier sequence.
                self.specifiers
                    .type_specifiers
                    .make_enum(parser, context, index, token);
                if let Some(source_vectors) = parser
                    .syntax
                    .enum_specifiers
                    .get(index.0 as usize)
                    .map(|specifier| specifier.source_vectors)
                {
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }));
                }
                self.consumed = true;
                if stopped_before_declaration {
                    self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
                    return ParseAction::Reduce(ParseValue::DeclarationSpecifiers(self.specifiers));
                }
                self.phase = DeclarationSpecifiersPhase::Collect;
                return ParseAction::Reprocess;
            },
            | DeclarationSpecifiersPhase::Collect => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
            },
        }

        // EOF has two distinct failures. Keep them mutually exclusive so one
        // absent production does not cause a cascade of specifier diagnostics.
        let Some(token) = token else {
            if !self.consumed {
                parser.report(
                    context,
                    ParserErrorType::UnexpectedEndBeforeDeclarationSpecifier,
                    None,
                );
            } else if self.specifiers.type_specifiers == TypeSpecifiers::Empty {
                parser.report(
                    context,
                    ParserErrorType::UnexpectedEndBeforeTypeSpecifier,
                    None,
                );
            }
            self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
            return ParseAction::Reduce(ParseValue::DeclarationSpecifiers(self.specifiers));
        };

        if matches!(
            token.kind,
            TokenType::Keyword(KeywordTokenType::Struct | KeywordTokenType::Union)
        ) {
            // Keep the keyword for diagnostics/type-conflict provenance; the
            // child frame consumes it as the first token it owns.
            self.pending_type_specifier = Some(token);
            self.phase = DeclarationSpecifiersPhase::AwaitStructOrUnion;
            return ParseAction::Push(ParseFrame::StructOrUnionSpecifier(
                StructOrUnionSpecifierFrame::new(),
            ));
        }
        if token.kind == TokenType::Keyword(KeywordTokenType::Enum) {
            self.pending_type_specifier = Some(token);
            self.phase = DeclarationSpecifiersPhase::AwaitEnum;
            return ParseAction::Push(ParseFrame::EnumSpecifier(EnumSpecifierFrame::new()));
        }

        if self.mode == SpecifierMode::Declaration
            && let Some(storage_class) = storage_class(token.kind)
        {
            if self.storage_seen {
                parser.report(
                    context,
                    ParserErrorType::StorageClassRedefinition(
                        self.specifiers.storage_class,
                        token.kind,
                    ),
                    Some(token),
                );
            }
            self.specifiers.storage_class = storage_class;
            self.storage_seen = true;
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if let TokenType::Keyword(keyword) = token.kind
            && let Some(specifier) = primitive_type_specifier(keyword)
        {
            self.apply_type_specifier(parser, context, token, specifier);
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if let Some(qualifier) = type_qualifier(token.kind) {
            if self.specifiers.type_qualifiers.contains(qualifier) {
                let error_type = match qualifier {
                    | TypeQualifiers::CONST => ParserErrorType::ConstSpecifiedTwice,
                    | TypeQualifiers::VOLATILE => ParserErrorType::VolatileSpecifiedTwice,
                    | TypeQualifiers::RESTRICT => ParserErrorType::RestrictSpecifiedTwice,
                    | _ => unreachable!("one type qualifier is handled at a time"),
                };
                parser.report(context, error_type, Some(token));
            }
            self.specifiers.type_qualifiers.insert(qualifier);
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if self.mode == SpecifierMode::Declaration
            && token.kind == TokenType::Keyword(KeywordTokenType::Inline)
        {
            if self.specifiers.function_specifiers.is_inline {
                parser.report(context, ParserErrorType::InlineSpecifiedTwice, Some(token));
            }
            self.specifiers.function_specifiers.is_inline = true;
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if token.kind == TokenType::Identifier
            && parser.scopes.is_typedef(token.contents)
            && (self.specifiers.type_specifiers == TypeSpecifiers::Empty
                || parser.typedef_name_continues_specifiers(context))
        {
            // A visible typedef spelling is still allowed to become the
            // declarator name. Consume it as a specifier only when no type is
            // present yet or lookahead proves another declarator follows.
            self.specifiers.type_specifiers.make_typedef_name(
                parser,
                context,
                Identifier::new(token.contents),
                token,
            );
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        // The first non-specifier belongs to the parent. Finalize without
        // consuming it, while reporting any missing mandatory component.
        if !self.consumed {
            parser.report(
                context,
                ParserErrorType::EmptyDeclarationSpecifiers(token.kind),
                Some(token),
            );
        }
        if self.specifiers.type_specifiers == TypeSpecifiers::Empty {
            parser.report(
                context,
                ParserErrorType::NoTypeSpecifiersInDeclarationSpecifiers(token.kind),
                Some(token),
            );
        }
        self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
        ParseAction::Reduce(ParseValue::DeclarationSpecifiers(self.specifiers))
    }

    fn apply_type_specifier(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Token,
        specifier: PrimitiveTypeSpecifier,
    ) {
        let type_specifiers = &mut self.specifiers.type_specifiers;
        // Normalize order-independent keyword sequences into one canonical
        // TypeSpecifiers value while preserving specific conflict diagnostics.
        macro_rules! apply_once {
            ($is_duplicate:ident, $apply:ident) => {
                if type_specifiers.$is_duplicate() {
                    parser.report(
                        context,
                        ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        Some(token),
                    );
                } else {
                    type_specifiers.$apply(parser, context, token);
                }
            };
        }

        match specifier {
            | PrimitiveTypeSpecifier::Signed => apply_once!(is_signed, make_signed),
            | PrimitiveTypeSpecifier::Unsigned => apply_once!(is_unsigned, make_unsigned),
            | PrimitiveTypeSpecifier::Int => apply_once!(is_int, make_int),
            | PrimitiveTypeSpecifier::Short => apply_once!(is_short, make_short),
            | PrimitiveTypeSpecifier::Long if type_specifiers.is_long_double() => {
                parser.report(
                    context,
                    ParserErrorType::LongLongDoubleSpecified,
                    Some(token),
                );
            },
            | PrimitiveTypeSpecifier::Long
                if type_specifiers.is_long() && type_specifiers.is_long_long() =>
            {
                parser.report(context, ParserErrorType::LongSpecifiedThrice, Some(token));
            },
            | PrimitiveTypeSpecifier::Long => type_specifiers.make_long(parser, context, token),
            | PrimitiveTypeSpecifier::Char => apply_once!(is_char, make_char),
            | PrimitiveTypeSpecifier::Float => apply_once!(is_float, make_float),
            | PrimitiveTypeSpecifier::Double if type_specifiers.is_double() => {
                parser.report(
                    context,
                    ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                    Some(token),
                );
            },
            | PrimitiveTypeSpecifier::Double if type_specifiers.is_long_long() => {
                parser.report(
                    context,
                    ParserErrorType::LongLongDoubleSpecified,
                    Some(token),
                );
            },
            | PrimitiveTypeSpecifier::Double => type_specifiers.make_double(parser, context, token),
            | PrimitiveTypeSpecifier::Void => apply_once!(is_void, make_void),
            | PrimitiveTypeSpecifier::Bool => apply_once!(is_bool, make_bool),
            | PrimitiveTypeSpecifier::Complex => apply_once!(is_complex, make_complex),
            | PrimitiveTypeSpecifier::Imaginary => apply_once!(is_imaginary, make_imaginary),
        }
    }
}

/// Classifies one storage-class-specifier keyword.
///
/// C99: §6.7.1, p. 98; PDF p. 110.
fn storage_class(token: TokenType) -> Option<StorageClass> {
    match token {
        | TokenType::Keyword(KeywordTokenType::Auto) => Some(StorageClass::Auto),
        | TokenType::Keyword(KeywordTokenType::Register) => Some(StorageClass::Register),
        | TokenType::Keyword(KeywordTokenType::Static) => Some(StorageClass::Static),
        | TokenType::Keyword(KeywordTokenType::Extern) => Some(StorageClass::Extern),
        | TokenType::Keyword(KeywordTokenType::Typedef) => Some(StorageClass::Typedef),
        | _ => None,
    }
}

/// Classifies one type-qualifier keyword.
///
/// C99: §6.7.3, p. 108; PDF p. 120.
fn type_qualifier(token: TokenType) -> Option<TypeQualifiers> {
    match token {
        | TokenType::Keyword(KeywordTokenType::Const) => Some(TypeQualifiers::CONST),
        | TokenType::Keyword(KeywordTokenType::Volatile) => Some(TypeQualifiers::VOLATILE),
        | TokenType::Keyword(KeywordTokenType::Restrict) => Some(TypeQualifiers::RESTRICT),
        | _ => None,
    }
}

/// Primitive keyword recognized while accumulating a C type-specifier set.
///
/// C99: normative type-specifiers are §6.7.2, pp. 99-100; PDF pp. 111-112.
/// `_Imaginary` comes from the keyword inventory in §6.4.1, p. 50; PDF p. 62
/// and is retained here only for the existing extension path.
#[derive(Debug, Clone, Copy)]
enum PrimitiveTypeSpecifier {
    Signed,
    Unsigned,
    Int,
    Short,
    Long,
    Char,
    Float,
    Double,
    Void,
    Bool,
    Complex,
    Imaginary,
}

/// Classifies a keyword for [`PrimitiveTypeSpecifier`] accumulation.
///
/// C99: §6.7.2, pp. 99-100; PDF pp. 111-112; `_Imaginary` caveat as documented
/// on [`PrimitiveTypeSpecifier`].
fn primitive_type_specifier(keyword: KeywordTokenType) -> Option<PrimitiveTypeSpecifier> {
    match keyword {
        | KeywordTokenType::Signed => Some(PrimitiveTypeSpecifier::Signed),
        | KeywordTokenType::Unsigned => Some(PrimitiveTypeSpecifier::Unsigned),
        | KeywordTokenType::Int => Some(PrimitiveTypeSpecifier::Int),
        | KeywordTokenType::Short => Some(PrimitiveTypeSpecifier::Short),
        | KeywordTokenType::Long => Some(PrimitiveTypeSpecifier::Long),
        | KeywordTokenType::Char => Some(PrimitiveTypeSpecifier::Char),
        | KeywordTokenType::Float => Some(PrimitiveTypeSpecifier::Float),
        | KeywordTokenType::Double => Some(PrimitiveTypeSpecifier::Double),
        | KeywordTokenType::Void => Some(PrimitiveTypeSpecifier::Void),
        | KeywordTokenType::Bool => Some(PrimitiveTypeSpecifier::Bool),
        | KeywordTokenType::Complex => Some(PrimitiveTypeSpecifier::Complex),
        | KeywordTokenType::Imaginary => Some(PrimitiveTypeSpecifier::Imaginary),
        | _ => None,
    }
}

impl DeclaratorFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | DeclaratorPhase::PointerOrBase => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::Asterisk) {
                    let token = token.expect("asterisk token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.has_pointer_level = true;
                    self.current_qualifiers = TypeQualifiers::empty();
                    self.phase = DeclaratorPhase::PointerQualifiers;
                    ParseAction::Consume
                } else {
                    self.phase = DeclaratorPhase::Base;
                    ParseAction::Reprocess
                }
            },
            | DeclaratorPhase::PointerQualifiers => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if let Some(token) = token
                    && let Some(qualifier) = type_qualifier(token.kind)
                {
                    if self.current_qualifiers.contains(qualifier) {
                        let error_type = match qualifier {
                            | TypeQualifiers::CONST => ParserErrorType::ConstSpecifiedTwice,
                            | TypeQualifiers::VOLATILE => ParserErrorType::VolatileSpecifiedTwice,
                            | TypeQualifiers::RESTRICT => ParserErrorType::RestrictSpecifiedTwice,
                            | _ => unreachable!("one qualifier is handled at a time"),
                        };
                        parser.report(context, error_type, Some(token));
                    }
                    self.current_qualifiers.insert(qualifier);
                    parser.merge_source(context, &mut self.source_vectors, token);
                    return ParseAction::Consume;
                }
                // Each `*` owns the qualifiers immediately following it. A
                // second `*` closes the current pointer level and starts the
                // next without involving Rust recursion.
                if is_operator(token, OperatorTokenType::Asterisk) {
                    self.pointer_qualifiers.push(self.current_qualifiers);
                    self.current_qualifiers = TypeQualifiers::empty();
                    let token = token.expect("asterisk token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    ParseAction::Consume
                } else {
                    self.pointer_qualifiers.push(self.current_qualifiers);
                    self.current_qualifiers = TypeQualifiers::empty();
                    self.phase = DeclaratorPhase::Base;
                    ParseAction::Reprocess
                }
            },
            | DeclaratorPhase::Base => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if let Some(token) = token
                    && token.kind == TokenType::Identifier
                    && self.mode != DeclaratorMode::Abstract
                {
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.direct_declarators
                        .push(DirectDeclarator::Identifier(Identifier::new(
                            token.contents,
                        )));
                    self.has_direct_declarator = true;
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Consume;
                }
                // In a named declarator, `(` must group another declarator. In
                // an abstract context it can instead start a function suffix,
                // so defer that choice to the classification phase.
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    let token = token.expect("opening-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = if self.mode == DeclaratorMode::Named {
                        DeclaratorPhase::PushNested
                    } else {
                        DeclaratorPhase::ClassifyAbstractParenthesis
                    };
                    return ParseAction::Consume;
                }
                if self.mode != DeclaratorMode::Named
                    && is_operator(token, OperatorTokenType::OpeningSquareBracket)
                {
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Reprocess;
                }
                if self.mode == DeclaratorMode::Named {
                    parser.report(
                        context,
                        ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                }
                self.phase = DeclaratorPhase::Finish;
                ParseAction::Reprocess
            },
            | DeclaratorPhase::PushNested => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = DeclaratorPhase::AwaitNested;
                ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(self.mode)))
            },
            | DeclaratorPhase::ClassifyAbstractParenthesis => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // `()` is an abstract function declarator; a declaration
                // starter begins a prototype; everything else is parsed as a
                // parenthesized abstract declarator.
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    self.direct_declarators.push(DirectDeclarator::Function {
                        parameter_list: VectorSlice::empty(),
                        is_variadic:    false,
                    });
                    self.has_direct_declarator = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    self.phase = DeclaratorPhase::AwaitParameterList;
                    ParseAction::Push(ParseFrame::ParameterList(ParameterListFrame::new(false)))
                } else {
                    self.phase = DeclaratorPhase::AwaitNested;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(self.mode)))
                }
            },
            | DeclaratorPhase::AwaitNested => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("nested declarator returned an unexpected value: {returned:?}");
                };
                let Some(declarator) = declarator else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Reprocess;
                };
                let source_vectors = declarator.source_vectors;
                self.source_vectors =
                    Some(self.source_vectors.map_or(source_vectors, |existing| {
                        context.merge_vectors(existing, source_vectors)
                    }));
                self.phase = DeclaratorPhase::ExpectNestedClose(declarator);
                ParseAction::Reprocess
            },
            | DeclaratorPhase::ExpectNestedClose(declarator) => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.direct_declarators
                    .push(DirectDeclarator::Parenthesized(declarator));
                self.has_direct_declarator = true;
                self.phase = DeclaratorPhase::Suffix;
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    ParseAction::Reprocess
                }
            },
            | DeclaratorPhase::Suffix => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Direct-declarator suffixes repeat left-to-right. Re-enter
                // this phase after every array or function child to preserve
                // binding order in the arena slice.
                if is_operator(token, OperatorTokenType::OpeningSquareBracket) {
                    let token = token.expect("opening-square-bracket token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.array_qualifiers = TypeQualifiers::empty();
                    self.array_qualifiers_before_static = false;
                    self.array_is_static = false;
                    self.array_is_pointer = false;
                    self.phase = DeclaratorPhase::Array;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    let token = token.expect("opening-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::FunctionStart;
                    ParseAction::Consume
                } else {
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                }
            },
            | DeclaratorPhase::Array => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // One state recognizes all four C99 array suffix families.
                // Keep `static`, qualifiers, and `*` as independent facts so
                // later semantic checks retain their original placement.
                if let Some(token) = token
                    && let Some(qualifier) = type_qualifier(token.kind)
                {
                    if self.array_qualifiers.contains(qualifier) {
                        let error_type = match qualifier {
                            | TypeQualifiers::CONST => ParserErrorType::ConstSpecifiedTwice,
                            | TypeQualifiers::VOLATILE => ParserErrorType::VolatileSpecifiedTwice,
                            | TypeQualifiers::RESTRICT => ParserErrorType::RestrictSpecifiedTwice,
                            | _ => unreachable!("one qualifier is handled at a time"),
                        };
                        parser.report(context, error_type, Some(token));
                    }
                    if self.array_is_static && self.array_qualifiers_before_static {
                        parser.report(
                            context,
                            ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
                            Some(token),
                        );
                    }
                    if !self.array_is_static {
                        self.array_qualifiers_before_static = true;
                    }
                    self.array_qualifiers.insert(qualifier);
                    parser.merge_source(context, &mut self.source_vectors, token);
                    return ParseAction::Consume;
                }
                if token
                    .is_some_and(|token| token.kind == TokenType::Keyword(KeywordTokenType::Static))
                {
                    let token = token.expect("static token exists");
                    if self.array_is_static {
                        parser.report(context, ParserErrorType::StaticSpecifiedTwice, Some(token));
                    }
                    self.array_is_static = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    return ParseAction::Consume;
                }
                if is_operator(token, OperatorTokenType::Asterisk) {
                    let token = token.expect("asterisk token exists");
                    if self.mode != DeclaratorMode::Named
                        && !self.has_direct_declarator
                        && !self.array_qualifiers.is_empty()
                    {
                        parser.report(
                            context,
                            ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator,
                            Some(token),
                        );
                    }
                    if self.array_is_static {
                        parser.report(
                            context,
                            ParserErrorType::BothStaticAndPointerInArrayDirectDeclarator,
                            Some(token),
                        );
                    }
                    self.array_is_pointer = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::ArrayExpectClose;
                    return ParseAction::Consume;
                }
                if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                    let token = token.expect("closing-square-bracket token exists");
                    if self.array_is_static {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator,
                            Some(token),
                        );
                    }
                    self.push_array();
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Consume;
                }
                if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(None),
                        None,
                    );
                    self.push_array();
                    self.phase = DeclaratorPhase::Finish;
                    return ParseAction::Reprocess;
                }
                self.phase = DeclaratorPhase::AwaitArrayBound;
                ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::new(
                    FutureChildKind::ArrayBoundExpression,
                )))
            },
            | DeclaratorPhase::ArrayExpectClose => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // After the VLA `*` marker, only `]` belongs to this grammar
                // alternative. Diagnose extra input specifically, then recover
                // to the owning bracket or an enclosing declaration boundary.
                if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                    let token = token.expect("closing-square-bracket token exists");
                    self.push_array();
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Asterisk) {
                    let token = token.expect("asterisk token exists");
                    parser.report(context, ParserErrorType::PointerSpecifiedTwice, Some(token));
                    parser.merge_source(context, &mut self.source_vectors, token);
                    ParseAction::Consume
                } else if let Some(token) = token {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(
                            token.kind,
                        ),
                        Some(token),
                    );
                    self.phase = DeclaratorPhase::AwaitArrayBound;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::ArrayBound,
                        target: ParseFrameKind::Declarator,
                    })
                } else {
                    parser.report(
                        context,
                        ParserErrorType::UnexpectedEndOfArrayDeclaratorAfterPointer,
                        None,
                    );
                    self.phase = DeclaratorPhase::AwaitArrayBound;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::ArrayBound,
                        target: ParseFrameKind::Declarator,
                    })
                }
            },
            | DeclaratorPhase::AwaitArrayBound => {
                // This phase is entered both after a future expression child
                // and after direct recovery from malformed `[* ...]` input.
                // Merge a child only when one actually returned.
                if let Some(returned) = returned {
                    let ParseValue::FutureChild(FutureChildResult {
                        kind: FutureChildKind::ArrayBoundExpression,
                        source_vectors,
                    }) = returned
                    else {
                        panic!("array-bound seam returned an unexpected value: {returned:?}");
                    };
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }));
                }
                if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                    let token = token.expect("closing-square-bracket token exists");
                    self.push_array();
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Comma)
                    || is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                    || token.is_some_and(|token| parser.declaration_starter(token))
                {
                    // These tokens belong to an enclosing production. Repair
                    // the absent `]`, finish this declarator, and reprocess the
                    // boundary instead of consuming it here.
                    if !self.array_is_pointer {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                    }
                    self.push_array();
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    if !self.array_is_pointer {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(
                                None,
                            ),
                            None,
                        );
                    }
                    self.push_array();
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::ArrayBound,
                        target: ParseFrameKind::Declarator,
                    })
                }
            },
            | DeclaratorPhase::FunctionStart => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // An identifier-list is legal only when this suffix is bound
                // to a named declarator. Abstract function declarators always
                // interpret their contents as a prototype.
                let allow_k_and_r = self.has_named_direct_declarator(parser);
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let direct = if allow_k_and_r {
                        DirectDeclarator::KAndRStyleFunction {
                            parameters: VectorSlice::empty(),
                        }
                    } else {
                        DirectDeclarator::Function {
                            parameter_list: VectorSlice::empty(),
                            is_variadic:    false,
                        }
                    };
                    self.direct_declarators.push(direct);
                    self.has_direct_declarator = true;
                    let token = token.expect("closing-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::UnexpectedEndOfFunctionDeclaratorParameterList,
                        None,
                    );
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    self.phase = DeclaratorPhase::AwaitParameterList;
                    ParseAction::Push(ParseFrame::ParameterList(ParameterListFrame::new(
                        allow_k_and_r,
                    )))
                }
            },
            | DeclaratorPhase::AwaitParameterList => {
                let Some(ParseValue::ParameterList(ParameterListResult {
                    direct_declarator,
                    source_vectors,
                })) = returned
                else {
                    panic!("parameter list returned an unexpected value: {returned:?}");
                };
                self.source_vectors =
                    Some(self.source_vectors.map_or(source_vectors, |existing| {
                        context.merge_vectors(existing, source_vectors)
                    }));
                self.direct_declarators.push(direct_declarator);
                self.has_direct_declarator = true;
                self.phase = DeclaratorPhase::Suffix;
                ParseAction::Reprocess
            },
            | DeclaratorPhase::Finish => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // `None` is a typed, recoverable absence used by optional
                // abstract declarators and by callers that issue the contextual
                // missing-declarator diagnostic.
                if !self.has_pointer_level && !self.has_direct_declarator {
                    return ParseAction::Reduce(ParseValue::Declarator(None));
                }
                if self.mode == DeclaratorMode::Named && !self.has_direct_declarator {
                    parser.report(
                        context,
                        ParserErrorType::TypeQualifiersWithoutDeclarator,
                        token,
                    );
                    return ParseAction::Reduce(ParseValue::Declarator(None));
                }

                // Commit both flat component lists atomically before returning
                // the value that references their stable slices.
                let pointer_start = parser.syntax.type_qualifiers.len().to_u32();
                parser
                    .syntax
                    .type_qualifiers
                    .append(&mut self.pointer_qualifiers);
                let direct_start = parser.syntax.direct_declarators.len().to_u32();
                parser
                    .syntax
                    .direct_declarators
                    .append(&mut self.direct_declarators);
                let declarator = Declarator {
                    pointer:        PointerDeclarator {
                        type_qualifiers_list: VectorSlice::new(
                            pointer_start,
                            parser.syntax.type_qualifiers.len().to_u32(),
                        ),
                    },
                    kind:           VectorSlice::new(
                        direct_start,
                        parser.syntax.direct_declarators.len().to_u32(),
                    ),
                    source_vectors: self.source_vectors.unwrap_or_default(),
                };
                ParseAction::Reduce(ParseValue::Declarator(Some(declarator)))
            },
        }
    }

    fn push_array(&mut self) {
        self.direct_declarators.push(DirectDeclarator::Array {
            type_qualifiers:       self.array_qualifiers,
            is_static:             self.array_is_static,
            is_pointer:            self.array_is_pointer,
            assignment_expression: None,
        });
        self.has_direct_declarator = true;
        self.array_qualifiers = TypeQualifiers::empty();
        self.array_qualifiers_before_static = false;
        self.array_is_static = false;
        self.array_is_pointer = false;
    }

    fn has_named_direct_declarator(&self, parser: &Parser) -> bool {
        self.direct_declarators.iter().any(|direct| match direct {
            | DirectDeclarator::Identifier(_) => true,
            | DirectDeclarator::Parenthesized(declarator) =>
                parser.declarator_identifier(*declarator).is_some(),
            | _ => false,
        })
    }
}

impl ParameterListFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | ParameterListPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Prototype scope starts with the parameter list. A visible
                // typedef forces prototype syntax; only a non-typedef
                // identifier can select the legacy identifier-list branch.
                self.entry_scope_depth = Some(parser.scopes.depth());
                parser.scopes.enter_scope(ScopeKind::FunctionPrototype);
                if self.allow_k_and_r
                    && token.is_some_and(|token| {
                        token.kind == TokenType::Identifier
                            && !parser.scopes.is_typedef(token.contents)
                    })
                {
                    self.phase = ParameterListPhase::KAndRIdentifier;
                } else {
                    self.phase = ParameterListPhase::PrototypeParameter;
                }
                ParseAction::Reprocess
            },
            | ParameterListPhase::KAndRIdentifier => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                let Some(token) = token else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(
                            None,
                        ),
                        None,
                    );
                    self.phase = ParameterListPhase::FinishKAndR;
                    return ParseAction::Reprocess;
                };
                // Once identifier-list syntax is selected, a declaration
                // starter cannot silently switch dialects mid-list.
                if parser.declaration_starter(token) {
                    parser.report(
                        context,
                        ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator,
                        Some(token),
                    );
                    self.phase = ParameterListPhase::KAndRSeparator;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::KAndRParameter,
                        target: ParseFrameKind::ParameterList,
                    });
                }
                if token.kind != TokenType::Identifier || parser.scopes.is_typedef(token.contents) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                    self.phase = ParameterListPhase::KAndRSeparator;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::KAndRParameter,
                        target: ParseFrameKind::ParameterList,
                    });
                }
                self.identifiers.push(Identifier::new(token.contents));
                parser.merge_source(context, &mut self.source_vectors, token);
                self.phase = ParameterListPhase::KAndRSeparator;
                ParseAction::Consume
            },
            | ParameterListPhase::KAndRSeparator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::Comma) {
                    if let Some(token) = token {
                        parser.merge_source(context, &mut self.source_vectors, token);
                    }
                    self.phase = ParameterListPhase::KAndRIdentifier;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = ParameterListPhase::FinishKAndR;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = ParameterListPhase::FinishKAndR;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                            None,
                        ),
                        None,
                    );
                    self.phase = ParameterListPhase::FinishKAndR;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| {
                    token.kind == TokenType::Identifier && !parser.scopes.is_typedef(token.contents)
                }) {
                    // Repair an omitted comma without discarding the next
                    // parameter name: diagnose, then reprocess it as an item.
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = ParameterListPhase::KAndRIdentifier;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::KAndRParameter,
                        target: ParseFrameKind::ParameterList,
                    })
                }
            },
            | ParameterListPhase::PrototypeParameter => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = ParameterListPhase::AwaitSpecifiers;
                ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                    DeclarationSpecifiersFrame::new(SpecifierMode::Declaration),
                ))
            },
            | ParameterListPhase::AwaitSpecifiers => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    panic!("parameter specifiers returned an unexpected value: {returned:?}");
                };
                self.pending_specifiers = Some(specifiers);
                self.pending_source = Some(specifiers.source_vectors);
                // parameter-declaration permits an absent abstract declarator,
                // so a separator can complete the parameter immediately.
                if is_operator(token, OperatorTokenType::Comma)
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.parameters.push(ParameterDeclaration {
                        declaration_specifiers: specifiers,
                        declarator:             None,
                        source_vectors:         specifiers.source_vectors,
                    });
                    self.source_vectors = Some(
                        self.source_vectors
                            .map_or(specifiers.source_vectors, |existing| {
                                context.merge_vectors(existing, specifiers.source_vectors)
                            }),
                    );
                    self.pending_source = None;
                    self.phase = ParameterListPhase::PrototypeSeparator;
                    ParseAction::Reprocess
                } else {
                    self.phase = ParameterListPhase::AwaitDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        DeclaratorMode::MaybeAbstract,
                    )))
                }
            },
            | ParameterListPhase::AwaitDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("parameter declarator returned an unexpected value: {returned:?}");
                };
                let declarator_source = declarator.map(|declarator| declarator.source_vectors);
                // Parameter names enter prototype scope as soon as their
                // declarator completes and may hide typedefs in later entries.
                if let Some(identifier) =
                    declarator.and_then(|declarator| parser.declarator_identifier(declarator))
                {
                    parser.scopes.publish(identifier.name, NameClass::Ordinary);
                }
                let parameter_source = match (self.pending_source.take(), declarator_source) {
                    | (Some(specifiers), Some(declarator)) =>
                        context.merge_vectors(specifiers, declarator),
                    | (Some(specifiers), None) => specifiers,
                    | (None, Some(declarator)) => declarator,
                    | (None, None) => SourceVectors::default(),
                };
                self.parameters.push(ParameterDeclaration {
                    declaration_specifiers: self
                        .pending_specifiers
                        .take()
                        .expect("parameter declarator follows specifiers"),
                    declarator,
                    source_vectors: parameter_source,
                });
                self.source_vectors =
                    Some(self.source_vectors.map_or(parameter_source, |existing| {
                        context.merge_vectors(existing, parameter_source)
                    }));
                self.phase = ParameterListPhase::PrototypeSeparator;
                ParseAction::Reprocess
            },
            | ParameterListPhase::PrototypeSeparator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Comma) {
                    if let Some(token) = token {
                        parser.merge_source(context, &mut self.source_vectors, token);
                    }
                    self.phase = ParameterListPhase::AfterComma;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                            None,
                        ),
                        None,
                    );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    // Preserve a plausible next parameter after an omitted
                    // comma instead of consuming it during recovery.
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = ParameterListPhase::PrototypeParameter;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::Parameter,
                        target: ParseFrameKind::ParameterList,
                    })
                }
            },
            | ParameterListPhase::AfterComma => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // `, ...` terminates parameter-type-list. A closing delimiter
                // here means the comma had no following parameter, which gets
                // its own diagnostic rather than a specifier cascade.
                if is_operator(token, OperatorTokenType::Ellipsis) {
                    self.is_variadic = true;
                    if let Some(token) = token {
                        parser.merge_source(context, &mut self.source_vectors, token);
                    }
                    self.phase = ParameterListPhase::ExpectCloseAfterEllipsis;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    parser.report(
                        context,
                        ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                    || token.is_none()
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else {
                    self.phase = ParameterListPhase::PrototypeParameter;
                    ParseAction::Reprocess
                }
            },
            | ParameterListPhase::ExpectCloseAfterEllipsis => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Ellipsis is terminal in the grammar, so the owning `)` is
                // the only legal continuation.
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    let token = token.expect("unwind token exists");
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                            token.kind,
                        ),
                        Some(token),
                    );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if let Some(token) = token
                    && self.can_unwind_variadic_recovery
                    && parser.declaration_starter(token)
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                            token.kind,
                        ),
                        Some(token),
                    );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::UnexpectedEndOfVariadicFunctionDeclaratorParameterList,
                        None,
                    );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if let Some(token) = token {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                            token.kind,
                        ),
                        Some(token),
                    );
                    let follows_comma = is_operator(Some(token), OperatorTokenType::Comma);
                    self.can_unwind_variadic_recovery = !follows_comma;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   if follows_comma {
                            SynchronizationKind::VariadicTrailingParameter
                        } else {
                            SynchronizationKind::VariadicParameterList
                        },
                        target: ParseFrameKind::ParameterList,
                    })
                } else {
                    unreachable!("EOF is handled before malformed variadic tokens")
                }
            },
            | ParameterListPhase::FinishKAndR => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Both successful and recovered exits must close prototype
                // scope before the enclosing declarator resumes.
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("parameter list entered prototype scope"),
                );
                let start = parser.syntax.identifiers.len().to_u32();
                parser.syntax.identifiers.append(&mut self.identifiers);
                ParseAction::Reduce(ParseValue::ParameterList(ParameterListResult {
                    direct_declarator: DirectDeclarator::KAndRStyleFunction {
                        parameters: VectorSlice::new(
                            start,
                            parser.syntax.identifiers.len().to_u32(),
                        ),
                    },
                    source_vectors:    self.source_vectors.unwrap_or_default(),
                }))
            },
            | ParameterListPhase::FinishPrototype => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Mirror the K&R exit: never leak prototype bindings into the
                // enclosing file or parameter scope.
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("parameter list entered prototype scope"),
                );
                let start = parser.syntax.parameter_declarations.len().to_u32();
                parser
                    .syntax
                    .parameter_declarations
                    .append(&mut self.parameters);
                ParseAction::Reduce(ParseValue::ParameterList(ParameterListResult {
                    direct_declarator: DirectDeclarator::Function {
                        parameter_list: VectorSlice::new(
                            start,
                            parser.syntax.parameter_declarations.len().to_u32(),
                        ),
                        is_variadic:    self.is_variadic,
                    },
                    source_vectors:    self.source_vectors.unwrap_or_default(),
                }))
            },
        }
    }
}

impl StructOrUnionSpecifierFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | StructOrUnionPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                let Some(token) = token else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStructOrUnionKeyword(None),
                        None,
                    );
                    return self.finish(parser);
                };
                self.kind = match token.kind {
                    | TokenType::Keyword(KeywordTokenType::Struct) => Some(StructOrUnion::Struct),
                    | TokenType::Keyword(KeywordTokenType::Union) => Some(StructOrUnion::Union),
                    | _ => {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedStructOrUnionKeyword(Some(token.kind)),
                            Some(token),
                        );
                        None
                    },
                };
                parser.merge_source(context, &mut self.source_vectors, token);
                self.phase = StructOrUnionPhase::NameOrBody;
                ParseAction::Consume
            },
            | StructOrUnionPhase::NameOrBody => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // The grammar allows either a tagged reference, a tagged
                // definition, or an anonymous definition. Record the tag
                // first and decide whether a body follows in a separate phase.
                if let Some(token) = token
                    && token.kind == TokenType::Identifier
                {
                    self.identifier = Some(Identifier::new(token.contents));
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = StructOrUnionPhase::AfterName;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::StructOrUnionSpecifierWithoutNameAndBody(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.finish(parser)
                }
            },
            | StructOrUnionPhase::AfterName => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Consume
                } else {
                    self.finish(parser)
                }
            },
            | StructOrUnionPhase::MemberStart => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // This frame owns the body's `}`. Any other token begins a
                // specifier-qualifier-list child for the next member.
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
                    if self.declarations.is_empty() {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedStructDeclarationBeforeClosingCurlyBrace,
                            Some(token),
                        );
                    }
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(None),
                        None,
                    );
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    self.member_source = None;
                    self.current_member_declarator_source = None;
                    self.phase = StructOrUnionPhase::AwaitMemberSpecifiers;
                    ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                        DeclarationSpecifiersFrame::new(SpecifierMode::SpecifierQualifier),
                    ))
                }
            },
            | StructOrUnionPhase::AwaitMemberSpecifiers => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    panic!("member specifiers returned an unexpected value: {returned:?}");
                };
                self.member_specifiers = Some(specifiers);
                self.member_source = Some(specifiers.source_vectors);
                // A leading colon is the unnamed-bit-field alternative; it
                // deliberately bypasses the named declarator child.
                if is_operator(token, OperatorTokenType::Colon) {
                    let token = token.expect("colon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    parser.merge_source(context, &mut self.current_member_declarator_source, token);
                    self.member_declarator = None;
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    parser.report(context, ParserErrorType::EmptyStructDeclarator, token);
                    let token = token.expect("semicolon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    self.finish_member(parser, context);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Consume
                } else {
                    self.phase = StructOrUnionPhase::PushMemberDeclarator;
                    ParseAction::Reprocess
                }
            },
            | StructOrUnionPhase::PushMemberDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::Colon) {
                    let token = token.expect("colon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    parser.merge_source(context, &mut self.current_member_declarator_source, token);
                    self.member_declarator = None;
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else {
                    self.phase = StructOrUnionPhase::AwaitMemberDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        DeclaratorMode::Named,
                    )))
                }
            },
            | StructOrUnionPhase::AwaitMemberDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("member declarator returned an unexpected value: {returned:?}");
                };
                self.member_declarator = declarator;
                if let Some(source_vectors) = declarator.map(|declarator| declarator.source_vectors)
                {
                    self.member_source =
                        Some(self.member_source.map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }));
                    self.current_member_declarator_source = Some(source_vectors);
                }
                self.phase = StructOrUnionPhase::AfterMemberDeclarator;
                ParseAction::Reprocess
            },
            | StructOrUnionPhase::AfterMemberDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Colon upgrades the completed member into a named bit-field.
                // Otherwise commit it as an ordinary member without consuming
                // the separator that follows.
                if is_operator(token, OperatorTokenType::Colon) {
                    let token = token.expect("colon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    parser.merge_source(context, &mut self.current_member_declarator_source, token);
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else {
                    let source_vectors = self
                        .current_member_declarator_source
                        .take()
                        .unwrap_or_default();
                    self.member_declarators.push(StructDeclarator {
                        declarator: self.member_declarator.take(),
                        bitfield_width: None,
                        source_vectors,
                    });
                    self.phase = StructOrUnionPhase::AfterStructDeclarator;
                    ParseAction::Reprocess
                }
            },
            | StructOrUnionPhase::PushBitFieldWidth => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = StructOrUnionPhase::AwaitBitFieldWidth;
                ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::new(
                    FutureChildKind::BitFieldWidthExpression,
                )))
            },
            | StructOrUnionPhase::AwaitBitFieldWidth => {
                let Some(ParseValue::FutureChild(FutureChildResult {
                    kind: FutureChildKind::BitFieldWidthExpression,
                    source_vectors,
                })) = returned
                else {
                    panic!("bit-field seam returned an unexpected value: {returned:?}");
                };
                self.member_source = Some(self.member_source.map_or(source_vectors, |existing| {
                    context.merge_vectors(existing, source_vectors)
                }));
                self.current_member_declarator_source = Some(
                    self.current_member_declarator_source
                        .map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }),
                );
                let source_vectors = self
                    .current_member_declarator_source
                    .take()
                    .unwrap_or_default();
                self.member_declarators.push(StructDeclarator {
                    declarator: self.member_declarator.take(),
                    bitfield_width: None,
                    source_vectors,
                });
                self.phase = StructOrUnionPhase::AfterStructDeclarator;
                ParseAction::Reprocess
            },
            | StructOrUnionPhase::AfterStructDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // `,` stays inside one struct-declarator-list; `;` commits the
                // whole struct-declaration. Enclosing delimiters are repaired
                // and reprocessed rather than swallowed by this frame.
                if is_operator(token, OperatorTokenType::Comma) {
                    let token = token.expect("comma token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    self.phase = StructOrUnionPhase::PushMemberDeclarator;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    let token = token.expect("semicolon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    self.finish_member(parser, context);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList,
                        token,
                    );
                    self.finish_member(parser, context);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.finish_member(parser, context);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.finish_member(parser, context);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(None),
                        None,
                    );
                    self.finish_member(parser, context);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::StructMember,
                        target: ParseFrameKind::StructOrUnionSpecifier,
                    })
                }
            },
            | StructOrUnionPhase::FinishBody => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.finish(parser)
            },
        }
    }

    fn finish_member(&mut self, parser: &mut Parser, context: &mut Context) {
        // Commit all declarators for this shared specifier-qualifier-list as a
        // single member declaration with one stable arena slice.
        let start = parser.syntax.struct_declarators.len().to_u32();
        parser
            .syntax
            .struct_declarators
            .append(&mut self.member_declarators);
        let specifiers = self
            .member_specifiers
            .take()
            .expect("a struct member has specifiers");
        let source_vectors = self.member_source.take().unwrap_or_default();
        self.declarations.push(StructDeclaration {
            type_qualifiers: specifiers.type_qualifiers,
            type_specifiers: specifiers.type_specifiers,
            struct_declarator_list: VectorSlice::new(
                start,
                parser.syntax.struct_declarators.len().to_u32(),
            ),
            source_vectors,
        });
        self.source_vectors = Some(self.source_vectors.map_or(source_vectors, |existing| {
            context.merge_vectors(existing, source_vectors)
        }));
    }

    fn finish(&mut self, parser: &mut Parser) -> ParseAction {
        let declaration_list = self.body_started.then(|| {
            let start = parser.syntax.struct_declarations.len().to_u32();
            parser
                .syntax
                .struct_declarations
                .append(&mut self.declarations);
            VectorSlice::new(start, parser.syntax.struct_declarations.len().to_u32())
        });
        let index = parser.syntax.struct_or_union_specifiers.len().to_u32();
        parser
            .syntax
            .struct_or_union_specifiers
            .push(StructOrUnionSpecifier {
                struct_or_union:         self.kind.unwrap_or(StructOrUnion::Struct),
                identifier:              self.identifier,
                struct_declaration_list: declaration_list,
                source_vectors:          self.source_vectors.unwrap_or_default(),
            });
        ParseAction::Reduce(ParseValue::StructOrUnionSpecifier(
            StructOrUnionSpecifierIndex(index),
        ))
    }
}

impl EnumSpecifierFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | EnumPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                let Some(token) = token else {
                    parser.report(context, ParserErrorType::ExpectedEnumKeyword(None), None);
                    return self.finish(parser);
                };
                if token.kind != TokenType::Keyword(KeywordTokenType::Enum) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumKeyword(Some(token.kind)),
                        Some(token),
                    );
                }
                parser.merge_source(context, &mut self.source_vectors, token);
                self.phase = EnumPhase::NameOrBody;
                ParseAction::Consume
            },
            | EnumPhase::NameOrBody => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Like tag specifiers for aggregates, an enum can be a tagged
                // reference, tagged definition, or anonymous definition.
                if let Some(token) = token
                    && token.kind == TokenType::Identifier
                {
                    self.name = Some(Identifier::new(token.contents));
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = EnumPhase::AfterName;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::EnumSpecifierWithoutNameAndBody(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.finish(parser)
                }
            },
            | EnumPhase::AfterName => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Consume
                } else {
                    self.finish(parser)
                }
            },
            | EnumPhase::EnumeratorOrClose => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // This state is also reached after a comma, which is why `}`
                // accepts the standard's optional trailing-comma form.
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
                    if self.enumerators.is_empty() {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedEnumeratorBeforeClosingCurlyBrace,
                            Some(token),
                        );
                    }
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Consume
                } else if let Some(token) = token
                    && token.kind == TokenType::Identifier
                {
                    self.current_enumerator = Some(Identifier::new(token.contents));
                    self.current_enumerator_source = Some(token.source_vectors);
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = EnumPhase::AfterEnumeratorName;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            None,
                        ),
                        None,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::AfterEnumerator;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::EnumeratorValue,
                        target: ParseFrameKind::EnumSpecifier,
                    })
                }
            },
            | EnumPhase::AfterEnumeratorName => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // The constant-expression is optional. Finalize immediately
                // unless `=` explicitly transfers ownership to the value child.
                if is_operator(token, OperatorTokenType::Equals) {
                    let token = token.expect("equals token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    parser.merge_source(context, &mut self.current_enumerator_source, token);
                    self.phase = EnumPhase::PushEnumeratorValue;
                    ParseAction::Consume
                } else {
                    self.finish_enumerator(parser);
                    self.phase = EnumPhase::AfterEnumerator;
                    ParseAction::Reprocess
                }
            },
            | EnumPhase::PushEnumeratorValue => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = EnumPhase::AwaitEnumeratorValue;
                ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::new(
                    FutureChildKind::EnumeratorValueExpression,
                )))
            },
            | EnumPhase::AwaitEnumeratorValue => {
                let Some(ParseValue::FutureChild(FutureChildResult {
                    kind: FutureChildKind::EnumeratorValueExpression,
                    source_vectors,
                })) = returned
                else {
                    panic!("enumerator-value seam returned an unexpected value: {returned:?}");
                };
                self.source_vectors =
                    Some(self.source_vectors.map_or(source_vectors, |existing| {
                        context.merge_vectors(existing, source_vectors)
                    }));
                self.current_enumerator_source = Some(
                    self.current_enumerator_source
                        .map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }),
                );
                self.finish_enumerator(parser);
                self.phase = EnumPhase::AfterEnumerator;
                ParseAction::Reprocess
            },
            | EnumPhase::AfterEnumerator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // A following identifier is a strong omitted-comma recovery
                // point: preserve it and parse it as the next enumerator.
                if is_operator(token, OperatorTokenType::Comma) {
                    let token = token.expect("comma token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| token.kind == TokenType::Identifier) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.stopped_before_declaration = true;
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(None),
                        None,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::EnumeratorValue,
                        target: ParseFrameKind::EnumSpecifier,
                    })
                }
            },
            | EnumPhase::FinishBody => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.finish(parser)
            },
        }
    }

    fn finish_enumerator(&mut self, parser: &mut Parser) {
        if let Some(name) = self.current_enumerator.take() {
            self.enumerators.push(Enumerator {
                name,
                expression: None,
                source_vectors: self.current_enumerator_source.take().unwrap_or_default(),
            });
            // Enumeration constants join the ordinary identifier namespace as
            // soon as their enumerator completes, affecting later values.
            parser.scopes.publish(name.name, NameClass::Ordinary);
        }
    }

    fn finish(&mut self, parser: &mut Parser) -> ParseAction {
        let enumeration_list = self.body_started.then(|| {
            let start = parser.syntax.enumerators.len().to_u32();
            parser.syntax.enumerators.append(&mut self.enumerators);
            VectorSlice::new(start, parser.syntax.enumerators.len().to_u32())
        });
        let index = parser.syntax.enum_specifiers.len().to_u32();
        parser.syntax.enum_specifiers.push(EnumSpecifier {
            name: self.name,
            enumeration_list,
            source_vectors: self.source_vectors.unwrap_or_default(),
        });
        ParseAction::Reduce(ParseValue::EnumSpecifier(EnumSpecifierResult {
            index: EnumSpecifierIndex(index),
            stopped_before_declaration: self.stopped_before_declaration,
        }))
    }
}

/// One top-level parser result.
///
/// Valid and recovered declarations both retain an arena handle. Consumers may
/// continue semantic analysis on recovered syntax while treating it as
/// diagnostic-tainted. `Error` is reserved for a future case where recovery
/// cannot construct a meaningful declaration at all.
///
/// C99: external-declaration and translation-unit are §6.9, p. 140; PDF
/// p. 152. Retaining recovered syntax after diagnosis is permitted by
/// §5.1.1.3 paragraph 1 and footnote 8, p. 11; PDF p. 23.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ExternalDeclaration {
    /// Declaration parsed without a hard syntax diagnostic.
    Declaration(DeclarationIndex),
    /// Repaired declaration produced after at least one hard syntax diagnostic.
    RecoveredDeclaration(DeclarationIndex),
    /// Function definition parsed without a hard syntax diagnostic.
    FunctionDefinition(FunctionDefinitionIndex),
    /// Function definition containing locally recovered syntax.
    RecoveredFunctionDefinition(FunctionDefinitionIndex),
    /// Provenance-only placeholder when no meaningful AST can be recovered.
    #[expect(
        dead_code,
        reason = "Current declaration recovery always retains a meaningful syntax node."
    )]
    Error(SourceVectors),
}

/// Typed handle into the declaration arena.
///
/// C99: declaration syntax is §6.7, pp. 97-130; PDF pp. 109-142.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct DeclarationIndex(u32);

/// Typed handle into the function-definition arena.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct FunctionDefinitionIndex(u32);

/// Typed handle into the expression arena.
///
/// C99: expressions are §6.5-§6.5.17, pp. 67-94; PDF pp. 79-106.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct ExpressionIndex(u32);

/// Expression handle whose grammar guarantees constant-expression syntax.
///
/// C99: constant-expression is §6.6, pp. 95-96; PDF pp. 107-108.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct ConstantExpressionIndex(u32);

impl From<ConstantExpressionIndex> for ExpressionIndex {
    fn from(index: ConstantExpressionIndex) -> Self {
        Self(index.0)
    }
}

/// Typed handle into the statement arena.
///
/// C99: statements and blocks are §6.8-§6.8.6.4, pp. 131-139;
/// PDF pp. 143-151.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct StatementIndex(u32);

/// Typed handle into the struct/union-specifier arena.
///
/// C99: §6.7.2.1, pp. 101-104; PDF pp. 113-116.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct StructOrUnionSpecifierIndex(u32);

/// Typed handle into the enum-specifier arena.
///
/// C99: §6.7.2.2, pp. 105-107; PDF pp. 117-119.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct EnumSpecifierIndex(u32);

/// Complete function-definition syntax produced at file scope.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct FunctionDefinition {
    pub(crate) declaration_specifiers: DeclarationSpecifiers,
    pub(crate) declarator:             Declarator,
    pub(crate) old_style_declarations: VectorSlice<DeclarationIndex>,
    pub(crate) body:                   StatementIndex,
    pub(crate) source_vectors:         SourceVectors,
    pub(crate) recovered:              bool,
}

/// One source-ordered item in a compound statement.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum BlockItem {
    Declaration(DeclarationIndex),
    Statement(StatementIndex),
}

/// A statement expression is either parsed by Phase 04, explicitly deferred
/// by Phase 03, or missing because recovery repaired a required position.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ExpressionSlot {
    #[expect(dead_code, reason = "Phase 04 will construct parsed expression slots.")]
    Parsed(ExpressionIndex),
    FutureChild(SourceVectors),
    Missing(SourceVectors),
}

/// Statement syntax node constructed by the statement frame.
///
/// C99: §6.8-§6.8.6.4, pp. 131-139; PDF pp. 143-151.
#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Statement {
    /// Grammar form and child handles of this statement.
    pub(crate) kind:           StatementType,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

/// C statement grammar forms represented through arena handles.
///
/// C99: statement alternatives are §6.8, p. 131; PDF p. 143; their detailed
/// productions are §6.8.1-§6.8.6.4, pp. 131-139; PDF pp. 143-151.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum StatementType {
    Compound {
        items: VectorSlice<BlockItem>,
    },
    Expression(ExpressionSlot),
    If {
        condition_expression: ExpressionSlot,
        then_statement:       StatementIndex,
        else_statement:       Option<StatementIndex>,
    },
    Switch {
        condition_expression: ExpressionSlot,
        body_statement:       StatementIndex,
    },
    While {
        condition_expression: ExpressionSlot,
        body_statement:       StatementIndex,
    },
    DoWhile {
        condition_expression: ExpressionSlot,
        body_statement:       StatementIndex,
    },
    For {
        initializer:          Option<ForInitializer>,
        condition_expression: Option<ExpressionSlot>,
        iteration_expression: Option<ExpressionSlot>,
        body_statement:       StatementIndex,
    },
    Return(Option<ExpressionSlot>),
    Break,
    Continue,
    Goto(Identifier),
    Label(Identifier, StatementIndex),
    Case(ExpressionSlot, StatementIndex),
    Default(StatementIndex),
    Null,
}

/// First clause of a `for` statement, which is syntactically either an
/// expression or a declaration.
///
/// C99: iteration-statement is §6.8.5, p. 135; PDF p. 147.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ForInitializer {
    Expression(ExpressionSlot),
    Declaration(DeclarationIndex),
}

/// Expression syntax node with exact source provenance.
///
/// C99: §6.5-§6.5.17, pp. 67-94; PDF pp. 79-106.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Expression {
    /// Operator/operand grammar form.
    pub(crate) kind:           ExpressionType,
    /// Original-source segments contributing to the expression.
    pub(crate) source_vectors: SourceVectors,
}

/// C expression grammar forms represented through arena handles.
///
/// C99: primary through comma expressions are §6.5.1-§6.5.17,
/// pp. 69-94; PDF pp. 81-106.
#[derive(Debug, PartialEq, Clone)]
#[expect(
    dead_code,
    reason = "The future expression frame will construct these syntax variants."
)]
pub(crate) enum ExpressionType {
    Conditional {
        condition_expression: ExpressionIndex,
        then_expression:      ExpressionIndex,
        else_expression:      ExpressionIndex,
    },
    Binary {
        operator:         BinaryOperator,
        left_expression:  ExpressionIndex,
        right_expression: ExpressionIndex,
    },
    Unary {
        operator:           UnaryOperator,
        operand_expression: ExpressionIndex,
    },
    Call {
        function_expression: ExpressionIndex,
        arguments:           VectorSlice<ExpressionIndex>,
    },
    DirectMember {
        base_expression: ExpressionIndex,
        member:          Identifier,
    },
    IndirectMember {
        base_expression: ExpressionIndex,
        member:          Identifier,
    },
    CompoundLiteral {
        struct_type:      TypeName,
        initializer_list: VectorSlice<Initializer>,
    },
    Identifier(Identifier),
    Constant(Constant),
    StringLiteral(StringCacheId),
    SizeofType(TypeName),
    SizeofExpr(ExpressionIndex),
    Cast {
        target_type:        TypeName,
        operand_expression: ExpressionIndex,
    },
}

/// Typed literal value accepted by a primary expression.
///
/// C99: primary-expression is §6.5.1, p. 69; PDF p. 81; constants are §6.4.4,
/// pp. 54-62; PDF pp. 66-74.
#[derive(Debug, PartialEq, Clone)]
#[expect(
    dead_code,
    reason = "The future expression frame will construct constants."
)]
pub(crate) enum Constant {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Char(CharacterTokenType),
}

/// Binary and postfix operators represented by expression nodes.
///
/// C99: postfix and binary expression productions are §6.5.2 and
/// §6.5.5-§6.5.17, pp. 69-94; PDF pp. 81-106.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
#[expect(
    dead_code,
    reason = "The future expression frame will construct binary operators."
)]
pub(crate) enum BinaryOperator {
    Multiplication,
    Division,
    Modulo,
    Addition,
    Subtraction,
    LeftShift,
    RightShift,
    LessThan,
    GreaterThan,
    LessThanOrEqual,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LogicalAnd,
    LogicalOr,
    Comma,
    Subscript,
    Assignment,
    MultiplicationAssignment,
    DivisionAssignment,
    ModuloAssignment,
    AdditionAssignment,
    SubtractionAssignment,
    LeftShiftAssignment,
    RightShiftAssignment,
    BitwiseAndAssignment,
    BitwiseXorAssignment,
    BitwiseOrAssignment,
}

/// Prefix, postfix, and cast-like unary operators.
///
/// C99: postfix, unary, and cast expressions are §6.5.2-§6.5.4,
/// pp. 69-81; PDF pp. 81-93.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
#[expect(
    dead_code,
    reason = "The future expression frame will construct unary operators."
)]
pub(crate) enum UnaryOperator {
    AddressOf,
    Indirection,
    Plus,
    Minus,
    BitwiseNot,
    LogicalNot,
    PreIncrement,
    PreDecrement,
    PostIncrement,
    PostDecrement,
    Cast,
}

/// Interned identifier spelling used by syntax nodes and scope classification.
///
/// C99: identifiers are §6.4.2.1, p. 51; PDF p. 63; their scopes and
/// namespaces are §6.2.1-§6.2.3, pp. 29-31; PDF pp. 41-43.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Identifier {
    /// Handle into [`Context`]'s shared string cache.
    pub(crate) name: StringCacheId,
}

impl Identifier {
    pub(crate) fn new(name: StringCacheId) -> Self {
        Self { name }
    }
}

/// storage-class-specifier:
/// typedef
/// extern
/// static
/// auto
/// register
///
/// C99: §6.7.1, p. 98; PDF p. 110.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Default)]
pub(crate) enum StorageClass {
    #[default]
    Auto,
    Register,
    Static,
    Extern,
    Typedef,
}

/// Structured parser diagnostic paired with original-source provenance.
///
/// C99: the diagnostic requirement is §5.1.1.3, p. 11; PDF p. 23.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ParserError {
    /// Dedicated diagnostic kind and its grammar-specific payload.
    pub(crate) error_type:     ParserErrorType,
    /// Source segments to underline when the diagnostic is rendered.
    pub(crate) source_vectors: SourceVectors,
}

impl Display for ParserError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl GetSeverity for ParserError {
    fn severity(&self) -> ErrorSeverity {
        self.error_type.severity()
    }
}

impl GetPosition for ParserError {
    fn position(&self, context: &Context) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for ParserError {
    fn source_vectors(&self, _context: &mut Context) -> SourceVectors {
        self.source_vectors
    }
}

impl std::error::Error for ParserError {}

/// Every diagnostic produced by the language parser.
///
/// Variants accepting `Option<TokenType>` use `Some` for an unexpected token
/// and `None` for EOF, keeping token and EOF messages specific without
/// stringly-typed context. Severity and display text are exhaustively defined
/// below, so adding a new parser failure requires an explicit policy.
///
/// C99: the obligation to diagnose syntax and constraint violations is
/// §5.1.1.3, p. 11; PDF p. 23. Each variant below also cites the production,
/// constraint, or semantic rule it concerns.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ParserErrorType {
    /// The preprocessed token stream contained no external declaration.
    /// C99: §6.9, p. 140; PDF p. 152.
    EmptyTranslationUnit,
    /// Internal invariant failure: a typed frame attempted to consume EOF.
    /// C99: implementation guard supporting §5.1.1.3, p. 11; PDF p. 23.
    ParserFrameConsumedAtEndOfInput(ParseFrameKind),
    /// Array-bound expression reached the deferred expression seam.
    /// C99: §6.7.5.2, pp. 116-117; PDF pp. 128-129.
    ArrayBoundExpressionNotImplemented,
    /// Bit-field width reached the deferred constant-expression seam.
    /// C99: §6.7.2.1, p. 101; PDF p. 113.
    BitFieldWidthExpressionNotImplemented,
    /// Enumerator value reached the deferred constant-expression seam.
    /// C99: §6.7.2.2, p. 105; PDF p. 117.
    EnumeratorValueExpressionNotImplemented,
    /// Initializer reached the deferred initializer seam.
    /// C99: §6.7.8, pp. 125-130; PDF pp. 137-142.
    InitializerNotImplemented,
    /// A present statement expression reached the Phase 04 seam.
    /// C99: §6.8-§6.8.6, pp. 131-139; PDF pp. 143-151.
    StatementExpressionNotImplemented,
    /// A function-definition head was not followed by its compound body.
    ExpectedFunctionBody(Option<TokenType>),
    /// A compound statement did not begin with `{`.
    ExpectedOpeningCurlyBraceInCompoundStatement(Option<TokenType>),
    /// A compound statement did not end with `}`.
    ExpectedClosingCurlyBraceInCompoundStatement(Option<TokenType>),
    /// No valid statement production began at the current token.
    ExpectedStatement(Option<TokenType>),
    /// `goto` was not followed by an identifier.
    ExpectedGotoLabel(Option<TokenType>),
    /// A required statement expression was absent.
    ExpectedStatementExpression(&'static str, Option<TokenType>),
    /// A statement header omitted its opening parenthesis.
    ExpectedOpeningParenthesisInStatement(&'static str, Option<TokenType>),
    /// A statement header omitted its closing parenthesis.
    ExpectedClosingParenthesisInStatement(&'static str, Option<TokenType>),
    /// A statement omitted its owned semicolon.
    ExpectedSemicolonInStatement(&'static str, Option<TokenType>),
    /// A label omitted its owned colon.
    ExpectedColonInLabel(&'static str, Option<TokenType>),
    /// A switch body contained more than one `default` label.
    DuplicateDefaultLabel,
    /// A `do` body was not followed by `while`.
    ExpectedWhileAfterDoBody(Option<TokenType>),
    /// A typedef declaration ended before naming its typedef.
    /// C99: §6.7, p. 97; PDF p. 109; §6.7.7, pp. 123-124;
    /// PDF pp. 135-136.
    ExpectedDeclaratorInTypedef(Option<TokenType>),
    /// A declaration required a named declarator but none could be parsed.
    /// C99: §6.7, p. 97; PDF p. 109.
    ExpectedDeclaratorInDeclaration(Option<TokenType>),
    /// Token after a declarator was not a legal declaration continuation.
    /// C99: §6.7, p. 97; PDF p. 109; function-definition continuation is
    /// §6.9.1, p. 141; PDF p. 153.
    ExpectedDeclarationContinuationAfterDeclarator(Option<TokenType>),
    /// EOF occurred before the first declaration specifier.
    /// C99: §6.7, p. 97; PDF p. 109.
    UnexpectedEndBeforeDeclarationSpecifier,
    /// EOF occurred after other specifiers but before a type specifier.
    /// C99: §6.7.2 paragraph 2, p. 99; PDF p. 111.
    UnexpectedEndBeforeTypeSpecifier,
    /// Named direct declarator did not begin with an identifier or `(`.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(Option<TokenType>),
    /// Parenthesized direct declarator had no nested declarator.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(Option<TokenType>),
    /// Parenthesized declarator was not closed by `)`.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    ExpectedClosingParenthesisAfterParenthesizedDeclarator(Option<TokenType>),
    /// Array declarator was not closed by `]`.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    ExpectedClosingSquareBracketInArrayDirectDeclarator(Option<TokenType>),
    /// EOF occurred inside a function declarator parameter list.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    UnexpectedEndOfFunctionDeclaratorParameterList,
    /// K&R identifier list contained no legal non-typedef identifier.
    /// C99: identifier-list is §6.7.5, p. 114; PDF p. 126; typedef preference
    /// is §6.7.5.3 paragraph 11, p. 119; PDF p. 131.
    ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(Option<TokenType>),
    /// K&R identifier was not followed by `,` or `)`.
    /// C99: identifier-list is §6.7.5, p. 114; PDF p. 126.
    ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(Option<TokenType>),
    /// Prototype parameter was not followed by `,` or `)`.
    /// C99: parameter-list is §6.7.5, p. 114; PDF p. 126.
    ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(Option<TokenType>),
    /// A prototype comma was not followed by a parameter or `...`.
    /// C99: parameter-type-list and parameter-list are §6.7.5,
    /// p. 114; PDF p. 126.
    ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(Option<TokenType>),
    /// Struct/union child was entered without its owning keyword.
    /// C99: §6.7.2.1, p. 101; PDF p. 113.
    ExpectedStructOrUnionKeyword(Option<TokenType>),
    /// Struct/union specifier had neither a tag nor a body.
    /// C99: §6.7.2.1, p. 101; PDF p. 113.
    StructOrUnionSpecifierWithoutNameAndBody(Option<TokenType>),
    /// Struct member list was not closed by `}`.
    /// C99: §6.7.2.1, p. 101; PDF p. 113.
    ExpectedClosingCurlyBraceInStructDeclarationList(Option<TokenType>),
    /// A struct or union definition had no member declaration.
    /// C99: struct-declaration-list is nonempty in §6.7.2.1, p. 101;
    /// PDF p. 113.
    ExpectedStructDeclarationBeforeClosingCurlyBrace,
    /// Struct member declaration reached `}` without its semicolon.
    /// C99: struct-declaration is §6.7.2.1, p. 101; PDF p. 113.
    ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList,
    /// Struct declarator was not followed by `,` or `;`.
    /// C99: struct-declarator-list and struct-declaration are §6.7.2.1,
    /// p. 101; PDF p. 113.
    ExpectedCommaOrSemicolonInStructDeclaratorList(Option<TokenType>),
    /// Enum child was entered without its owning keyword.
    /// C99: §6.7.2.2, p. 105; PDF p. 117.
    ExpectedEnumKeyword(Option<TokenType>),
    /// Enum specifier had neither a tag nor a body.
    /// C99: §6.7.2.2, p. 105; PDF p. 117.
    EnumSpecifierWithoutNameAndBody(Option<TokenType>),
    /// Enumerator list expected a name or its closing brace.
    /// C99: §6.7.2.2, p. 105; PDF p. 117.
    ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(Option<TokenType>),
    /// An enum definition had no enumerator.
    /// C99: enumerator-list is nonempty in §6.7.2.2, p. 105; PDF p. 117.
    ExpectedEnumeratorBeforeClosingCurlyBrace,
    /// Enumerator was not followed by `,` or `}`.
    /// C99: §6.7.2.2, p. 105; PDF p. 117.
    ExpectedCommaOrClosingCurlyInEnumeratorList(Option<TokenType>),
    /// Declaration specified more than one storage class.
    /// C99: §6.7.1 paragraph 2, p. 98; PDF p. 110.
    StorageClassRedefinition(StorageClass, TokenType),
    /// `const` occurred more than once in one qualifier sequence.
    /// C99: §6.7.3 paragraph 4, p. 108; PDF p. 120 says repetition has the
    /// same behavior as one occurrence; this diagnostic is therefore a warning.
    ConstSpecifiedTwice,
    /// `volatile` occurred more than once in one qualifier sequence.
    /// C99: §6.7.3 paragraph 4, p. 108; PDF p. 120 says repetition has the
    /// same behavior as one occurrence; this diagnostic is therefore a warning.
    VolatileSpecifiedTwice,
    /// `restrict` occurred more than once in one qualifier sequence.
    /// C99: §6.7.3 paragraph 4, p. 108; PDF p. 120 says repetition has the
    /// same behavior as one occurrence; this diagnostic is therefore a warning.
    RestrictSpecifiedTwice,
    /// `inline` occurred more than once in declaration specifiers.
    /// C99: inline is specified by §6.7.4, p. 113; PDF p. 125. The warning is
    /// an implementation quality diagnostic, not a required C99 diagnostic.
    InlineSpecifiedTwice,
    /// `static` occurred more than once in an array declarator.
    /// C99: §6.7.5 and §6.7.5.2, pp. 114 and 116-117;
    /// PDF pp. 126 and 128-129.
    StaticSpecifiedTwice,
    /// Array qualifiers appeared on both sides of `static`.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
    /// A new type specifier conflicts with the already accumulated type.
    /// C99: the permitted specifier sets are §6.7.2 paragraph 2,
    /// pp. 99-100; PDF pp. 111-112.
    ConflictingTypeSpecifiers(TypeSpecifiers, TokenType),
    /// A type keyword was repeated where no repetition is legal.
    /// C99: the permitted specifier sets are §6.7.2 paragraph 2,
    /// pp. 99-100; PDF pp. 111-112.
    TypeSpecifierSpecifiedTwice(TokenType),
    /// More than two `long` keywords occurred in one type.
    /// C99: the permitted specifier sets are §6.7.2 paragraph 2,
    /// pp. 99-100; PDF pp. 111-112.
    LongSpecifiedThrice,
    /// `double` was combined with `long long`.
    /// C99: the permitted specifier sets are §6.7.2 paragraph 2,
    /// pp. 99-100; PDF pp. 111-112.
    LongLongDoubleSpecified,
    /// Declaration-specifier parsing started on a non-specifier token.
    /// C99: declaration-specifiers are §6.7, p. 97; PDF p. 109.
    EmptyDeclarationSpecifiers(TokenType),
    /// A specifier sequence ended without a C99 type specifier.
    /// C99: §6.7.2 paragraph 2, p. 99; PDF p. 111.
    NoTypeSpecifiersInDeclarationSpecifiers(TokenType),
    /// An array declarator combined `static` and the `*` form.
    /// C99: the array alternatives are distinct productions in §6.7.5,
    /// p. 114; PDF p. 126.
    BothStaticAndPointerInArrayDirectDeclarator,
    /// `static` array syntax omitted its required assignment expression.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator,
    /// A token other than `]` followed `*` in an array declarator.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(TokenType),
    /// EOF followed `*` in an array declarator.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    UnexpectedEndOfArrayDeclaratorAfterPointer,
    /// The `*` variable-length-array marker occurred more than once.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    PointerSpecifiedTwice,
    /// Pointer qualifiers were present without a direct declarator.
    /// C99: named declarator requires direct-declarator under §6.7.5,
    /// p. 114; PDF p. 126.
    TypeQualifiersWithoutDeclarator,
    /// Abstract array qualifiers illegally preceded the `*` marker.
    /// C99: direct-abstract-declarator permits `[*]`, not a qualified `*`,
    /// under §6.7.6, p. 122; PDF p. 134.
    TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator,
    /// One function suffix mixed K&R names with prototype declarations.
    /// C99: function suffix alternatives are distinct productions in §6.7.5,
    /// p. 114; PDF p. 126.
    KAndRFunctionDeclaratorMixedWithModernDeclarator,
    /// A token other than `)` followed `...`.
    /// C99: parameter-type-list ends in `, ...` under §6.7.5,
    /// p. 114; PDF p. 126.
    ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(TokenType),
    /// EOF followed `...` before its closing parenthesis.
    /// C99: parameter-type-list ends in `, ...` under §6.7.5,
    /// p. 114; PDF p. 126.
    UnexpectedEndOfVariadicFunctionDeclaratorParameterList,
    /// Struct member declaration contained no declarator or bit-field.
    /// C99: struct-declarator-list is nonempty under §6.7.2.1,
    /// p. 101; PDF p. 113.
    EmptyStructDeclarator,
}

impl GetSeverity for ParserErrorType {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::EmptyTranslationUnit
            | Self::ParserFrameConsumedAtEndOfInput(..)
            | Self::ArrayBoundExpressionNotImplemented
            | Self::BitFieldWidthExpressionNotImplemented
            | Self::EnumeratorValueExpressionNotImplemented
            | Self::InitializerNotImplemented
            | Self::StatementExpressionNotImplemented
            | Self::ExpectedFunctionBody(..)
            | Self::ExpectedOpeningCurlyBraceInCompoundStatement(..)
            | Self::ExpectedClosingCurlyBraceInCompoundStatement(..)
            | Self::ExpectedStatement(..)
            | Self::ExpectedGotoLabel(..)
            | Self::ExpectedStatementExpression(..)
            | Self::ExpectedOpeningParenthesisInStatement(..)
            | Self::ExpectedClosingParenthesisInStatement(..)
            | Self::ExpectedSemicolonInStatement(..)
            | Self::ExpectedColonInLabel(..)
            | Self::DuplicateDefaultLabel
            | Self::ExpectedWhileAfterDoBody(..)
            | Self::ExpectedDeclaratorInTypedef(..)
            | Self::ExpectedDeclaratorInDeclaration(..)
            | Self::ExpectedDeclarationContinuationAfterDeclarator(..)
            | Self::UnexpectedEndBeforeDeclarationSpecifier
            | Self::UnexpectedEndBeforeTypeSpecifier
            | Self::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(..)
            | Self::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(..)
            | Self::ExpectedClosingParenthesisAfterParenthesizedDeclarator(..)
            | Self::ExpectedClosingSquareBracketInArrayDirectDeclarator(..)
            | Self::UnexpectedEndOfFunctionDeclaratorParameterList
            | Self::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(..)
            | Self::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(..)
            | Self::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(..)
            | Self::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(..)
            | Self::ExpectedStructOrUnionKeyword(..)
            | Self::StructOrUnionSpecifierWithoutNameAndBody(..)
            | Self::ExpectedClosingCurlyBraceInStructDeclarationList(..)
            | Self::ExpectedStructDeclarationBeforeClosingCurlyBrace
            | Self::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList
            | Self::ExpectedCommaOrSemicolonInStructDeclaratorList(..)
            | Self::ExpectedEnumKeyword(..)
            | Self::EnumSpecifierWithoutNameAndBody(..)
            | Self::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(..)
            | Self::ExpectedEnumeratorBeforeClosingCurlyBrace
            | Self::ExpectedCommaOrClosingCurlyInEnumeratorList(..)
            | Self::EmptyDeclarationSpecifiers(..)
            | Self::NoTypeSpecifiersInDeclarationSpecifiers(..)
            | Self::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator
            | Self::BothStaticAndPointerInArrayDirectDeclarator
            | Self::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator
            | Self::UnexpectedEndOfArrayDeclaratorAfterPointer
            | Self::TypeQualifiersWithoutDeclarator
            | Self::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                ..,
            )
            | Self::UnexpectedEndOfVariadicFunctionDeclaratorParameterList
            | Self::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator
            | Self::KAndRFunctionDeclaratorMixedWithModernDeclarator
            | Self::EmptyStructDeclarator
            | Self::PointerSpecifiedTwice
            | Self::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(..) =>
                ErrorSeverity::Error,
            | Self::StorageClassRedefinition(..)
            | Self::ConstSpecifiedTwice
            | Self::VolatileSpecifiedTwice
            | Self::RestrictSpecifiedTwice
            | Self::InlineSpecifiedTwice
            | Self::StaticSpecifiedTwice
            | Self::ConflictingTypeSpecifiers(..)
            | Self::TypeSpecifierSpecifiedTwice(..)
            | Self::LongSpecifiedThrice
            | Self::LongLongDoubleSpecified => ErrorSeverity::Warning,
        }
    }
}

impl Display for ParserErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | Self::EmptyTranslationUnit => write!(
                f,
                "Translation unit is empty; expected an external declaration."
            ),
            | Self::ParserFrameConsumedAtEndOfInput(frame) => write!(
                f,
                "Parser frame `{}` attempted to consume a token at end of input.",
                frame.label()
            ),
            | Self::ArrayBoundExpressionNotImplemented => write!(
                f,
                "The array-bound expression parser is not implemented yet; skipped the bound."
            ),
            | Self::BitFieldWidthExpressionNotImplemented => write!(
                f,
                "The bit-field-width expression parser is not implemented yet; skipped the width."
            ),
            | Self::EnumeratorValueExpressionNotImplemented => write!(
                f,
                "The enumerator-value expression parser is not implemented yet; skipped the value."
            ),
            | Self::InitializerNotImplemented => write!(
                f,
                "The initializer parser is not implemented yet; skipped the initializer."
            ),
            | Self::StatementExpressionNotImplemented => write!(
                f,
                "The expression parser is not implemented yet; deferred the statement expression."
            ),
            | Self::ExpectedFunctionBody(found) => write_expected(
                f,
                "function definition",
                "a compound-statement body",
                *found,
            ),
            | Self::ExpectedOpeningCurlyBraceInCompoundStatement(found) =>
                write_expected(f, "compound statement", "`{`", *found),
            | Self::ExpectedClosingCurlyBraceInCompoundStatement(found) =>
                write_expected(f, "compound statement", "`}`", *found),
            | Self::ExpectedStatement(found) =>
                write_expected(f, "statement", "a statement", *found),
            | Self::ExpectedGotoLabel(found) =>
                write_expected(f, "goto statement", "an identifier", *found),
            | Self::ExpectedStatementExpression(position, found) =>
                write_expected(f, position, "an expression", *found),
            | Self::ExpectedOpeningParenthesisInStatement(position, found) =>
                write_expected(f, position, "`(`", *found),
            | Self::ExpectedClosingParenthesisInStatement(position, found) =>
                write_expected(f, position, "`)`", *found),
            | Self::ExpectedSemicolonInStatement(position, found) =>
                write_expected(f, position, "`;`", *found),
            | Self::ExpectedColonInLabel(position, found) =>
                write_expected(f, position, "`:`", *found),
            | Self::DuplicateDefaultLabel => write!(
                f,
                "A switch statement cannot contain more than one `default` label."
            ),
            | Self::ExpectedWhileAfterDoBody(found) =>
                write_expected(f, "do statement", "`while` after the body", *found),
            | Self::ExpectedDeclaratorInTypedef(found) => write_expected(
                f,
                "typedef declaration",
                "a declarator naming the typedef",
                *found,
            ),
            | Self::ExpectedDeclaratorInDeclaration(found) =>
                write_expected(f, "declaration", "a declarator", *found),
            | Self::ExpectedDeclarationContinuationAfterDeclarator(found) => write_expected(
                f,
                "declaration after a declarator",
                "`,`, `=`, `;`, or a function body",
                *found,
            ),
            | Self::UnexpectedEndBeforeDeclarationSpecifier => write!(
                f,
                "Unexpected end of input before any declaration specifier was parsed."
            ),
            | Self::UnexpectedEndBeforeTypeSpecifier => write!(
                f,
                "Unexpected end of input before any type specifier was parsed."
            ),
            | Self::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(found) =>
                write_expected(f, "direct declarator", "an identifier or `(`", *found),
            | Self::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(found) =>
                write_expected(
                    f,
                    "parenthesized direct declarator",
                    "a declarator after `(`",
                    *found,
                ),
            | Self::ExpectedClosingParenthesisAfterParenthesizedDeclarator(found) =>
                write_expected(
                    f,
                    "parenthesized declarator",
                    "`)` after the nested declarator",
                    *found,
                ),
            | Self::ExpectedClosingSquareBracketInArrayDirectDeclarator(found) => write_expected(
                f,
                "array direct declarator",
                "`]` after the array-bound expression",
                *found,
            ),
            | Self::UnexpectedEndOfFunctionDeclaratorParameterList => write!(
                f,
                "Unexpected end of input in a function declarator parameter list; expected `)`."
            ),
            | Self::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(found) =>
                write_expected(
                    f,
                    "K&R function declarator parameter list",
                    "a non-typedef identifier",
                    *found,
                ),
            | Self::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                found,
            ) => write_expected(
                f,
                "K&R function declarator parameter list",
                "`,` or `)` after a parameter name",
                *found,
            ),
            | Self::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(found) =>
                write_expected(
                    f,
                    "function declarator parameter list",
                    "`,` or `)` after a parameter declaration",
                    *found,
                ),
            | Self::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(found) =>
                write_expected(
                    f,
                    "function declarator parameter list after `,`",
                    "a parameter declaration or `...`",
                    *found,
                ),
            | Self::ExpectedStructOrUnionKeyword(found) => write_expected(
                f,
                "struct-or-union specifier",
                "the `struct` or `union` keyword",
                *found,
            ),
            | Self::StructOrUnionSpecifierWithoutNameAndBody(found) =>
                write_expected(f, "struct-or-union specifier", "a tag name or `{`", *found),
            | Self::ExpectedClosingCurlyBraceInStructDeclarationList(found) =>
                write_expected(f, "struct declaration list", "`}`", *found),
            | Self::ExpectedStructDeclarationBeforeClosingCurlyBrace => write!(
                f,
                "Expected a struct declaration before `}}` in a struct or union body."
            ),
            | Self::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList => {
                write!(f, "Expected `;` before `}}` in a struct declarator list.")
            },
            | Self::ExpectedCommaOrSemicolonInStructDeclaratorList(found) =>
                write_expected(f, "struct declarator list", "`,` or `;`", *found),
            | Self::ExpectedEnumKeyword(found) =>
                write_expected(f, "enum specifier", "the `enum` keyword", *found),
            | Self::EnumSpecifierWithoutNameAndBody(found) =>
                write_expected(f, "enum specifier", "a tag name or `{`", *found),
            | Self::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(found) =>
                write_expected(
                    f,
                    "enumerator list",
                    "an enumeration constant or `}`",
                    *found,
                ),
            | Self::ExpectedEnumeratorBeforeClosingCurlyBrace => {
                write!(f, "Expected an enumerator before `}}` in an enum body.")
            },
            | Self::ExpectedCommaOrClosingCurlyInEnumeratorList(found) => write_expected(
                f,
                "enumerator list",
                "`,` or `}` after an enumerator",
                *found,
            ),
            | Self::StorageClassRedefinition(last, new) => {
                write!(f, "Redefinition of storage class {last:?} with {new:?}!")
            },
            | Self::ConstSpecifiedTwice => {
                write!(f, "`const` keyword specified twice in type declaration!")
            },
            | Self::VolatileSpecifiedTwice => {
                write!(f, "`volatile` keyword specified twice in type declaration!")
            },
            | Self::RestrictSpecifiedTwice => {
                write!(f, "`restrict` keyword specified twice in type declaration!")
            },
            | Self::InlineSpecifiedTwice => {
                write!(
                    f,
                    "`inline` keyword specified twice in function declaration!"
                )
            },
            | Self::StaticSpecifiedTwice => {
                write!(f, "`static` keyword specified twice in array declarator!")
            },
            | Self::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator => write!(
                f,
                "Type qualifiers specified both before and after `static` in array direct \
                 declarator!"
            ),
            | Self::ConflictingTypeSpecifiers(specifiers, token) => {
                write!(
                    f,
                    "Conflicting type specifiers {specifiers:?} and {token:?}!"
                )
            },
            | Self::TypeSpecifierSpecifiedTwice(token) => {
                write!(f, "Type specifier {token:?} specified twice!")
            },
            | Self::LongSpecifiedThrice => {
                write!(
                    f,
                    "`long` keyword specified three times in type declaration!"
                )
            },
            | Self::LongLongDoubleSpecified => {
                write!(f, "`long long` and `double` cannot be combined!")
            },
            | Self::EmptyDeclarationSpecifiers(token) => {
                write!(f, "Expected declaration specifiers, found {token:?}!")
            },
            | Self::NoTypeSpecifiersInDeclarationSpecifiers(token) => {
                write!(f, "Expected a type specifier before {token:?}!")
            },
            | Self::BothStaticAndPointerInArrayDirectDeclarator => {
                write!(f, "Both `static` and `*` appeared in one array declarator!")
            },
            | Self::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator => {
                write!(f, "Expected an assignment expression after `static`!")
            },
            | Self::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(found) => {
                write!(
                    f,
                    "Expected `]` immediately after `*` in an array direct declarator; found \
                     {found:?}."
                )
            },
            | Self::UnexpectedEndOfArrayDeclaratorAfterPointer => write!(
                f,
                "Unexpected end of input after `*` in an array direct declarator; expected `]`."
            ),
            | Self::PointerSpecifiedTwice => {
                write!(
                    f,
                    "Pointer marker `*` specified twice in an array declarator."
                )
            },
            | Self::TypeQualifiersWithoutDeclarator => {
                write!(f, "Type qualifiers specified without a declarator.")
            },
            | Self::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator => write!(
                f,
                "Type qualifiers appeared before `*` in an array abstract declarator."
            ),
            | Self::KAndRFunctionDeclaratorMixedWithModernDeclarator => write!(
                f,
                "A K&R identifier list cannot contain a prototype-style parameter declaration."
            ),
            | Self::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                found,
            ) => write!(
                f,
                "Expected `)` immediately after `...` in a function declarator parameter list; \
                 found {found:?}."
            ),
            | Self::UnexpectedEndOfVariadicFunctionDeclaratorParameterList => write!(
                f,
                "Unexpected end of input after `...` in a function declarator parameter list; \
                 expected `)`."
            ),
            | Self::EmptyStructDeclarator => {
                write!(f, "Expected a declarator in the struct member declaration!")
            },
        }
    }
}

fn write_expected(
    f: &mut Formatter<'_>,
    grammar_position: &str,
    expected: &str,
    found: Option<TokenType>,
) -> FmtResult {
    match found {
        | Some(found) => write!(
            f,
            "Expected {expected} in {grammar_position}; found {found:?}."
        ),
        | None => write!(
            f,
            "Unexpected end of input in {grammar_position}; expected {expected}."
        ),
    }
}

impl TranslationPhase for Parser {
    type Item = ExternalDeclaration;

    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item> {
        self.drive(context)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        translation_phases::TranslationError,
        util::shared::SharedVec,
    };

    struct Parsed {
        parser:  Parser,
        context: Context,
        items:   Vec<ExternalDeclaration>,
        errors:  Vec<TranslationError>,
        source:  String,
    }

    fn parse(source: &str) -> Parsed {
        let source = source.to_owned();
        let mut context = Context::new();
        let preprocessor = Preprocessor::new(
            &mut context,
            PathBuf::from("<parser-test>").into_boxed_path(),
            source.clone().into(),
            SharedVec::default(),
            SharedVec::default(),
        );
        let mut parser = Parser::new(preprocessor);
        let mut items = Vec::new();
        while let Some(item) = parser.next_item(&mut context) {
            items.push(item);
            assert!(
                items.len() < 10_000,
                "parser failed to make item-level progress"
            );
        }
        let mut errors = Vec::new();
        while let Some(error) = context.pop_pending_error() {
            errors.push(error);
        }
        Parsed {
            parser,
            context,
            items,
            errors,
            source,
        }
    }

    fn declaration(parsed: &Parsed, item: usize) -> &Declaration {
        let index = match parsed.items[item] {
            | ExternalDeclaration::Declaration(index)
            | ExternalDeclaration::RecoveredDeclaration(index) => index,
            | ExternalDeclaration::FunctionDefinition(_)
            | ExternalDeclaration::RecoveredFunctionDefinition(_)
            | ExternalDeclaration::Error(_) => panic!("expected a declaration item"),
        };
        &parsed.parser.syntax.declarations[index.0 as usize]
    }

    fn function_definition(parsed: &Parsed, item: usize) -> &FunctionDefinition {
        let (ExternalDeclaration::FunctionDefinition(index)
        | ExternalDeclaration::RecoveredFunctionDefinition(index)) = parsed.items[item]
        else {
            panic!("expected a function-definition item")
        };
        &parsed.parser.syntax.function_definitions[index.0 as usize]
    }

    fn block_items(parsed: &Parsed, statement: StatementIndex) -> &[BlockItem] {
        let StatementType::Compound { items } =
            parsed.parser.syntax.statements[statement.0 as usize].kind
        else {
            panic!("expected a compound statement")
        };
        let start = items.start_index as usize;
        let end = start + items.length as usize;
        &parsed.parser.syntax.block_items[start..end]
    }

    fn init_declarators<'a>(parsed: &'a Parsed, declaration: &Declaration) -> &'a [InitDeclarator] {
        let start = declaration.init_declarators.start_index as usize;
        let end = start + declaration.init_declarators.length as usize;
        &parsed.parser.syntax.init_declarators[start..end]
    }

    fn identifier_name(parsed: &Parsed, declarator: Declarator) -> Option<String> {
        parsed
            .parser
            .declarator_identifier(declarator)
            .map(|identifier| parsed.context.string_cache.at(identifier.name).to_owned())
    }

    fn parser_errors(parsed: &Parsed) -> impl Iterator<Item = &ParserErrorType> {
        parsed.errors.iter().filter_map(|error| match error {
            | TranslationError::Parsing(error) => Some(&error.error_type),
            | _ => None,
        })
    }

    fn sourced_text(parsed: &Parsed, source_vectors: SourceVectors) -> String {
        parsed
            .context
            .get_source_vectors(source_vectors)
            .iter()
            .map(|vector| &parsed.source[vector.index..vector.index.saturating_add(vector.length)])
            .collect()
    }

    #[test]
    fn prototype_style_definition_produces_real_function_syntax() {
        let parsed = parse("int defined(int value) { return; }\n");

        assert!(
            parser_errors(&parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        let ExternalDeclaration::FunctionDefinition(index) = parsed.items[0] else {
            panic!("expected a function definition: {:#?}", parsed.items);
        };
        let definition = &parsed.parser.syntax.function_definitions[index.0 as usize];
        assert_eq!(
            identifier_name(&parsed, definition.declarator).as_deref(),
            Some("defined")
        );
        assert_eq!(
            sourced_text(&parsed, definition.source_vectors),
            "intdefined(intvalue){return;}"
        );
        assert!(!definition.recovered);
        assert!(matches!(
            parsed.parser.syntax.statements[definition.body.0 as usize].kind,
            StatementType::Compound { .. }
        ));
    }

    #[test]
    fn compound_blocks_preserve_mixed_items_and_every_statement_family() {
        let parsed = parse(
            "int all(int value) {\nint local;\n; value; label: ; case 1: ; default: ;\nif (value) \
             ; else ; switch (value) { case 1: ; default: ; }\nwhile (value) ; do ; while \
             (value);\nfor (;;) ; for (value; value; value) ; for (int i; i; i) ;\ngoto label; \
             continue; break; return; return value;\n}\n",
        );

        assert_eq!(parsed.items.len(), 1);
        let definition = function_definition(&parsed, 0);
        let items = block_items(&parsed, definition.body);
        assert!(matches!(items[0], BlockItem::Declaration(_)));
        assert!(
            items[1..]
                .iter()
                .all(|item| matches!(item, BlockItem::Statement(_)))
        );

        let kinds = items
            .iter()
            .filter_map(|item| match item {
                | BlockItem::Statement(index) =>
                    Some(parsed.parser.syntax.statements[index.0 as usize].kind),
                | BlockItem::Declaration(_) => None,
            })
            .collect::<Vec<_>>();
        assert!(kinds.iter().any(|kind| matches!(kind, StatementType::Null)));
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::Expression(_)))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::Label(..)))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::Case(..)))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::Default(..)))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::If { .. }))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::Switch { .. }))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::While { .. }))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::DoWhile { .. }))
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| matches!(kind, StatementType::For { .. }))
                .count(),
            3
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::Goto(_)))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::Continue))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind, StatementType::Break))
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| matches!(kind, StatementType::Return(_)))
                .count(),
            2
        );
        assert!(
            parser_errors(&parsed)
                .all(|error| matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
        assert!(parsed.parser.scopes.nested_scopes.is_empty());
        assert!(parsed.parser.label_scopes.is_empty());
        assert!(parsed.parser.switch_scopes.is_empty());
    }

    #[test]
    fn old_style_definition_keeps_its_parameter_declaration_list() {
        let parsed = parse(
            "int declared(int value);\nint (*pointer)(int value);\nint old_style(left, right) int \
             left; int right; { return; }\n",
        );

        assert!(matches!(
            parsed.items[0],
            ExternalDeclaration::Declaration(_)
        ));
        assert!(matches!(
            parsed.items[1],
            ExternalDeclaration::Declaration(_)
        ));
        let definition = function_definition(&parsed, 2);
        assert_eq!(definition.old_style_declarations.length, 2);
        let start = definition.old_style_declarations.start_index as usize;
        let names = parsed.parser.syntax.declaration_indices
            [start..start + definition.old_style_declarations.length as usize]
            .iter()
            .map(|index| {
                identifier_name(
                    &parsed,
                    init_declarators(
                        &parsed,
                        &parsed.parser.syntax.declarations[index.0 as usize],
                    )[0]
                    .declarator,
                )
                .expect("old-style declaration name")
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["left", "right"]);
        assert_eq!(
            sourced_text(&parsed, definition.source_vectors),
            "intold_style(left,right)intleft;intright;{return;}"
        );
        assert!(!definition.recovered);
        assert!(
            parser_errors(&parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );

        let nested_statements = format!(
            "int control(void) {{ {} ; }}\n",
            (0..127)
                .map(|index| if index % 2 == 0 {
                    "if (condition)"
                } else {
                    "while (condition)"
                })
                .collect::<Vec<_>>()
                .join(" ")
        );
        let parsed = parse(&nested_statements);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredFunctionDefinition(_))
        ));
        assert!(
            parser_errors(&parsed)
                .all(|error| matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
        assert!(parsed.parser.scopes.nested_scopes.is_empty());
        assert!(
            parsed
                .parser
                .trace
                .iter()
                .map(|event| event.depth)
                .max()
                .is_some_and(|depth| depth > 127)
        );
    }

    #[test]
    fn function_definition_publishes_identifier_bound_parameters() {
        let old_style = parse("int (*legacy(a))(int) int a; { return; }\n");
        let definition = function_definition(&old_style, 0);
        assert_eq!(definition.old_style_declarations.length, 1);
        assert!(!definition.recovered);
        assert!(
            parser_errors(&old_style).next().is_none(),
            "{:#?}",
            old_style.errors
        );

        let nested = parse("typedef int Y;\nint (*nested(int x))(int Y) { Y value; return 0; }\n");
        let definition = function_definition(&nested, 1);
        let items = block_items(&nested, definition.body);
        assert!(matches!(items[0], BlockItem::Declaration(_)));
        assert_eq!(
            identifier_name(
                &nested,
                init_declarators(
                    &nested,
                    &nested.parser.syntax.declarations[match items[0] {
                        | BlockItem::Declaration(index) => index.0 as usize,
                        | BlockItem::Statement(_) => unreachable!(),
                    }],
                )[0]
                .declarator,
            )
            .as_deref(),
            Some("value")
        );
        assert!(
            parser_errors(&nested)
                .all(|error| matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
    }

    #[test]
    fn old_style_parameter_recovery_preserves_the_function_body() {
        for source in [
            "int f(a) int a { return; }\n",
            "int f(a) int a = value { return; }\n",
        ] {
            let parsed = parse(source);
            let definition = function_definition(&parsed, 0);
            assert_eq!(definition.old_style_declarations.length, 1);
            let [BlockItem::Statement(statement)] = block_items(&parsed, definition.body) else {
                panic!("expected the recovered function body to retain its return statement")
            };
            assert!(matches!(
                parsed.parser.syntax.statements[statement.0 as usize].kind,
                StatementType::Return(None)
            ));
            assert!(parser_errors(&parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(Some(
                    TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                ))
            )));
        }
    }

    #[test]
    fn dangling_else_binds_to_the_nearest_unmatched_if() {
        let parsed = parse("int f(void) { if (outer) if (inner) ; else ; }\n");
        let definition = function_definition(&parsed, 0);
        let [BlockItem::Statement(outer)] = block_items(&parsed, definition.body) else {
            panic!("expected one outer if statement")
        };
        let StatementType::If {
            then_statement: inner,
            else_statement: outer_else,
            ..
        } = parsed.parser.syntax.statements[outer.0 as usize].kind
        else {
            panic!("expected outer if")
        };
        let StatementType::If { else_statement, .. } =
            parsed.parser.syntax.statements[inner.0 as usize].kind
        else {
            panic!("expected inner if")
        };
        assert!(outer_else.is_none());
        assert!(else_statement.is_some());
    }

    #[test]
    fn missing_if_body_leaves_else_for_its_enclosing_if() {
        let parsed = parse("int f(void) { if (condition) else value; }\n");
        let [BlockItem::Statement(if_statement)] =
            block_items(&parsed, function_definition(&parsed, 0).body)
        else {
            panic!("expected one if statement")
        };
        let StatementType::If {
            then_statement,
            else_statement: Some(else_statement),
            ..
        } = parsed.parser.syntax.statements[if_statement.0 as usize].kind
        else {
            panic!("expected a recovered if statement with an else branch")
        };
        assert!(matches!(
            parsed.parser.syntax.statements[then_statement.0 as usize].kind,
            StatementType::Null
        ));
        assert!(matches!(
            parsed.parser.syntax.statements[else_statement.0 as usize].kind,
            StatementType::Expression(ExpressionSlot::FutureChild(_))
        ));
    }

    #[test]
    fn nested_missing_if_body_leaves_else_for_its_enclosing_if() {
        let parsed = parse("int f(void) { if (outer) while (inner) else value; }\n");
        let [BlockItem::Statement(if_statement)] =
            block_items(&parsed, function_definition(&parsed, 0).body)
        else {
            panic!("expected one if statement")
        };
        let StatementType::If {
            then_statement,
            else_statement: Some(else_statement),
            ..
        } = parsed.parser.syntax.statements[if_statement.0 as usize].kind
        else {
            panic!("expected a recovered if statement with an else branch")
        };
        let StatementType::While { body_statement, .. } =
            parsed.parser.syntax.statements[then_statement.0 as usize].kind
        else {
            panic!("expected the if then-branch to be a while statement")
        };
        assert!(matches!(
            parsed.parser.syntax.statements[body_statement.0 as usize].kind,
            StatementType::Null
        ));
        assert!(matches!(
            parsed.parser.syntax.statements[else_statement.0 as usize].kind,
            StatementType::Expression(ExpressionSlot::FutureChild(_))
        ));
    }

    #[test]
    fn missing_else_body_leaves_a_later_else_for_its_enclosing_if() {
        let parsed = parse("int f(void) { if (outer) if (inner) ; else else value; }\n");
        let [BlockItem::Statement(if_statement)] =
            block_items(&parsed, function_definition(&parsed, 0).body)
        else {
            panic!("expected one if statement")
        };
        let StatementType::If {
            then_statement,
            else_statement: Some(outer_else),
            ..
        } = parsed.parser.syntax.statements[if_statement.0 as usize].kind
        else {
            panic!("expected the outer if to retain its else branch")
        };
        let StatementType::If {
            else_statement: Some(inner_else),
            ..
        } = parsed.parser.syntax.statements[then_statement.0 as usize].kind
        else {
            panic!("expected the inner if to retain its first else branch")
        };
        assert!(matches!(
            parsed.parser.syntax.statements[inner_else.0 as usize].kind,
            StatementType::Null
        ));
        assert!(matches!(
            parsed.parser.syntax.statements[outer_else.0 as usize].kind,
            StatementType::Expression(ExpressionSlot::FutureChild(_))
        ));
    }

    #[test]
    fn case_recovery_distinguishes_a_conditional_colon_from_the_label_colon() {
        let parsed = parse("int f(void) { switch (value) { case a ? b : c: ; } }\n");
        let [BlockItem::Statement(switch)] =
            block_items(&parsed, function_definition(&parsed, 0).body)
        else {
            panic!("expected one switch statement")
        };
        let StatementType::Switch { body_statement, .. } =
            parsed.parser.syntax.statements[switch.0 as usize].kind
        else {
            panic!("expected switch syntax")
        };
        let [BlockItem::Statement(case)] = block_items(&parsed, body_statement) else {
            panic!("expected one case label")
        };
        let StatementType::Case(ExpressionSlot::FutureChild(expression), _) =
            parsed.parser.syntax.statements[case.0 as usize].kind
        else {
            panic!("expected a deferred case expression")
        };

        assert_eq!(sourced_text(&parsed, expression), "a?b:c");
        assert!(
            parser_errors(&parsed)
                .all(|error| matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
    }

    #[test]
    fn case_recovery_does_not_steal_a_colon_after_a_closed_conditional_delimiter() {
        let parsed = parse("int f(void) { switch (value) { case (a ? b) : ; } }\n");
        let [BlockItem::Statement(switch)] =
            block_items(&parsed, function_definition(&parsed, 0).body)
        else {
            panic!("expected one switch statement")
        };
        let StatementType::Switch { body_statement, .. } =
            parsed.parser.syntax.statements[switch.0 as usize].kind
        else {
            panic!("expected switch syntax")
        };
        let [BlockItem::Statement(case)] = block_items(&parsed, body_statement) else {
            panic!("expected one case label")
        };
        let StatementType::Case(ExpressionSlot::FutureChild(expression), _) =
            parsed.parser.syntax.statements[case.0 as usize].kind
        else {
            panic!("expected a deferred case expression")
        };

        assert_eq!(sourced_text(&parsed, expression), "(a?b)");
        assert!(
            parser_errors(&parsed)
                .all(|error| matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
    }

    #[test]
    fn case_recovery_matches_an_outer_question_after_closed_inner_nesting() {
        let parsed = parse("int f(void) { switch (value) { case a ? (b ? c) : d : ; } }\n");
        let [BlockItem::Statement(switch)] =
            block_items(&parsed, function_definition(&parsed, 0).body)
        else {
            panic!("expected one switch statement")
        };
        let StatementType::Switch { body_statement, .. } =
            parsed.parser.syntax.statements[switch.0 as usize].kind
        else {
            panic!("expected switch syntax")
        };
        let [BlockItem::Statement(case)] = block_items(&parsed, body_statement) else {
            panic!("expected one case label")
        };
        let StatementType::Case(ExpressionSlot::FutureChild(expression), _) =
            parsed.parser.syntax.statements[case.0 as usize].kind
        else {
            panic!("expected a deferred case expression")
        };

        assert_eq!(sourced_text(&parsed, expression), "a?(b?c):d");
    }

    #[test]
    fn stray_else_consumes_its_token_and_preserves_following_items() {
        let parsed = parse("int f(void) { else; return; }\nint after;\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(first) if matches!(
            parsed.parser.syntax.statements[first.0 as usize].kind,
            StatementType::Null
        )));
        assert!(matches!(items[1], BlockItem::Statement(second) if matches!(
            parsed.parser.syntax.statements[second.0 as usize].kind,
            StatementType::Null
        )));
        assert!(matches!(items[2], BlockItem::Statement(third) if matches!(
            parsed.parser.syntax.statements[third.0 as usize].kind,
            StatementType::Return(None)
        )));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedStatement(Some(TokenType::Keyword(KeywordTokenType::Else)))
        )));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator,
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn duplicate_default_is_diagnosed_per_innermost_switch() {
        let parsed = parse(
            "int f(void) { switch (outer) { default: ; switch (inner) { default: ; } default: ; } \
             }\n",
        );

        assert_eq!(
            parser_errors(&parsed)
                .filter(|error| matches!(error, ParserErrorType::DuplicateDefaultLabel))
                .count(),
            1
        );
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredFunctionDefinition(_))
        ));
        assert!(parsed.parser.switch_scopes.is_empty());
    }

    #[test]
    fn for_slots_distinguish_absence_deferred_expressions_and_declarations() {
        let parsed = parse(
            "int f(void) {\nfor (;;) ;\nfor (start; ; ) ;\nfor (; condition; ) ;\nfor (; ; step) \
             ;\nfor (int item; condition; step) ;}\n",
        );
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        let forms = items
            .iter()
            .map(|item| {
                let BlockItem::Statement(index) = item else {
                    panic!("expected for statement")
                };
                let StatementType::For {
                    initializer,
                    condition_expression,
                    iteration_expression,
                    ..
                } = parsed.parser.syntax.statements[index.0 as usize].kind
                else {
                    panic!("expected for syntax")
                };
                (
                    initializer
                        .map(|initializer| matches!(initializer, ForInitializer::Declaration(_))),
                    condition_expression.is_some(),
                    iteration_expression.is_some(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            forms,
            [
                (None, false, false),
                (Some(false), false, false),
                (None, true, false),
                (None, false, true),
                (Some(true), true, true),
            ]
        );
    }

    #[test]
    fn missing_expressions_remain_distinct_from_present_deferred_children() {
        let parsed = parse("int f(void) { for () ; if (;) ; case ; return }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        let [
            BlockItem::Statement(for_statement),
            BlockItem::Statement(if_statement),
            BlockItem::Statement(case_statement),
            BlockItem::Statement(return_statement),
        ] = items
        else {
            panic!("expected four recovered statements: {items:#?}")
        };

        let StatementType::For {
            initializer,
            condition_expression,
            iteration_expression,
            ..
        } = parsed.parser.syntax.statements[for_statement.0 as usize].kind
        else {
            panic!("expected a for statement")
        };
        assert!(initializer.is_none());
        assert!(condition_expression.is_none());
        assert!(iteration_expression.is_none());

        let StatementType::If {
            condition_expression: ExpressionSlot::Missing(if_expression),
            ..
        } = parsed.parser.syntax.statements[if_statement.0 as usize].kind
        else {
            panic!("expected an if statement with a missing expression")
        };
        let StatementType::Case(ExpressionSlot::Missing(case_expression), _) =
            parsed.parser.syntax.statements[case_statement.0 as usize].kind
        else {
            panic!("expected a case statement with a missing expression")
        };
        assert_eq!(sourced_text(&parsed, if_expression), "");
        assert_eq!(sourced_text(&parsed, case_expression), "");
        assert!(matches!(
            parsed.parser.syntax.statements[return_statement.0 as usize].kind,
            StatementType::Return(None)
        ));
        assert!(
            parser_errors(&parsed)
                .all(|error| !matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
    }

    #[test]
    fn expression_recovery_preserves_following_statement_keywords() {
        let parsed = parse("int f(void) { return value break; return; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(Some(ExpressionSlot::FutureChild(_)))
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Break
        )));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(None)
        )));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedSemicolonInStatement("return statement", Some(_))
        )));
    }

    #[test]
    fn bare_return_recovery_preserves_following_statement_keywords() {
        let parsed = parse("int f(void) { return break; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(None)
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Break
        )));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedSemicolonInStatement(
                "return statement",
                Some(TokenType::Keyword(KeywordTokenType::Break))
            )
        )));
    }

    #[test]
    fn bare_return_recovery_preserves_following_declarations() {
        let parsed = parse("int f(void) { return int saved; break; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(None)
        )));
        assert!(matches!(items[1], BlockItem::Declaration(_)));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Break
        )));
    }

    #[test]
    fn block_declaration_recovery_preserves_following_statement_keywords() {
        let parsed = parse("int f(void) { int value return; break; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Declaration(_)));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(None)
        )));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Break
        )));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(Some(
                TokenType::Keyword(KeywordTokenType::Return)
            ))
        )));
    }

    #[test]
    fn expression_recovery_preserves_following_declarations() {
        let parsed = parse("int f(void) { value int saved; return; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Expression(ExpressionSlot::FutureChild(_))
        )));
        assert!(matches!(items[1], BlockItem::Declaration(_)));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(None)
        )));
    }

    #[test]
    fn expression_recovery_preserves_following_identifier_labels() {
        let parsed = parse("int f(void) { value label: ; return; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Expression(ExpressionSlot::FutureChild(_))
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Label(_, _)
        )));
        assert!(matches!(items[2], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(None)
        )));
    }

    #[test]
    fn compound_literals_survive_deferred_parenthesized_expression_recovery() {
        let parsed =
            parse("struct S { int x; };\nint f(void) { if ((struct S){0}.x) ; return; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 1).body);
        assert_eq!(items.len(), 2);
        let BlockItem::Statement(if_statement) = items[0] else {
            panic!("expected an if statement")
        };
        let StatementType::If {
            condition_expression: ExpressionSlot::FutureChild(expression),
            then_statement,
            else_statement: None,
        } = parsed.parser.syntax.statements[if_statement.0 as usize].kind
        else {
            panic!("expected a deferred if condition without an else branch")
        };
        assert!(sourced_text(&parsed, expression).contains("{0}"));
        assert!(matches!(
            parsed.parser.syntax.statements[then_statement.0 as usize].kind,
            StatementType::Null
        ));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(None)
        )));
    }

    #[test]
    fn malformed_condition_preserves_the_following_body_brace() {
        let parsed = parse("int f(void) { if (value { return; } break; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 2);
        let BlockItem::Statement(if_statement) = items[0] else {
            panic!("expected an if statement")
        };
        let StatementType::If {
            then_statement,
            else_statement: None,
            ..
        } = parsed.parser.syntax.statements[if_statement.0 as usize].kind
        else {
            panic!("expected a recovered if statement")
        };
        assert!(matches!(
            parsed.parser.syntax.statements[then_statement.0 as usize].kind,
            StatementType::Compound { .. }
        ));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Break
        )));
    }

    #[test]
    fn malformed_for_declaration_recovery_preserves_the_header_close() {
        let parsed = parse("int f(void) { for (int i) ; return; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 2);
        let BlockItem::Statement(for_statement) = items[0] else {
            panic!("expected a for statement")
        };
        let StatementType::For {
            initializer: Some(ForInitializer::Declaration(_)),
            condition_expression: None,
            iteration_expression: None,
            body_statement,
        } = parsed.parser.syntax.statements[for_statement.0 as usize].kind
        else {
            panic!("expected a recovered declaration-form for statement")
        };
        assert!(matches!(
            parsed.parser.syntax.statements[body_statement.0 as usize].kind,
            StatementType::Null
        ));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(None)
        )));
    }

    #[test]
    fn malformed_for_initializer_recovery_preserves_the_header_close() {
        let parsed = parse("int f(void) { for (int i = value) ; return; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 0).body);
        assert_eq!(items.len(), 2);
        let BlockItem::Statement(for_statement) = items[0] else {
            panic!("expected a for statement")
        };
        let StatementType::For {
            initializer: Some(ForInitializer::Declaration(declaration)),
            condition_expression: None,
            iteration_expression: None,
            body_statement,
        } = parsed.parser.syntax.statements[for_statement.0 as usize].kind
        else {
            panic!("expected a recovered declaration-form for statement")
        };
        assert_eq!(
            parsed.parser.syntax.declarations[declaration.0 as usize]
                .init_declarators
                .length,
            1
        );
        assert!(matches!(
            parsed.parser.syntax.statements[body_statement.0 as usize].kind,
            StatementType::Null
        ));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(None)
        )));
    }

    #[test]
    fn function_block_implicit_and_label_scopes_restore_typedef_classification() {
        let parsed = parse(
            "typedef int T; typedef int __func__;\nint f(int T) { T; __func__; T: ; goto T; { \
             typedef int U; U value; }\nif (T) ; while (T) ; for (int T; ; ) ; }\nT after; \
             __func__ outside;\n",
        );

        assert_eq!(parsed.items.len(), 5);
        let definition = function_definition(&parsed, 2);
        let items = block_items(&parsed, definition.body);
        assert!(matches!(items[0], BlockItem::Statement(_)));
        assert!(matches!(items[1], BlockItem::Statement(_)));
        assert!(matches!(items[2], BlockItem::Statement(_)));
        assert!(matches!(items[3], BlockItem::Statement(_)));
        assert!(parsed.parser.scopes.nested_scopes.is_empty());
        assert!(parsed.parser.label_scopes.is_empty());

        for kind in [
            ScopeKind::FunctionPrototype,
            ScopeKind::Function,
            ScopeKind::Block,
            ScopeKind::ImplicitSelection,
            ScopeKind::ImplicitIteration,
        ] {
            assert!(
                parsed
                    .parser
                    .scopes
                    .trace
                    .iter()
                    .any(|event| event.kind == kind && event.enter)
            );
            assert_eq!(
                parsed
                    .parser
                    .scopes
                    .trace
                    .iter()
                    .filter(|event| event.kind == kind && event.enter)
                    .count(),
                parsed
                    .parser
                    .scopes
                    .trace
                    .iter()
                    .filter(|event| event.kind == kind && !event.enter)
                    .count(),
                "scope kind {kind:?} leaked"
            );
        }
        assert!(
            declaration(&parsed, 3)
                .declaration_specifiers
                .type_specifiers
                .is_typedef_name()
        );
        assert!(
            declaration(&parsed, 4)
                .declaration_specifiers
                .type_specifiers
                .is_typedef_name()
        );
    }

    #[test]
    fn malformed_statement_delimiters_recover_the_body_and_next_file_item() {
        for source in [
            "int f(void) { expression } int after;\n",
            "int f(void) { return value } int after;\n",
            "int f(void) { if (value ; } int after;\n",
            "int f(void) { case value ; } int after;\n",
            "int f(void) { do ; while (value) } int after;\n",
            "int f(void) { for (value; value; value ; } int after;\n",
        ] {
            let parsed = parse(source);
            assert!(
                matches!(
                    parsed.items.first(),
                    Some(ExternalDeclaration::RecoveredFunctionDefinition(_))
                ),
                "missing recovered function for {source:?}: {:#?}",
                parsed.items
            );
            assert!(
                matches!(
                    parsed.items.get(1),
                    Some(ExternalDeclaration::Declaration(_))
                ),
                "recovery swallowed the next declaration for {source:?}: {:#?}",
                parsed.items
            );
            assert_eq!(
                identifier_name(
                    &parsed,
                    init_declarators(&parsed, declaration(&parsed, 1))[0].declarator,
                )
                .as_deref(),
                Some("after")
            );
            assert!(parsed.parser.scopes.nested_scopes.is_empty());
            assert!(parsed.parser.label_scopes.is_empty());
            assert!(parsed.errors.iter().all(|error| match error {
                | TranslationError::Parsing(error) => error.source_vectors.length > 0,
                | _ => true,
            }));
        }

        let eof = parse("int f(void) { if (value) return;");
        assert!(matches!(
            eof.items.first(),
            Some(ExternalDeclaration::RecoveredFunctionDefinition(_))
        ));
        assert!(parser_errors(&eof).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingCurlyBraceInCompoundStatement(None)
        )));
        assert!(eof.parser.scopes.nested_scopes.is_empty());
        assert!(eof.parser.label_scopes.is_empty());
    }

    #[test]
    fn premature_eof_unwinds_every_phase_03_frame_family() {
        for source in [
            "int f(void) {",
            "int f(void) { if (",
            "int f(void) { if (condition",
            "int f(void) { case value",
            "int f(void) { label:",
            "int f(void) { goto",
            "int f(void) { do ;",
            "int f(void) { for (",
            "int f(parameter) int parameter;",
        ] {
            let parsed = parse(source);
            assert!(
                matches!(
                    parsed.items.first(),
                    Some(ExternalDeclaration::RecoveredFunctionDefinition(_))
                ),
                "expected a recovered function for {source:?}: {:#?}",
                parsed.items
            );
            assert!(
                parser_errors(&parsed).next().is_some(),
                "expected a diagnostic for {source:?}"
            );
            assert!(parsed.parser.scopes.nested_scopes.is_empty(), "{source:?}");
            assert!(parsed.parser.label_scopes.is_empty(), "{source:?}");
            assert!(parsed.parser.switch_scopes.is_empty(), "{source:?}");
        }
    }

    #[test]
    fn statement_delimiter_trace_names_the_owning_frame() {
        let parsed = parse("int f(void) { int item; if (condition) return; }\n");

        let brace_events = parsed.parser.trace.iter().filter(|event| {
            event.action == "consume"
                && matches!(
                    event.token,
                    Some(TokenType::Operator(
                        OperatorTokenType::OpeningCurlyBrace | OperatorTokenType::ClosingCurlyBrace
                    ))
                )
        });
        assert!(
            brace_events
                .clone()
                .all(|event| event.frame == "compound-statement")
        );
        assert_eq!(brace_events.count(), 2);

        assert!(parsed.parser.trace.iter().any(|event| {
            event.frame == "declaration"
                && event.action == "consume"
                && event.token == Some(TokenType::Operator(OperatorTokenType::Semicolon))
        }));
        assert!(parsed.parser.trace.iter().any(|event| {
            event.frame == "statement"
                && event.action == "consume"
                && event.token == Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis))
        }));
    }

    #[test]
    fn blocks_and_definition_parameters_meet_the_c99_translation_floor() {
        let nested = format!(
            "int deep(void) {{{}{}{} }}\n",
            "{".repeat(127),
            ";",
            "}".repeat(127)
        );
        let parsed = parse(&nested);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::FunctionDefinition(_))
        ));
        assert!(
            parser_errors(&parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        assert!(
            parsed
                .parser
                .trace
                .iter()
                .map(|event| event.depth)
                .max()
                .is_some_and(|depth| depth > 127)
        );

        let parameters = (0..127)
            .map(|index| format!("int parameter_{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let parsed = parse(&format!("int many({parameters}) {{ return; }}\n"));
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::FunctionDefinition(_))
        ));
        assert_eq!(parsed.parser.syntax.parameter_declarations.len(), 127);
        assert!(
            parser_errors(&parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
    }

    #[test]
    fn specifier_only_declaration_runs_through_the_machine() {
        let parsed = parse("int;\n");

        assert_eq!(parsed.items.len(), 1);
        let declaration = declaration(&parsed, 0);
        assert_eq!(
            declaration.declaration_specifiers.type_specifiers,
            TypeSpecifiers::Int
        );
        assert_eq!(declaration.init_declarators.length, 0);
        assert!(parser_errors(&parsed).next().is_none());
        assert!(
            parsed.parser.trace.iter().any(|event| {
                event.frame == "declaration-specifiers" && event.action == "reduce"
            })
        );
        assert!(
            parsed
                .parser
                .trace
                .iter()
                .any(|event| event.token.is_some())
        );
        assert!(
            parsed
                .parser
                .trace
                .iter()
                .any(|event| { event.frame == "external-declaration" && event.action == "reduce" })
        );
    }

    #[test]
    fn empty_translation_unit_emits_one_dedicated_diagnostic() {
        let mut parsed = parse("");

        assert!(parsed.items.is_empty());
        assert_eq!(
            parser_errors(&parsed).collect::<Vec<_>>(),
            [&ParserErrorType::EmptyTranslationUnit]
        );
        assert_eq!(parsed.parser.next_item(&mut parsed.context), None);
        assert!(parsed.context.pop_pending_error().is_none());
    }

    #[test]
    fn ordinary_pointer_and_typedef_declarations_are_reachable() {
        let parsed = parse(
            "int x;\nint x2, y;\nconst unsigned long *p;\nint *const *volatile q;\ntypedef int \
             T;\nT value;\n",
        );

        assert_eq!(parsed.items.len(), 6);
        assert!(parser_errors(&parsed).next().is_none());
        let names = parsed
            .items
            .iter()
            .enumerate()
            .flat_map(|(item, _)| init_declarators(&parsed, declaration(&parsed, item)))
            .map(|init| identifier_name(&parsed, init.declarator).expect("named declarator"))
            .collect::<Vec<_>>();
        assert_eq!(names, ["x", "x2", "y", "p", "q", "T", "value"]);
        assert_eq!(
            declaration(&parsed, 5)
                .declaration_specifiers
                .type_specifiers,
            TypeSpecifiers::TypedefName(Identifier::new(
                parsed
                    .context
                    .string_cache
                    .get_id_from_string("T")
                    .expect("interned T")
            ))
        );

        let pointer_declaration = declaration(&parsed, 3);
        let pointer = init_declarators(&parsed, pointer_declaration)[0]
            .declarator
            .pointer
            .type_qualifiers_list;
        let start = pointer.start_index as usize;
        let end = start + pointer.length as usize;
        assert_eq!(
            &parsed.parser.syntax.type_qualifiers[start..end],
            &[TypeQualifiers::CONST, TypeQualifiers::VOLATILE]
        );
    }

    #[test]
    fn name_classification_is_published_after_each_declarator() {
        let parsed = parse("typedef int T, Prototype(T);\nint T, OldStyle(T);\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(
            parser_errors(&parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        assert!(parsed
            .parser
            .syntax
            .direct_declarators
            .iter()
            .any(|direct| matches!(direct, DirectDeclarator::Function { parameter_list, .. } if parameter_list.length == 1)));
        assert!(parsed
            .parser
            .syntax
            .direct_declarators
            .iter()
            .any(|direct| matches!(direct, DirectDeclarator::KAndRStyleFunction { parameters } if parameters.length == 1)));
        let t = parsed
            .context
            .string_cache
            .get_id_from_string("T")
            .expect("interned T");
        assert_eq!(
            parsed.parser.scopes.file_scope.get(&t),
            Some(&NameClass::Ordinary)
        );
    }

    #[test]
    fn duplicate_storage_class_keeps_the_last_class_for_typedef_publication() {
        let parsed = parse("extern typedef int T;\nT x;\n");

        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            declaration(&parsed, 0).declaration_specifiers.storage_class,
            StorageClass::Typedef
        );
        assert!(
            parser_errors(&parsed)
                .any(|error| matches!(error, ParserErrorType::StorageClassRedefinition(..)))
        );
        let t = parsed
            .context
            .string_cache
            .get_id_from_string("T")
            .expect("interned T");
        assert_eq!(
            declaration(&parsed, 1)
                .declaration_specifiers
                .type_specifiers,
            TypeSpecifiers::TypedefName(Identifier::new(t))
        );
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("x")
        );
    }

    #[test]
    fn conflicting_type_specifiers_are_anchored_to_the_conflicting_token() {
        let parsed = parse(
            "int float primitive; int struct S { int member; } tagged; int union U { int member; \
             } united; int enum E { A } enumerated;\n",
        );

        let mut conflict_sources = parsed
            .errors
            .iter()
            .filter_map(|error| match error {
                | TranslationError::Parsing(error)
                    if matches!(
                        error.error_type,
                        ParserErrorType::ConflictingTypeSpecifiers(..)
                    ) =>
                    Some(sourced_text(&parsed, error.source_vectors)),
                | _ => None,
            })
            .collect::<Vec<_>>();
        conflict_sources.sort_unstable();
        assert_eq!(conflict_sources, ["enum", "float", "struct", "union"]);
    }

    #[test]
    fn conflicting_typedef_names_remain_specifiers_and_preserve_following_declarations() {
        let parsed = parse(
            "typedef int T; unsigned T x; unsigned T (*pointer); unsigned T ((*nested)); T y;\n",
        );

        assert_eq!(parsed.items.len(), 5);
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ConflictingTypeSpecifiers(
                TypeSpecifiers::Unsigned,
                TokenType::Identifier
            )
        )));
        let conflict = parsed
            .errors
            .iter()
            .find_map(|error| match error {
                | TranslationError::Parsing(error)
                    if matches!(
                        error.error_type,
                        ParserErrorType::ConflictingTypeSpecifiers(
                            TypeSpecifiers::Unsigned,
                            TokenType::Identifier
                        )
                    ) =>
                    Some(error),
                | _ => None,
            })
            .expect("typedef conflict diagnostic");
        assert_eq!(sourced_text(&parsed, conflict.source_vectors), "T");
        assert_eq!(
            parsed
                .parser
                .syntax
                .init_declarators
                .iter()
                .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
                .collect::<Vec<_>>(),
            ["T", "x", "pointer", "nested", "y"]
        );
        assert!(
            declaration(&parsed, 4)
                .declaration_specifiers
                .type_specifiers
                .is_typedef_name()
        );
    }

    #[test]
    fn parenthesized_identifier_lists_preserve_typedef_shadowing() {
        let parsed = parse("typedef int T; unsigned T (x); T y;\n");

        assert_eq!(parsed.items.len(), 3);
        assert_eq!(
            [declaration(&parsed, 0), declaration(&parsed, 1)]
                .into_iter()
                .flat_map(|declaration| init_declarators(&parsed, declaration))
                .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
                .collect::<Vec<_>>(),
            ["T", "T"]
        );
        assert!(matches!(
            parsed.items[2],
            ExternalDeclaration::RecoveredDeclaration(_)
        ));
        let typedef_name = parsed
            .context
            .string_cache
            .get_id_from_string("T")
            .expect("interned typedef name");
        assert!(!parsed.parser.scopes.is_typedef(typedef_name));
    }

    #[test]
    fn arrays_functions_abstract_parameters_variadics_and_k_and_r_parse() {
        let parsed = parse(
            "int a[];\nint matrix[][];\nint f(int, const char *, ...);\nint old(a,b);\nint \
             (*factory(void))(int);\n",
        );

        assert_eq!(parsed.items.len(), 5);
        assert!(
            parser_errors(&parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        assert!(
            parsed
                .parser
                .syntax
                .direct_declarators
                .iter()
                .any(|direct| {
                    matches!(
                        direct,
                        DirectDeclarator::Array {
                            assignment_expression: None,
                            ..
                        }
                    )
                })
        );
        assert!(
            parsed
                .parser
                .syntax
                .direct_declarators
                .iter()
                .any(|direct| {
                    matches!(
                        direct,
                        DirectDeclarator::Function {
                            is_variadic: true,
                            ..
                        }
                    )
                })
        );
        assert!(parsed.parser.syntax.direct_declarators.iter().any(|direct| {
            matches!(direct, DirectDeclarator::KAndRStyleFunction { parameters } if parameters.length == 2)
        }));
    }

    #[test]
    fn union_kind_and_enum_arena_slice_are_correct() {
        let parsed = parse(
            "struct S;\nunion Forward;\nstruct { int anonymous; };\nunion U { int x; char y; \
             };\nstruct Outer { union { int nested; } value; };\nenum E { A, B, };\nenum { C, D \
             };\n",
        );

        assert_eq!(parsed.items.len(), 7);
        assert!(
            parser_errors(&parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        assert!(
            parsed
                .parser
                .syntax
                .struct_or_union_specifiers
                .iter()
                .any(|specifier| specifier.struct_or_union == StructOrUnion::Union)
        );
        let enum_specifier = parsed
            .parser
            .syntax
            .enum_specifiers
            .iter()
            .find(|specifier| specifier.name.is_some())
            .expect("named enum specifier");
        let enumeration_list = enum_specifier.enumeration_list.expect("enum body");
        assert_eq!(enumeration_list.length, 2);
        let start = enumeration_list.start_index as usize;
        let names = parsed.parser.syntax.enumerators[start..start + 2]
            .iter()
            .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
            .collect::<Vec<_>>();
        assert_eq!(names, ["A", "B"]);
        assert!(
            parsed
                .parser
                .syntax
                .enumerators
                .iter()
                .all(|enumerator| enumerator.source_vectors.length > 0)
        );
        assert!(
            parsed
                .parser
                .syntax
                .struct_declarations
                .iter()
                .map(|declaration| declaration.source_vectors)
                .chain(
                    parsed
                        .parser
                        .syntax
                        .struct_declarators
                        .iter()
                        .map(|declarator| declarator.source_vectors)
                )
                .all(|source_vectors| source_vectors.length > 0)
        );
    }

    #[test]
    fn specifier_combinations_and_conflicts_keep_legacy_diagnostics() {
        let parsed = parse(
            "extern const unsigned long int x;\ninline static double f(void);\nconst const int \
             duplicate;\nlong long double conflict;\ndouble long long reordered;\n",
        );

        assert_eq!(parsed.items.len(), 5);
        assert_eq!(
            declaration(&parsed, 0)
                .declaration_specifiers
                .type_specifiers,
            TypeSpecifiers::UnsignedLongInt
        );
        assert!(
            declaration(&parsed, 0)
                .declaration_specifiers
                .type_qualifiers
                .contains(TypeQualifiers::CONST)
        );
        assert!(
            declaration(&parsed, 1)
                .declaration_specifiers
                .function_specifiers
                .is_inline
        );
        assert!(
            parser_errors(&parsed)
                .any(|error| { matches!(error, ParserErrorType::ConstSpecifiedTwice) })
        );
        assert_eq!(
            parser_errors(&parsed)
                .filter(|error| matches!(error, ParserErrorType::LongLongDoubleSpecified))
                .count(),
            2
        );
        assert!(TypeSpecifiers::Long.is_long());
        assert!(TypeSpecifiers::LongDouble.is_long_double());
        assert!(TypeSpecifiers::StructOrUnion(StructOrUnionSpecifierIndex(0)).is_struct_or_union());
        assert!(TypeSpecifiers::Enum(EnumSpecifierIndex(0)).is_enum());
        let duplicate = parsed
            .context
            .string_cache
            .get_id_from_string("duplicate")
            .expect("interned identifier");
        assert!(TypeSpecifiers::TypedefName(Identifier::new(duplicate)).is_typedef_name());
    }

    #[test]
    fn expression_dependent_positions_use_typed_future_children() {
        let parsed = parse(
            "int bounded[4];\nstruct Bits { unsigned value:3; };\nenum Values { A=1, B };\nint \
             initialized=42;\nint function(void) { return 0; }\nint after;\n",
        );

        assert_eq!(parsed.items.len(), 6);
        let future_children = parser_errors(&parsed)
            .filter_map(|error| match error {
                | ParserErrorType::ArrayBoundExpressionNotImplemented =>
                    Some(FutureChildKind::ArrayBoundExpression),
                | ParserErrorType::BitFieldWidthExpressionNotImplemented =>
                    Some(FutureChildKind::BitFieldWidthExpression),
                | ParserErrorType::EnumeratorValueExpressionNotImplemented =>
                    Some(FutureChildKind::EnumeratorValueExpression),
                | ParserErrorType::InitializerNotImplemented => Some(FutureChildKind::Initializer),
                | ParserErrorType::StatementExpressionNotImplemented =>
                    Some(FutureChildKind::StatementExpression),
                | _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            future_children,
            [
                FutureChildKind::StatementExpression,
                FutureChildKind::Initializer,
                FutureChildKind::EnumeratorValueExpression,
                FutureChildKind::BitFieldWidthExpression,
                FutureChildKind::ArrayBoundExpression,
            ]
        );
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 5))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn only_function_declarators_can_own_a_braced_body() {
        let parsed = parse("int object { int swallowed; } int after;\n");

        assert!(
            !parser_errors(&parsed)
                .any(|error| matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(Some(
                TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
            ))
        )));
        let after_item = parsed.items.len() - 1;
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, after_item))[0].declarator
            )
            .as_deref(),
            Some("after")
        );

        let parsed = parse("int object, f() { int swallowed; } int after;\n");
        assert!(
            !parser_errors(&parsed)
                .any(|error| matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(Some(
                TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
            ))
        )));
        assert_eq!(init_declarators(&parsed, declaration(&parsed, 0)).len(), 2);
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn function_body_dispatch_follows_parenthesized_pointer_binding() {
        let function = parse("int (f()) { return 0; } int after;\n");
        assert!(
            parser_errors(&function)
                .any(|error| matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
        assert!(!parser_errors(&function).any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(Some(
                TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
            ))
        )));

        let pointer = parse("int (*fp)(void) { int swallowed; } int after;\n");
        assert!(
            !parser_errors(&pointer)
                .any(|error| matches!(error, ParserErrorType::StatementExpressionNotImplemented))
        );
        assert!(parser_errors(&pointer).any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(Some(
                TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
            ))
        )));
        let after_item = pointer.items.len() - 1;
        assert_eq!(
            identifier_name(
                &pointer,
                init_declarators(&pointer, declaration(&pointer, after_item))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn malformed_external_declarations_retain_recovered_nodes_and_continue() {
        let parsed = parse("}\nint after;\n");

        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(matches!(
            parsed.items.get(1),
            Some(ExternalDeclaration::Declaration(_))
        ));
        assert!(
            parser_errors(&parsed)
                .any(|error| matches!(error, ParserErrorType::EmptyDeclarationSpecifiers(..)))
        );
    }

    #[test]
    fn missing_declarators_skip_post_declarator_diagnostics() {
        for source in ["int", "int + int after;\n"] {
            let parsed = parse(source);

            assert!(
                parser_errors(&parsed).any(|error| matches!(
                    error,
                    ParserErrorType::ExpectedDeclaratorInDeclaration(_)
                ))
            );
            assert!(!parser_errors(&parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(_)
            )));
        }

        let parsed = parse("int + int after;\n");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn malformed_parameter_recovery_stops_at_comma_and_keeps_the_next_parameter() {
        let parsed = parse("int f(int x +, char y);\nint after;\n");

        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(matches!(
            parsed.items.get(1),
            Some(ExternalDeclaration::Declaration(_))
        ));
        assert_eq!(parsed.parser.syntax.parameter_declarations.len(), 2);
        assert_eq!(
            parsed
                .parser
                .syntax
                .parameter_declarations
                .iter()
                .map(|parameter| sourced_text(&parsed, parameter.source_vectors))
                .collect::<Vec<_>>(),
            ["intx", "chary"]
        );
        assert_eq!(
            parsed.parser.syntax.parameter_declarations[1]
                .declaration_specifiers
                .type_specifiers,
            TypeSpecifiers::Char
        );
        assert_eq!(
            parsed.parser.syntax.parameter_declarations[1]
                .declarator
                .and_then(|declarator| identifier_name(&parsed, declarator))
                .as_deref(),
            Some("y")
        );
    }

    #[test]
    fn omitted_parameter_comma_reprocesses_the_next_declaration_starter() {
        let parsed = parse("int f(int a int b);\n");

        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                Some(TokenType::Keyword(KeywordTokenType::Int))
            )
        )));
        assert_eq!(parsed.parser.syntax.parameter_declarations.len(), 2);
        assert_eq!(
            parsed
                .parser
                .syntax
                .parameter_declarations
                .iter()
                .filter_map(|parameter| parameter
                    .declarator
                    .and_then(|declarator| identifier_name(&parsed, declarator)))
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn omitted_struct_member_semicolon_reprocesses_the_next_declaration_starter() {
        let parsed = parse("struct S { int first int second; };\n");

        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(Some(
                TokenType::Keyword(KeywordTokenType::Int)
            ))
        )));
        assert_eq!(
            parsed
                .parser
                .syntax
                .struct_declarators
                .iter()
                .filter_map(|declarator| declarator
                    .declarator
                    .and_then(|declarator| identifier_name(&parsed, declarator)))
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn omitted_enumerator_comma_reprocesses_the_next_identifier() {
        let parsed = parse("enum E { A B, C };\n");

        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(Some(
                TokenType::Identifier
            ))
        )));
        assert_eq!(
            parsed
                .parser
                .syntax
                .enumerators
                .iter()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
    }

    #[test]
    fn named_parameter_declarators_retain_nested_k_and_r_identifier_lists() {
        let parsed = parse("int outer(int callback(arg));\n");

        assert!(
            parser_errors(&parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        let callback = parsed.parser.syntax.parameter_declarations[0]
            .declarator
            .expect("named callback declarator");
        let start = callback.kind.start_index as usize;
        let end = start + callback.kind.length as usize;
        let parameters = parsed.parser.syntax.direct_declarators[start..end]
            .iter()
            .find_map(|direct| match direct {
                | DirectDeclarator::KAndRStyleFunction { parameters } => Some(*parameters),
                | _ => None,
            })
            .expect("callback retains a K&R identifier-list suffix");
        assert_eq!(parameters.length, 1);
        assert_eq!(
            parsed
                .context
                .string_cache
                .at(parsed.parser.syntax.identifiers[parameters.start_index as usize].name),
            "arg"
        );
    }

    #[test]
    fn nested_recovery_stops_before_grammar_starters() {
        let parsed = parse("int f(int a + int b);\n");
        assert_eq!(
            parsed
                .parser
                .syntax
                .parameter_declarations
                .iter()
                .filter_map(|parameter| parameter
                    .declarator
                    .and_then(|declarator| identifier_name(&parsed, declarator)))
                .collect::<Vec<_>>(),
            ["a", "b"]
        );

        let parsed = parse("struct S { int first + int second; };\n");
        assert_eq!(
            parsed
                .parser
                .syntax
                .struct_declarators
                .iter()
                .filter_map(|declarator| declarator
                    .declarator
                    .and_then(|declarator| identifier_name(&parsed, declarator)))
                .collect::<Vec<_>>(),
            ["first", "second"]
        );

        let parsed = parse("struct S { int first : int; int second; };\n");
        assert_eq!(
            parsed
                .parser
                .syntax
                .struct_declarators
                .iter()
                .filter_map(|declarator| declarator
                    .declarator
                    .and_then(|declarator| identifier_name(&parsed, declarator)))
                .collect::<Vec<_>>(),
            ["first", "second"]
        );

        let parsed = parse("enum E { A + B, C };\n");
        assert_eq!(
            parsed
                .parser
                .syntax
                .enumerators
                .iter()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );

        let parsed = parse("int f(a + b, c);\n");
        assert_eq!(
            parsed
                .parser
                .syntax
                .identifiers
                .iter()
                .map(|identifier| parsed.context.string_cache.at(identifier.name))
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );

        let parsed = parse("enum E { A = + int after;\n");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );

        let parsed = parse("enum E { A = VALUE + OTHER, B };\n");
        assert_eq!(
            parsed
                .parser
                .syntax
                .enumerators
                .iter()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>(),
            ["A", "B"]
        );

        let parsed = parse("enum E { A = int, B };\n");
        assert_eq!(
            parsed
                .parser
                .syntax
                .enumerators
                .iter()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>(),
            ["A", "B"]
        );

        let parsed = parse("int array[int];\nint after;\n");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );

        let parsed = parse("int array[* int];\nint after;\n");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );

        let parsed = parse("int initialized = int;\nint after;\n");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );

        let parsed = parse("int f(int a, ... + int after;\n");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );

        let parsed = parse("int f(int a, ... int b);\nint after;\n");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 0))[0].declarator
            )
            .as_deref(),
            Some("f")
        );
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );

        let parsed = parse("int f(int a,);\nint after;\n");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parser_errors(&parsed).count(), 1);
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(Some(
                TokenType::Operator(OperatorTokenType::ClosingParenthesis)
            ))
        )));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn omitted_k_and_r_comma_reprocesses_the_next_identifier() {
        let parsed = parse("int f(a b, c);\n");

        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                Some(TokenType::Identifier)
            )
        )));
        assert_eq!(
            parsed
                .parser
                .syntax
                .identifiers
                .iter()
                .map(|identifier| parsed.context.string_cache.at(identifier.name))
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn malformed_parameter_after_ellipsis_terminates() {
        let parsed = parse("int f(int, ..., char trailing);\nint after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                TokenType::Operator(OperatorTokenType::Comma)
            )
        )));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn malformed_array_bound_recovery_stops_at_the_owning_bracket() {
        let parsed = parse("int a[(1];\nint after;\n");

        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(matches!(
            parsed.items.get(1),
            Some(ExternalDeclaration::Declaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn malformed_children_stop_at_unambiguous_owning_delimiters() {
        for source in [
            "int x = (1; int after;\n",
            "enum E { A = (1, B }; int after;\n",
            "int f(int x + [); int after;\n",
            "struct S { int x + ( ; }; int after;\n",
        ] {
            let parsed = parse(source);

            assert!(matches!(
                parsed.items.first(),
                Some(ExternalDeclaration::RecoveredDeclaration(_))
            ));
            assert!(matches!(
                parsed.items.get(1),
                Some(ExternalDeclaration::Declaration(_))
            ));
            assert_eq!(
                identifier_name(
                    &parsed,
                    init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
                )
                .as_deref(),
                Some("after"),
                "recovery swallowed the declaration after {source:?}"
            );
        }
    }

    #[test]
    fn malformed_initializer_recovery_preserves_the_next_declaration() {
        let parsed = parse("int x = + int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn malformed_array_bound_recovery_preserves_the_next_declaration() {
        let parsed = parse("int a[+ int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn deferred_expression_recovery_keeps_semicolons_inside_nested_braces() {
        for source in [
            "int array[sizeof(struct Inner { int member; })], after;\n",
            "int initialized = sizeof(struct Inner { int member; }), after;\n",
        ] {
            let parsed = parse(source);

            assert_eq!(
                parsed
                    .parser
                    .syntax
                    .init_declarators
                    .iter()
                    .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
                    .collect::<Vec<_>>(),
                if source.starts_with("int array") {
                    vec!["array", "after"]
                } else {
                    vec!["initialized", "after"]
                },
                "nested member semicolon escaped recovery for {source:?}"
            );
        }

        let parsed = parse("enum E { A = sizeof(struct Inner { int member; }), B };\n");
        assert_eq!(
            parsed
                .parser
                .syntax
                .enumerators
                .iter()
                .map(|enumerator| parsed.context.string_cache.at(enumerator.name.name))
                .collect::<Vec<_>>(),
            ["A", "B"]
        );

        let parsed = parse(
            "struct Outer { unsigned width : sizeof(struct Inner { int member; }); int after; };\n",
        );
        assert_eq!(
            parsed
                .parser
                .syntax
                .struct_declarators
                .iter()
                .filter_map(|declarator| {
                    declarator
                        .declarator
                        .and_then(|declarator| identifier_name(&parsed, declarator))
                })
                .collect::<Vec<_>>(),
            ["width", "after"]
        );
    }

    #[test]
    fn declaration_recovery_keeps_semicolons_inside_nested_braces() {
        let parsed = parse("int x + sizeof(struct Inner { int member; }); int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn initializer_recovery_unwinds_at_a_top_level_closing_brace() {
        let parsed = parse("int x = 1 } int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(Some(
                TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
            ))
        )));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn initializer_recovery_preserves_an_enclosing_brace_despite_unbalanced_children() {
        let parsed = parse("int x = (1 } int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn array_recovery_unwinds_at_the_enclosing_declaration_semicolon() {
        let parsed = parse("int a[1; int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(Some(
                TokenType::Operator(OperatorTokenType::Semicolon)
            ))
        )));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn array_recovery_unwinds_at_a_top_level_declarator_comma() {
        let parsed = parse("int a[1, b;\n");

        assert_eq!(
            parsed
                .parser
                .syntax
                .init_declarators
                .iter()
                .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(Some(
                TokenType::Operator(OperatorTokenType::Comma)
            ))
        )));
    }

    #[test]
    fn array_recovery_preserves_an_enclosing_closing_brace() {
        let parsed = parse("struct S { int a[1 } int after;\n");

        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            parsed
                .parser
                .syntax
                .init_declarators
                .iter()
                .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
                .collect::<Vec<_>>(),
            ["after"]
        );
    }

    #[test]
    fn array_recovery_preserves_an_enclosing_closing_parenthesis() {
        let parsed = parse("int f(int a[1) int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn struct_recovery_preserves_an_enclosing_closing_parenthesis() {
        let parsed = parse("int f(struct S { int x + ) int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn array_recovery_unwinds_at_semicolons_despite_unbalanced_children() {
        let parsed = parse("int a[(1; int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn parameter_recovery_unwinds_at_the_enclosing_declaration_semicolon() {
        let parsed = parse("int f(int x; int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                Some(TokenType::Operator(OperatorTokenType::Semicolon))
            )
        )));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn parameter_recovery_unwinds_at_semicolons_despite_unbalanced_children() {
        let parsed = parse("int f(int x + (1; int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn parameter_recovery_preserves_an_enclosing_closing_brace() {
        let parsed = parse("int f(int x } int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                Some(TokenType::Operator(OperatorTokenType::ClosingCurlyBrace))
            )
        )));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn enum_recovery_unwinds_at_the_enclosing_declaration_semicolon() {
        for source in ["enum E { A; int after;\n", "enum E { A = (1; int after;\n"] {
            let parsed = parse(source);

            assert_eq!(parsed.items.len(), 2);
            assert!(matches!(
                parsed.items.first(),
                Some(ExternalDeclaration::RecoveredDeclaration(_))
            ));
            assert!(parser_errors(&parsed).any(|error| matches!(
                error,
                ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(Some(
                    TokenType::Operator(OperatorTokenType::Semicolon)
                ))
            )));
            assert_eq!(
                identifier_name(
                    &parsed,
                    init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
                )
                .as_deref(),
                Some("after"),
                "enum recovery swallowed the declaration after {source:?}"
            );
        }
    }

    #[test]
    fn enum_recovery_preserves_an_enclosing_closing_parenthesis() {
        let parsed = parse("int f(enum E { A + ) int after;\n");

        assert_eq!(parsed.items.len(), 2);
        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn malformed_declaration_recovery_stops_at_comma_and_keeps_next_declarator() {
        let parsed = parse("int x +, y; int after;\n");

        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(matches!(
            parsed.items.get(1),
            Some(ExternalDeclaration::Declaration(_))
        ));
        assert_eq!(
            parsed
                .parser
                .syntax
                .init_declarators
                .iter()
                .filter_map(|declarator| identifier_name(&parsed, declarator.declarator))
                .collect::<Vec<_>>(),
            ["x", "y", "after"]
        );
    }

    #[test]
    fn direct_recovery_sources_are_retained_by_the_recovered_declaration() {
        for (source, expected) in [
            ("int x +;\n", "intx+;"),
            ("}\nint after;\n", "}"),
            ("int x }\nint after;\n", "intx}"),
        ] {
            let parsed = parse(source);
            let Some(ExternalDeclaration::RecoveredDeclaration(index)) = parsed.items.first()
            else {
                panic!("malformed declaration should retain recovered syntax")
            };
            let source_vectors = parsed.parser.syntax.declarations[index.0 as usize].source_vectors;

            assert_eq!(
                sourced_text(&parsed, source_vectors),
                expected,
                "error-node provenance did not retain all owned tokens for {source:?}"
            );
        }
    }

    #[test]
    fn recovery_trace_identifies_the_owner_of_every_consumed_token() {
        let parsed = parse("int x + (1);\n");
        let recovery = parsed
            .parser
            .trace
            .iter()
            .filter(|event| event.action == "recover-consume")
            .collect::<Vec<_>>();

        assert!(recovery.iter().all(|event| event.frame == "declaration"));
        assert_eq!(
            recovery
                .iter()
                .filter_map(|event| event.token)
                .collect::<Vec<_>>(),
            [
                TokenType::Operator(OperatorTokenType::Plus),
                TokenType::Operator(OperatorTokenType::OpeningParenthesis),
                TokenType::Integer(IntegerTokenType::Int(1)),
                TokenType::Operator(OperatorTokenType::ClosingParenthesis),
            ]
        );
    }

    #[test]
    fn unnamed_bit_field_after_member_comma_does_not_require_a_declarator() {
        let parsed = parse("struct S { int named, : 3; };\n");

        assert!(!parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(..)
        )));
        assert_eq!(parsed.parser.syntax.struct_declarators.len(), 2);
        assert!(
            parsed.parser.syntax.struct_declarators[1]
                .declarator
                .is_none()
        );
        assert_eq!(
            parsed
                .parser
                .syntax
                .struct_declarators
                .iter()
                .map(|declarator| sourced_text(&parsed, declarator.source_vectors))
                .collect::<Vec<_>>(),
            ["named", ":3"]
        );
    }

    #[test]
    fn prototype_enumerators_stop_hiding_file_scope_typedefs_at_the_closing_parenthesis() {
        let parsed = parse("typedef int A; int f(enum { A } x); A y;\n");

        assert_eq!(parsed.items.len(), 3);
        assert!(
            parser_errors(&parsed).next().is_none(),
            "{:#?}",
            parsed.errors
        );
        let a = parsed
            .context
            .string_cache
            .get_id_from_string("A")
            .expect("interned A");
        assert_eq!(
            parsed.parser.scopes.file_scope.get(&a),
            Some(&NameClass::Typedef)
        );
        assert_eq!(
            declaration(&parsed, 2)
                .declaration_specifiers
                .type_specifiers,
            TypeSpecifiers::TypedefName(Identifier::new(a))
        );
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 2))[0].declarator
            )
            .as_deref(),
            Some("y")
        );
    }

    #[test]
    fn definition_parameter_enumerators_are_visible_in_the_function_body() {
        let parsed = parse("typedef int A; int f(enum { A } x) { A; return 0; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 1).body);

        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Expression(ExpressionSlot::FutureChild(_))
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(Some(ExpressionSlot::FutureChild(_)))
        )));
    }

    #[test]
    fn nested_definition_parameter_enumerators_are_visible_in_the_function_body() {
        let parsed = parse("typedef int A; int f(struct { enum { A } e; } x) { A; return 0; }\n");
        let items = block_items(&parsed, function_definition(&parsed, 1).body);

        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Expression(ExpressionSlot::FutureChild(_))
        )));
        assert!(matches!(items[1], BlockItem::Statement(index) if matches!(
            parsed.parser.syntax.statements[index.0 as usize].kind,
            StatementType::Return(Some(ExpressionSlot::FutureChild(_)))
        )));
    }

    #[test]
    fn named_parameters_hide_typedefs_for_later_prototype_parameters() {
        let parsed = parse("typedef int T; int f(int T, T x);\n");

        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::NoTypeSpecifiersInDeclarationSpecifiers(TokenType::Identifier)
        )));
        assert_eq!(parsed.parser.syntax.parameter_declarations.len(), 2);
        assert_eq!(
            parsed.parser.syntax.parameter_declarations[1]
                .declaration_specifiers
                .type_specifiers,
            TypeSpecifiers::Empty
        );
    }

    #[test]
    fn array_recovery_consumes_nested_brackets_before_the_owning_bracket() {
        let parsed = parse("int a[sizeof(int[2])][*];\n");
        let declarator = parsed.parser.syntax.init_declarators[0].declarator;
        let start = declarator.kind.start_index as usize;
        let end = start + declarator.kind.length as usize;

        assert_eq!(
            parsed.parser.syntax.direct_declarators[start..end]
                .iter()
                .filter(|direct| matches!(direct, DirectDeclarator::Array { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn parameter_recovery_consumes_nested_parentheses_before_the_owning_separator() {
        let parsed = parse("int f(int x + (1), char y);\nint after;\n");

        assert_eq!(parsed.parser.syntax.parameter_declarations.len(), 2);
        assert_eq!(
            parsed.parser.syntax.parameter_declarations[1]
                .declaration_specifiers
                .type_specifiers,
            TypeSpecifiers::Char
        );
        assert_eq!(
            parsed.parser.syntax.parameter_declarations[1]
                .declarator
                .and_then(|declarator| identifier_name(&parsed, declarator))
                .as_deref(),
            Some("y")
        );
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration(&parsed, 1))[0].declarator
            )
            .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn pointer_array_declarator_reports_its_specific_closing_bracket_diagnostic() {
        let parsed = parse("int array[* trailing];\n");

        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(
                TokenType::Identifier
            )
        )));
    }

    #[test]
    fn pointer_array_specific_diagnostics_do_not_fall_through_to_generic_recovery() {
        let at_semicolon = parse("int array[*;\n");
        assert!(matches!(
            at_semicolon.items.first(),
            Some(ExternalDeclaration::RecoveredDeclaration(_))
        ));
        assert!(parser_errors(&at_semicolon).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(
                TokenType::Operator(OperatorTokenType::Semicolon)
            )
        )));
        assert!(!parser_errors(&at_semicolon).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(..)
        )));

        let at_eof = parse("int array[*");
        assert!(parser_errors(&at_eof).any(|error| matches!(
            error,
            ParserErrorType::UnexpectedEndOfArrayDeclaratorAfterPointer
        )));
        assert!(!parser_errors(&at_eof).any(|error| matches!(
            error,
            ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(..)
        )));
    }

    #[test]
    fn hard_syntax_errors_retain_an_explicitly_recovered_declaration() {
        let parsed = parse("int array[*;\n");
        let Some(ExternalDeclaration::RecoveredDeclaration(index)) = parsed.items.first() else {
            panic!("hard syntax errors should retain a recovered declaration");
        };

        let declaration = &parsed.parser.syntax.declarations[index.0 as usize];
        assert_eq!(declaration.init_declarators.length, 1);
        assert!(parser_errors(&parsed).any(|error| {
            matches!(
                error,
                ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(
                    TokenType::Operator(OperatorTokenType::Semicolon)
                )
            ) && error.severity() == ErrorSeverity::Error
        }));
        assert_eq!(
            identifier_name(
                &parsed,
                init_declarators(&parsed, declaration)[0].declarator
            )
            .as_deref(),
            Some("array")
        );
    }

    #[test]
    fn struct_recovery_distinguishes_a_closing_parenthesis_from_eof() {
        let parsed = parse("int f(struct S { int x + ) int after;\n");
        let error = parsed
            .errors
            .iter()
            .find_map(|error| match error {
                | TranslationError::Parsing(error)
                    if matches!(
                        error.error_type,
                        ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(Some(
                            TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                        ))
                    ) =>
                    Some(error),
                | _ => None,
            })
            .expect("missing closing-curly diagnostic anchored to `)`");

        assert_eq!(
            error.to_string(),
            "Expected `}` in struct declaration list; found Operator(ClosingParenthesis)."
        );
    }

    #[test]
    fn missing_parameter_after_comma_avoids_specifier_cascade_diagnostics() {
        let parsed = parse("int function(int,\n");
        let errors = parser_errors(&parsed).collect::<Vec<_>>();

        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|error| matches!(
            error,
            ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(None)
        )));
        assert!(errors.iter().any(|error| matches!(
            error,
            ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(None)
        )));
        assert!(!errors.iter().any(|error| matches!(
            error,
            ParserErrorType::UnexpectedEndBeforeDeclarationSpecifier
                | ParserErrorType::UnexpectedEndBeforeTypeSpecifier
        )));
    }

    #[test]
    fn pointer_without_direct_declarator_keeps_the_legacy_diagnostic() {
        for source in ["int *;\n", "int *\n"] {
            let parsed = parse(source);

            assert_eq!(
                parser_errors(&parsed)
                    .filter(|error| matches!(
                        error,
                        ParserErrorType::TypeQualifiersWithoutDeclarator
                    ))
                    .count(),
                1,
                "missing pointer-without-declarator diagnostic for {source:?}"
            );
            assert_eq!(
                parser_errors(&parsed)
                    .filter(|error| matches!(
                        error,
                        ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(..)
                    ))
                    .count(),
                1,
                "generic direct-declarator diagnostic count changed for {source:?}"
            );
        }
    }

    #[test]
    fn duplicate_pointer_array_marker_keeps_its_legacy_diagnostic() {
        let parsed = parse("int array[**];\n");

        assert!(
            parser_errors(&parsed)
                .any(|error| matches!(error, ParserErrorType::PointerSpecifiedTwice))
        );
    }

    #[test]
    fn qualifiers_before_an_abstract_array_pointer_keep_their_legacy_diagnostic() {
        let parsed = parse("int function(int [const *]);\n");

        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator
        )));
    }

    #[test]
    fn mixed_k_and_r_and_prototype_parameters_keep_their_legacy_diagnostic() {
        let parsed = parse("int function(first, int second);\n");

        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator
        )));
    }

    #[test]
    fn array_qualifiers_on_both_sides_of_static_keep_the_legacy_diagnostic() {
        let parsed = parse("int values[const static volatile 4];\n");

        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator
        )));
    }

    #[test]
    fn migrated_nodes_retain_their_exact_owned_token_provenance() {
        let parsed = parse(
            "int (*value);\nint function(const char *name, unsigned count);\nstruct S { int \
             first, *second; unsigned bits:3; };\n",
        );

        let parenthesized = init_declarators(&parsed, declaration(&parsed, 0))[0].declarator;
        let parenthesized_source = parenthesized.source_vectors;
        assert_eq!(sourced_text(&parsed, parenthesized_source), "(*value)");
        assert_eq!(
            identifier_name(&parsed, parenthesized).as_deref(),
            Some("value")
        );

        let parameter_text = parsed
            .parser
            .syntax
            .parameter_declarations
            .iter()
            .map(|parameter| sourced_text(&parsed, parameter.source_vectors))
            .collect::<Vec<_>>();
        assert_eq!(parameter_text, ["constchar*name", "unsignedcount"]);

        let member_text = parsed
            .parser
            .syntax
            .struct_declarations
            .iter()
            .map(|declaration| sourced_text(&parsed, declaration.source_vectors))
            .collect::<Vec<_>>();
        assert_eq!(member_text, ["intfirst,*second;", "unsignedbits:3;"]);
        let member_declarator_text = parsed
            .parser
            .syntax
            .struct_declarators
            .iter()
            .map(|declarator| sourced_text(&parsed, declarator.source_vectors))
            .collect::<Vec<_>>();
        assert_eq!(member_declarator_text, ["first", "*second", "bits:3"]);
    }

    #[test]
    fn empty_abstract_function_declarator_owns_both_parentheses() {
        let parsed = parse("int f(int ());\n");
        let declarator = parsed.parser.syntax.parameter_declarations[0]
            .declarator
            .expect("abstract function declarator");

        assert_eq!(sourced_text(&parsed, declarator.source_vectors), "()");
    }

    #[test]
    fn malformed_and_eof_paths_terminate_with_source_backed_diagnostics() {
        type DiagnosticMatcher = fn(&ParserErrorType) -> bool;
        let cases: &[(&str, DiagnosticMatcher)] = &[
            ("int\n", |error| {
                matches!(
                    error,
                    ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(
                        None
                    )
                )
            }),
            ("int *\n", |error| {
                matches!(
                    error,
                    ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(
                        None
                    )
                )
            }),
            ("int (value\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(None)
                )
            }),
            ("int (value;\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(Some(
                        TokenType::Operator(OperatorTokenType::Semicolon)
                    ))
                )
            }),
            ("int array[\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(None)
                )
            }),
            ("int array[*;\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(
                        TokenType::Operator(OperatorTokenType::Semicolon)
                    )
                )
            }),
            ("int function(int\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                        None
                    )
                )
            }),
            ("int function(int, ...;\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                        TokenType::Operator(OperatorTokenType::Semicolon)
                    )
                )
            }),
            ("struct S { int member\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(None)
                )
            }),
            ("struct S { int first,\n", |error| {
                matches!(
                    error,
                    ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(
                        None
                    )
                )
            }),
            ("struct S { int member }\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList
                )
            }),
            ("struct {\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(..)
                )
            }),
            ("struct S {};\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedStructDeclarationBeforeClosingCurlyBrace
                )
            }),
            ("union U {};\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedStructDeclarationBeforeClosingCurlyBrace
                )
            }),
            ("enum E { A\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(None)
                )
            }),
            ("enum E { A,, B };\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                        Some(TokenType::Operator(OperatorTokenType::Comma))
                    )
                )
            }),
            ("enum {\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                        None
                    )
                )
            }),
            ("enum E {};\n", |error| {
                matches!(
                    error,
                    ParserErrorType::ExpectedEnumeratorBeforeClosingCurlyBrace
                )
            }),
            ("typedef ;\n", |error| {
                matches!(error, ParserErrorType::ExpectedDeclaratorInTypedef(..))
            }),
            ("}\nint after;\n", |error| {
                matches!(error, ParserErrorType::EmptyDeclarationSpecifiers(..))
            }),
        ];

        for (source, expected_diagnostic) in cases {
            let parsed = parse(source);
            let errors = parsed
                .errors
                .iter()
                .filter_map(|error| match error {
                    | TranslationError::Parsing(error) => Some(error),
                    | _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                errors
                    .iter()
                    .any(|error| expected_diagnostic(&error.error_type)),
                "missing expected parser diagnostic for {source:?}: {errors:#?}"
            );
            assert!(errors.iter().all(|error| error.source_vectors.length > 0));
        }

        for source in ["struct S {};\n", "union U {};\n", "enum E {};\n"] {
            let parsed = parse(source);
            assert!(matches!(
                parsed.items.first(),
                Some(ExternalDeclaration::RecoveredDeclaration(_))
            ));
        }
    }

    #[test]
    fn parenthesized_declarators_meet_the_c99_floor_and_stress_the_heap_stack() {
        for depth in [63, 4_096] {
            let source = format!("int {}deep{};\n", "(".repeat(depth), ")".repeat(depth));
            let parsed = parse(&source);

            assert_eq!(parsed.items.len(), 1);
            assert!(
                parser_errors(&parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            assert!(
                parsed
                    .parser
                    .trace
                    .iter()
                    .map(|event| event.depth)
                    .max()
                    .expect("nonempty trace")
                    > depth,
                "grammar depth must be represented by heap-backed frames"
            );
            assert!(
                parsed.context.source_vectors.0.len() <= depth * 8 + 32,
                "source provenance must grow linearly with grammar depth"
            );
        }
    }
}
