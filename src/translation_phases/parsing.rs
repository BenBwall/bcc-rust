use std::{
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
#[expect(
    unused_imports,
    reason = "We'll definitely need HashMap and HashSet in the future."
)]
use crate::util::{
    HashMap,
    HashSet,
};
use crate::{
    translation_phases::preprocessing::OperatorTokenType,
    util::{
        string_cache::StringCacheId,
        vector_slice::{
            UsizeExt,
            VectorSlice,
        },
    },
};

pub(crate) struct Parser {
    cursor:              TokenCursor,
    frames:              Vec<ParseFrame>,
    returned:            Option<ParseValue>,
    syntax:              SyntaxStore,
    scopes:              ScopeStack,
    recovery:            RecoveryState,
    invalid_event_count: usize,
    #[cfg(test)]
    trace:               Vec<FrameTraceEvent>,
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
}

/// init-declarator:
/// - declarator
/// - declarator = initializer
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
/// Each variant represents a valid combination of type specifiers. For
/// example, the declaration `unsigned int foo` would be represented as
/// `TypeSpecifiers::UnsignedInt`.
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

        fn $map_fn_name(self, parser: &mut Parser, context: &mut Context) -> Self {
            match self {
                $($map_match_pattern $(if $map_guard)? => $map_result,)+
                | type_specifier => {
                    let source_vectors = context.create_source_vectors(
                        parser.position(context),
                        parser.source_file_index(),
                        0,
                    );
                    context.parser_error(ParserError {
                        error_type: ParserErrorType::ConflictingTypeSpecifiers(
                            type_specifier,
                            TokenType::Keyword(KeywordTokenType::$keyword_token_type),
                        ),
                        source_vectors,
                    });
                    type_specifier
                },
            }
        }

        fn $make_fn_name(&mut self, parser: &mut Parser, context: &mut Context) {
            *self = self.$map_fn_name(parser, context);
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
        parser: &mut Parser,
        context: &mut Context,
        index: StructOrUnionSpecifierIndex,
    ) -> Self {
        match self {
            | TypeSpecifiers::Empty => TypeSpecifiers::StructOrUnion(index),
            | type_specifiers => {
                let source_vectors = context.create_source_vectors(
                    parser.position(context),
                    parser.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::ConflictingTypeSpecifiers(
                        type_specifiers,
                        TokenType::Keyword(KeywordTokenType::Struct),
                    ),
                    source_vectors,
                });
                type_specifiers
            },
        }
    }

    fn make_struct_or_union(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        index: StructOrUnionSpecifierIndex,
    ) {
        *self = self.map_struct_or_union(parser, context, index);
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

    fn map_enum(
        self,
        parser: &mut Parser,
        context: &mut Context,
        index: EnumSpecifierIndex,
    ) -> Self {
        match self {
            | TypeSpecifiers::Empty => TypeSpecifiers::Enum(index),
            | type_specifiers => {
                let source_vectors = context.create_source_vectors(
                    parser.position(context),
                    parser.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::ConflictingTypeSpecifiers(
                        type_specifiers,
                        TokenType::Keyword(KeywordTokenType::Enum),
                    ),
                    source_vectors,
                });
                type_specifiers
            },
        }
    }

    fn make_enum(&mut self, parser: &mut Parser, context: &mut Context, index: EnumSpecifierIndex) {
        *self = self.map_enum(parser, context, index);
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

    fn map_typedef_name(
        self,
        parser: &mut Parser,
        context: &mut Context,
        name: Identifier,
    ) -> Self {
        match self {
            | TypeSpecifiers::Empty => TypeSpecifiers::TypedefName(name),
            | type_specifiers => {
                let source_vectors = context.create_source_vectors(
                    parser.position(context),
                    parser.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::ConflictingTypeSpecifiers(
                        type_specifiers,
                        TokenType::Identifier,
                    ),
                    source_vectors,
                });
                type_specifiers
            },
        }
    }

    fn make_typedef_name(&mut self, parser: &mut Parser, context: &mut Context, name: Identifier) {
        *self = self.map_typedef_name(parser, context, name);
    }
}

/// struct-or-union-specifier:
/// - struct-or-union identifier? { struct-declaration-list }
/// - struct-or-union identifier
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct StructOrUnionSpecifier {
    pub(crate) struct_or_union:         StructOrUnion,
    pub(crate) identifier:              Option<Identifier>,
    /// None indicates that the body is missing. An empty vector indicates an
    /// empty body. `struct Foo;` has no body. `struct Foo {};` has an empty
    /// body.
    pub(crate) struct_declaration_list: Option<VectorSlice<StructDeclaration>>,
}

/// struct-or-union:
/// - struct
/// - union
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum StructOrUnion {
    Struct,
    Union,
}

/// struct-declaration:
/// - specifier-qualifier-list struct-declarator-list ;
///
/// `type_qualifiers` and `type_specifiers` are split into two fields.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct StructDeclaration {
    type_qualifiers:        TypeQualifiers,
    type_specifiers:        TypeSpecifiers,
    struct_declarator_list: VectorSlice<StructDeclarator>,
}

/// struct-declarator:
/// - declarator
/// - declarator? : constant-expression
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct StructDeclarator {
    declarator:     Option<Declarator>,
    bitfield_width: Option<ConstantExpressionIndex>,
}

/// enum-specifier:
/// - enum identifier? { enumerator-list }
/// - enum identifier? { enumerator-list , }
/// - enum identifier
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct EnumSpecifier {
    pub(crate) name:             Option<Identifier>,
    pub(crate) enumeration_list: Option<VectorSlice<Enumerator>>,
}

/// enumerator:
/// - enumeration-constant
/// - enumeration-constant = constant-expression
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Enumerator {
    pub(crate) name:       Identifier,
    pub(crate) expression: Option<ConstantExpressionIndex>,
}

/// function-specifier:
/// - inline
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
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Declarator {
    pub(crate) pointer_declarator: PointerDeclarator,
    pub(crate) kind:               VectorSlice<DirectDeclarator>,
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
/// - direct-abstract-declarator? [ * ]
/// - direct-abstract-declarator? ( parameter-type-list? )
///
/// Represents both direct-declarators and direct-abstract-declarators.
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
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ParameterDeclaration {
    declaration_specifiers: DeclarationSpecifiers,
    /// Could be a declarator or an abstract declarator or neither.
    declarator:             Option<Declarator>,
}

// The Phase 02 parser machine is implemented below the retained syntax model.

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TypeName {
    pub(crate) declaration_specifiers: DeclarationSpecifiers,
    pub(crate) declarator:             Option<Declarator>,
}

struct TokenCursor {
    preprocessor: Preprocessor,
    current:      Option<Token>,
    reached_eof:  bool,
}

impl TokenCursor {
    fn new(preprocessor: Preprocessor) -> Self {
        Self {
            preprocessor,
            current: None,
            reached_eof: false,
        }
    }

    fn current(&mut self, context: &mut Context) -> Option<Token> {
        if self.current.is_none() && !self.reached_eof {
            self.current = self.preprocessor.next_item(context);
            self.reached_eof = self.current.is_none();
        }
        self.current
    }

    fn consume(&mut self) {
        debug_assert!(self.current.is_some(), "cannot consume parser EOF");
        self.current = None;
    }
}

#[derive(Default)]
struct SyntaxStore {
    #[expect(dead_code, reason = "Owned by the future type-name frame.")]
    type_names:                 Vec<TypeName>,
    declarations:               Vec<Declaration>,
    init_declarators:           Vec<InitDeclarator>,
    #[expect(dead_code, reason = "Owned by the future expression frame.")]
    expressions:                Vec<Expression>,
    #[expect(dead_code, reason = "Owned by the future expression frame.")]
    expression_indices:         Vec<ExpressionIndex>,
    #[expect(dead_code, reason = "Owned by the future statement frame.")]
    statements:                 Vec<Statement>,
    type_qualifiers:            Vec<TypeQualifiers>,
    direct_declarators:         Vec<DirectDeclarator>,
    identifiers:                Vec<Identifier>,
    parameter_declarations:     Vec<ParameterDeclaration>,
    struct_or_union_specifiers: Vec<StructOrUnionSpecifier>,
    struct_declarations:        Vec<StructDeclaration>,
    struct_declarators:         Vec<StructDeclarator>,
    enum_specifiers:            Vec<EnumSpecifier>,
    enumerators:                Vec<Enumerator>,
    declaration_sources:        Vec<SourceVectors>,
    declarator_sources:         Vec<SourceVectors>,
    parameter_sources:          Vec<SourceVectors>,
    struct_specifier_sources:   Vec<SourceVectors>,
    enum_specifier_sources:     Vec<SourceVectors>,
    struct_declaration_sources: Vec<SourceVectors>,
    struct_declarator_sources:  Vec<SourceVectors>,
    enumerator_sources:         Vec<SourceVectors>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum NameClass {
    Typedef,
    Ordinary,
}

#[derive(Default)]
struct ScopeStack {
    file_scope: HashMap<StringCacheId, NameClass>,
}

impl ScopeStack {
    fn is_typedef(&self, name: StringCacheId) -> bool {
        self.file_scope.get(&name) == Some(&NameClass::Typedef)
    }

    fn publish(&mut self, name: StringCacheId, class: NameClass) {
        _ = self.file_scope.insert(name, class);
    }
}

#[derive(Debug)]
enum ParseAction {
    Consume,
    Push(ParseFrame),
    Reduce(ParseValue),
    Reprocess,
    Recover(SynchronizationSet),
}

#[derive(Debug, Clone, Copy)]
enum ParseValue {
    DeclarationSpecifiers(DeclarationSpecifiers),
    Declarator(Option<Declarator>),
    ParameterList(ParameterListResult),
    StructOrUnionSpecifier(StructOrUnionSpecifierIndex),
    EnumSpecifier(EnumSpecifierIndex),
    Declaration(DeclarationIndex),
    ExternalDeclaration(ExternalDeclaration),
    FutureChild(FutureChildResult),
}

#[derive(Debug, Clone, Copy)]
struct ParameterListResult {
    direct_declarator: DirectDeclarator,
    source_vectors:    SourceVectors,
}

#[derive(Debug, Clone, Copy)]
struct FutureChildResult {
    kind:           FutureChildKind,
    source_vectors: SourceVectors,
}

#[derive(Debug)]
enum ParseFrame {
    ExternalDeclaration(ExternalDeclarationFrame),
    Declaration(DeclarationFrame),
    DeclarationSpecifiers(DeclarationSpecifiersFrame),
    Declarator(DeclaratorFrame),
    ParameterList(ParameterListFrame),
    StructOrUnionSpecifier(StructOrUnionSpecifierFrame),
    EnumSpecifier(EnumSpecifierFrame),
    FutureChild(FutureChildFrame),
}

struct FrameStep {
    frame_name: &'static str,
    action:     ParseAction,
}

#[derive(Debug, Clone, Copy)]
struct SynchronizationSet {
    kind:   SynchronizationKind,
    target: RecoveryTarget,
}

#[derive(Debug, Clone, Copy)]
enum SynchronizationKind {
    Declaration,
    Initializer,
    ArrayBound,
    Parameter,
    StructMember,
    EnumeratorValue,
    FunctionBody,
}

#[derive(Debug, Clone, Copy)]
enum RecoveryTarget {
    CurrentFrame,
}

#[derive(Default)]
struct RecoveryState {
    active:         bool,
    skipped_tokens: usize,
    last:           Option<SynchronizationSet>,
    source_vectors: Option<SourceVectors>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FutureChildKind {
    ArrayBoundExpression,
    BitFieldWidthExpression,
    EnumeratorValueExpression,
    Initializer,
    FunctionBody,
}

impl FutureChildKind {
    fn frame_name(self) -> &'static str {
        match self {
            | Self::ArrayBoundExpression => "array-bound-expression",
            | Self::BitFieldWidthExpression => "bit-field-width-expression",
            | Self::EnumeratorValueExpression => "enumerator-value-expression",
            | Self::Initializer => "initializer",
            | Self::FunctionBody => "statement",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FutureChildFrame {
    kind:  FutureChildKind,
    phase: FutureChildPhase,
}

#[derive(Debug, Clone, Copy)]
enum FutureChildPhase {
    Start,
    Recovered,
}

impl FutureChildFrame {
    fn new(kind: FutureChildKind) -> Self {
        Self {
            kind,
            phase: FutureChildPhase::Start,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ExternalDeclarationFrame {
    phase:                ExternalDeclarationPhase,
    starting_error_count: usize,
}

#[derive(Debug, Clone, Copy)]
enum ExternalDeclarationPhase {
    Start,
    AwaitDeclaration,
}

impl ExternalDeclarationFrame {
    fn new(starting_error_count: usize) -> Self {
        Self {
            phase: ExternalDeclarationPhase::Start,
            starting_error_count,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DeclarationFrame {
    phase:                  DeclarationPhase,
    declaration_specifiers: Option<DeclarationSpecifiers>,
    init_declarator_start:  u32,
    source_vectors:         Option<SourceVectors>,
    last_init_index:        Option<u32>,
    initializer_source:     Option<SourceVectors>,
}

#[derive(Debug, Clone, Copy)]
enum DeclarationPhase {
    Start,
    AwaitSpecifiers,
    AwaitDeclarator,
    AfterDeclarator,
    PushInitializer,
    AwaitInitializer,
    AwaitFunctionBody,
    BeforeNextDeclarator,
    Finish,
}

impl DeclarationFrame {
    fn new(init_declarator_start: u32) -> Self {
        Self {
            phase: DeclarationPhase::Start,
            declaration_specifiers: None,
            init_declarator_start,
            source_vectors: None,
            last_init_index: None,
            initializer_source: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpecifierMode {
    Declaration,
    SpecifierQualifier,
}

#[derive(Debug, Clone, Copy)]
struct DeclarationSpecifiersFrame {
    phase:          DeclarationSpecifiersPhase,
    mode:           SpecifierMode,
    specifiers:     DeclarationSpecifiers,
    consumed:       bool,
    storage_seen:   bool,
    source_vectors: Option<SourceVectors>,
}

#[derive(Debug, Clone, Copy)]
enum DeclarationSpecifiersPhase {
    Collect,
    AwaitStructOrUnion,
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
            source_vectors: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclaratorMode {
    Named,
    Abstract,
    MaybeAbstract,
}

#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "The booleans retain independent C array/declarator grammar facts."
)]
struct DeclaratorFrame {
    phase: DeclaratorPhase,
    mode: DeclaratorMode,
    pointer_qualifiers: Vec<TypeQualifiers>,
    direct_declarators: Vec<DirectDeclarator>,
    current_qualifiers: TypeQualifiers,
    has_pointer_level: bool,
    has_direct_declarator: bool,
    array_qualifiers: TypeQualifiers,
    array_qualifiers_before_static: bool,
    array_is_static: bool,
    array_is_pointer: bool,
    source_vectors: Option<SourceVectors>,
}

#[derive(Debug, Clone, Copy)]
enum DeclaratorPhase {
    PointerOrBase,
    PointerQualifiers,
    Base,
    PushNested,
    ClassifyAbstractParenthesis,
    AwaitNested,
    ExpectNestedClose(Declarator),
    Suffix,
    Array,
    ArrayExpectClose,
    AwaitArrayBound,
    FunctionStart,
    AwaitParameterList,
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

#[derive(Debug)]
struct ParameterListFrame {
    phase:              ParameterListPhase,
    allow_k_and_r:      bool,
    parameters:         Vec<ParameterDeclaration>,
    parameter_sources:  Vec<SourceVectors>,
    identifiers:        Vec<Identifier>,
    pending_specifiers: Option<DeclarationSpecifiers>,
    pending_source:     Option<SourceVectors>,
    is_variadic:        bool,
    source_vectors:     Option<SourceVectors>,
}

#[derive(Debug, Clone, Copy)]
enum ParameterListPhase {
    Start,
    KAndRIdentifier,
    KAndRSeparator,
    PrototypeParameter,
    AwaitSpecifiers,
    AwaitDeclarator,
    PrototypeSeparator,
    AfterComma,
    ExpectCloseAfterEllipsis,
    FinishKAndR,
    FinishPrototype,
}

impl ParameterListFrame {
    fn new(allow_k_and_r: bool) -> Self {
        Self {
            phase: ParameterListPhase::Start,
            allow_k_and_r,
            parameters: Vec::new(),
            parameter_sources: Vec::new(),
            identifiers: Vec::new(),
            pending_specifiers: None,
            pending_source: None,
            is_variadic: false,
            source_vectors: None,
        }
    }
}

#[derive(Debug)]
struct StructOrUnionSpecifierFrame {
    phase: StructOrUnionPhase,
    kind: Option<StructOrUnion>,
    identifier: Option<Identifier>,
    declarations: Vec<StructDeclaration>,
    member_declarators: Vec<StructDeclarator>,
    declaration_sources: Vec<SourceVectors>,
    member_declarator_sources: Vec<SourceVectors>,
    member_specifiers: Option<DeclarationSpecifiers>,
    member_declarator: Option<Declarator>,
    body_started: bool,
    source_vectors: Option<SourceVectors>,
    member_source: Option<SourceVectors>,
    current_member_declarator_source: Option<SourceVectors>,
}

#[derive(Debug, Clone, Copy)]
enum StructOrUnionPhase {
    Start,
    NameOrBody,
    AfterName,
    MemberStart,
    AwaitMemberSpecifiers,
    PushMemberDeclarator,
    AwaitMemberDeclarator,
    AfterMemberDeclarator,
    PushBitFieldWidth,
    AwaitBitFieldWidth,
    AfterStructDeclarator,
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
            declaration_sources: Vec::new(),
            member_declarator_sources: Vec::new(),
            member_specifiers: None,
            member_declarator: None,
            body_started: false,
            source_vectors: None,
            member_source: None,
            current_member_declarator_source: None,
        }
    }
}

#[derive(Debug)]
struct EnumSpecifierFrame {
    phase: EnumPhase,
    name: Option<Identifier>,
    enumerators: Vec<Enumerator>,
    enumerator_sources: Vec<SourceVectors>,
    current_enumerator: Option<Identifier>,
    body_started: bool,
    source_vectors: Option<SourceVectors>,
    current_enumerator_source: Option<SourceVectors>,
}

#[derive(Debug, Clone, Copy)]
enum EnumPhase {
    Start,
    NameOrBody,
    AfterName,
    EnumeratorOrClose,
    AfterEnumeratorName,
    PushEnumeratorValue,
    AwaitEnumeratorValue,
    AfterEnumerator,
    FinishBody,
}

impl EnumSpecifierFrame {
    fn new() -> Self {
        Self {
            phase: EnumPhase::Start,
            name: None,
            enumerators: Vec::new(),
            enumerator_sources: Vec::new(),
            current_enumerator: None,
            body_started: false,
            source_vectors: None,
            current_enumerator_source: None,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
struct FrameTraceEvent {
    frame:  &'static str,
    action: &'static str,
    token:  Option<TokenType>,
    depth:  usize,
}

impl Parser {
    pub(crate) fn new(preprocessor: Preprocessor) -> Self {
        Self {
            cursor:              TokenCursor::new(preprocessor),
            frames:              Vec::new(),
            returned:            None,
            syntax:              SyntaxStore::default(),
            scopes:              ScopeStack::default(),
            recovery:            RecoveryState::default(),
            invalid_event_count: 0,
            #[cfg(test)]
            trace:               Vec::new(),
        }
    }

    fn drive(&mut self, context: &mut Context) -> Option<ExternalDeclaration> {
        loop {
            if matches!(self.returned, Some(ParseValue::ExternalDeclaration(_))) {
                let Some(ParseValue::ExternalDeclaration(external)) = self.returned.take() else {
                    unreachable!("the returned value was just checked")
                };
                return Some(external);
            }
            debug_assert!(
                self.returned.is_none() || !self.frames.is_empty(),
                "a child value must have a parent frame"
            );

            if self.frames.is_empty() {
                _ = self.cursor.current(context)?;
                self.frames.push(ParseFrame::ExternalDeclaration(
                    ExternalDeclarationFrame::new(self.invalid_event_count),
                ));
            }

            let token = self.cursor.current(context);
            let mut frame = self.frames.pop().expect("parser frame stack is nonempty");
            let returned = self.returned.take();
            let FrameStep { frame_name, action } = frame.step(self, context, token, returned);

            #[cfg(test)]
            self.trace.push(FrameTraceEvent {
                frame:  frame_name,
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
                        self.report_eof(
                            context,
                            frame_name,
                            "a token before completing the active frame",
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
                        matches!(set.target, RecoveryTarget::CurrentFrame),
                        "every recovery set must identify a legal unwind target"
                    );
                    self.recovery.source_vectors = self.recover(context, set);
                    self.frames.push(frame);
                },
            }
        }
    }

    fn recover(&mut self, context: &mut Context, set: SynchronizationSet) -> Option<SourceVectors> {
        self.recovery.active = true;
        self.recovery.last = Some(set);
        let mut parentheses = 0usize;
        let mut brackets = 0usize;
        let mut braces = 0usize;
        let mut source_vectors = None;

        while let Some(token) = self.cursor.current(context) {
            let at_top_level = parentheses == 0 && brackets == 0 && braces == 0;
            let at_owning_array_bracket = matches!(set.kind, SynchronizationKind::ArrayBound)
                && brackets == 0
                && token.kind == TokenType::Operator(OperatorTokenType::ClosingSquareBracket);
            if at_owning_array_bracket || at_top_level && set.kind.stops_before(token.kind) {
                break;
            }

            match token.kind {
                | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => {
                    parentheses += 1;
                },
                | TokenType::Operator(OperatorTokenType::ClosingParenthesis) if parentheses > 0 => {
                    parentheses -= 1;
                },
                | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                    brackets += 1;
                },
                | TokenType::Operator(OperatorTokenType::ClosingSquareBracket) if brackets > 0 => {
                    brackets -= 1;
                },
                | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                    braces += 1;
                },
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) if braces > 0 => {
                    braces -= 1;
                    if matches!(set.kind, SynchronizationKind::FunctionBody) && braces == 0 {
                        self.merge_source(context, &mut source_vectors, token);
                        self.cursor.consume();
                        self.recovery.skipped_tokens += 1;
                        break;
                    }
                },
                | _ => {},
            }

            self.merge_source(context, &mut source_vectors, token);
            self.cursor.consume();
            self.recovery.skipped_tokens += 1;
        }
        self.recovery.active = false;
        source_vectors
    }

    fn report(&mut self, context: &mut Context, error_type: ParserErrorType, token: Option<Token>) {
        if error_type.severity() == ErrorSeverity::Error {
            self.invalid_event_count += 1;
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

    fn report_unexpected(
        &mut self,
        context: &mut Context,
        frame: &'static str,
        expected: &'static str,
        token: Option<Token>,
    ) {
        if let Some(token) = token {
            self.report(
                context,
                ParserErrorType::UnexpectedToken {
                    frame,
                    expected,
                    found: token.kind,
                },
                Some(token),
            );
        } else {
            self.report_eof(context, frame, expected);
        }
    }

    fn report_eof(&mut self, context: &mut Context, frame: &'static str, expected: &'static str) {
        self.report(
            context,
            ParserErrorType::UnexpectedEndOfFrame { frame, expected },
            None,
        );
    }

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
    fn stops_before(self, token: TokenType) -> bool {
        match self {
            | Self::Declaration => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::Initializer => matches!(
                token,
                TokenType::Operator(OperatorTokenType::Comma | OperatorTokenType::Semicolon)
            ),
            | Self::ArrayBound =>
                token == TokenType::Operator(OperatorTokenType::ClosingSquareBracket),
            | Self::Parameter => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma | OperatorTokenType::ClosingParenthesis
                )
            ),
            | Self::StructMember => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::EnumeratorValue => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::FunctionBody => false,
        }
    }
}

fn is_operator(token: Option<Token>, operator: OperatorTokenType) -> bool {
    token.is_some_and(|token| token.kind == TokenType::Operator(operator))
}

impl ParseFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> FrameStep {
        match self {
            | Self::ExternalDeclaration(frame) => FrameStep {
                frame_name: "external-declaration",
                action:     frame.step(parser, context, token, returned),
            },
            | Self::Declaration(frame) => FrameStep {
                frame_name: "declaration",
                action:     frame.step(parser, context, token, returned),
            },
            | Self::DeclarationSpecifiers(frame) => FrameStep {
                frame_name: "declaration-specifiers",
                action:     frame.step(parser, context, token, returned),
            },
            | Self::Declarator(frame) => FrameStep {
                frame_name: "declarator",
                action:     frame.step(parser, context, token, returned),
            },
            | Self::ParameterList(frame) => FrameStep {
                frame_name: "parameter-list",
                action:     frame.step(parser, context, token, returned),
            },
            | Self::StructOrUnionSpecifier(frame) => FrameStep {
                frame_name: "struct-or-union-specifier",
                action:     frame.step(parser, context, token, returned),
            },
            | Self::EnumSpecifier(frame) => FrameStep {
                frame_name: "enum-specifier",
                action:     frame.step(parser, context, token, returned),
            },
            | Self::FutureChild(frame) => FrameStep {
                frame_name: frame.kind.frame_name(),
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
                parser.report(
                    context,
                    ParserErrorType::FutureChildNotImplemented(self.kind),
                    token,
                );
                self.phase = FutureChildPhase::Recovered;
                let kind = match self.kind {
                    | FutureChildKind::ArrayBoundExpression => SynchronizationKind::ArrayBound,
                    | FutureChildKind::BitFieldWidthExpression => SynchronizationKind::StructMember,
                    | FutureChildKind::EnumeratorValueExpression =>
                        SynchronizationKind::EnumeratorValue,
                    | FutureChildKind::Initializer => SynchronizationKind::Initializer,
                    | FutureChildKind::FunctionBody => SynchronizationKind::FunctionBody,
                };
                ParseAction::Recover(SynchronizationSet {
                    kind,
                    target: RecoveryTarget::CurrentFrame,
                })
            },
            | FutureChildPhase::Recovered =>
                ParseAction::Reduce(ParseValue::FutureChild(FutureChildResult {
                    kind:           self.kind,
                    source_vectors: parser.recovery.source_vectors.take().unwrap_or_default(),
                })),
        }
    }
}

impl ExternalDeclarationFrame {
    fn step(
        &mut self,
        parser: &mut Parser,
        _context: &mut Context,
        _token: Option<Token>,
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
                )))
            },
            | ExternalDeclarationPhase::AwaitDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("declaration frame returned an unexpected value: {returned:?}");
                };
                if parser.invalid_event_count > self.starting_error_count {
                    let source_vectors = parser
                        .syntax
                        .declaration_sources
                        .get(declaration.0 as usize)
                        .copied()
                        .unwrap_or_default();
                    return ParseAction::Reduce(ParseValue::ExternalDeclaration(
                        ExternalDeclaration::Error(source_vectors),
                    ));
                }
                ParseAction::Reduce(ParseValue::ExternalDeclaration(
                    ExternalDeclaration::Declaration(declaration),
                ))
            },
        }
    }
}

impl DeclarationFrame {
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
                if is_operator(token, OperatorTokenType::Semicolon) {
                    if specifiers.storage_class == StorageClass::Typedef {
                        parser.report_unexpected(
                            context,
                            "declaration",
                            "a typedef declarator",
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
                    parser.report_unexpected(context, "declaration", "a declarator", token);
                    self.phase = DeclarationPhase::AfterDeclarator;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::Declaration,
                        target: RecoveryTarget::CurrentFrame,
                    });
                };

                let source_vectors = parser
                    .syntax
                    .declarator_sources
                    .last()
                    .copied()
                    .unwrap_or_default();
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
            | DeclarationPhase::AfterDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
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
                } else if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    self.phase = DeclarationPhase::AwaitFunctionBody;
                    ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::new(
                        FutureChildKind::FunctionBody,
                    )))
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    parser.report_unexpected(
                        context,
                        "declaration",
                        "`,`, `=`, or `;` after a declarator",
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report_eof(
                        context,
                        "declaration",
                        "`,`, `=`, or `;` after a declarator",
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    parser.report_unexpected(
                        context,
                        "declaration",
                        "`,`, `=`, or `;` after a declarator",
                        token,
                    );
                    self.phase = DeclarationPhase::AfterDeclarator;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::Declaration,
                        target: RecoveryTarget::CurrentFrame,
                    })
                }
            },
            | DeclarationPhase::PushInitializer => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = DeclarationPhase::AwaitInitializer;
                ParseAction::Push(ParseFrame::FutureChild(FutureChildFrame::new(
                    FutureChildKind::Initializer,
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
            | DeclarationPhase::AwaitFunctionBody => {
                let Some(ParseValue::FutureChild(FutureChildResult {
                    kind: FutureChildKind::FunctionBody,
                    source_vectors,
                })) = returned
                else {
                    panic!("statement seam returned an unexpected value: {returned:?}");
                };
                self.source_vectors =
                    Some(self.source_vectors.map_or(source_vectors, |existing| {
                        context.merge_vectors(existing, source_vectors)
                    }));
                self.phase = DeclarationPhase::Finish;
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
                });
                parser.syntax.declaration_sources.push(source_vectors);
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
                self.specifiers
                    .type_specifiers
                    .make_struct_or_union(parser, context, index);
                if let Some(source_vectors) = parser
                    .syntax
                    .struct_specifier_sources
                    .get(index.0 as usize)
                    .copied()
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
                let Some(ParseValue::EnumSpecifier(index)) = returned else {
                    panic!("enum specifier returned an unexpected value: {returned:?}");
                };
                self.specifiers
                    .type_specifiers
                    .make_enum(parser, context, index);
                if let Some(source_vectors) = parser
                    .syntax
                    .enum_specifier_sources
                    .get(index.0 as usize)
                    .copied()
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
            | DeclarationSpecifiersPhase::Collect => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
            },
        }

        let Some(token) = token else {
            if !self.consumed {
                parser.report_eof(context, "declaration-specifiers", "a declaration specifier");
            }
            if self.specifiers.type_specifiers == TypeSpecifiers::Empty {
                parser.report_eof(
                    context,
                    "declaration-specifiers",
                    "at least one type specifier",
                );
            }
            self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
            return ParseAction::Reduce(ParseValue::DeclarationSpecifiers(self.specifiers));
        };

        if matches!(
            token.kind,
            TokenType::Keyword(KeywordTokenType::Struct | KeywordTokenType::Union)
        ) {
            self.phase = DeclarationSpecifiersPhase::AwaitStructOrUnion;
            return ParseAction::Push(ParseFrame::StructOrUnionSpecifier(
                StructOrUnionSpecifierFrame::new(),
            ));
        }
        if token.kind == TokenType::Keyword(KeywordTokenType::Enum) {
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
            && is_primitive_type_keyword(keyword)
        {
            self.apply_type_specifier(parser, context, token, keyword);
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
            && self.specifiers.type_specifiers == TypeSpecifiers::Empty
            && parser.scopes.is_typedef(token.contents)
        {
            self.specifiers.type_specifiers.make_typedef_name(
                parser,
                context,
                Identifier::new(token.contents),
            );
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

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
        keyword: KeywordTokenType,
    ) {
        let type_specifiers = &mut self.specifiers.type_specifiers;
        let duplicate = match keyword {
            | KeywordTokenType::Signed => type_specifiers.is_signed(),
            | KeywordTokenType::Unsigned => type_specifiers.is_unsigned(),
            | KeywordTokenType::Int => type_specifiers.is_int(),
            | KeywordTokenType::Short => type_specifiers.is_short(),
            | KeywordTokenType::Long => false,
            | KeywordTokenType::Char => type_specifiers.is_char(),
            | KeywordTokenType::Float => type_specifiers.is_float(),
            | KeywordTokenType::Double => type_specifiers.is_double(),
            | KeywordTokenType::Void => type_specifiers.is_void(),
            | KeywordTokenType::Bool => type_specifiers.is_bool(),
            | KeywordTokenType::Complex => type_specifiers.is_complex(),
            | KeywordTokenType::Imaginary => type_specifiers.is_imaginary(),
            | _ => unreachable!("only primitive type keywords reach this method"),
        };
        if duplicate {
            parser.report(
                context,
                ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                Some(token),
            );
            return;
        }

        match keyword {
            | KeywordTokenType::Signed => type_specifiers.make_signed(parser, context),
            | KeywordTokenType::Unsigned => type_specifiers.make_unsigned(parser, context),
            | KeywordTokenType::Int => type_specifiers.make_int(parser, context),
            | KeywordTokenType::Short => type_specifiers.make_short(parser, context),
            | KeywordTokenType::Long if type_specifiers.is_long_double() => {
                parser.report(
                    context,
                    ParserErrorType::LongLongDoubleSpecified,
                    Some(token),
                );
            },
            | KeywordTokenType::Long
                if type_specifiers.is_long() && type_specifiers.is_long_long() =>
            {
                parser.report(context, ParserErrorType::LongSpecifiedThrice, Some(token));
            },
            | KeywordTokenType::Long => type_specifiers.make_long(parser, context),
            | KeywordTokenType::Char => type_specifiers.make_char(parser, context),
            | KeywordTokenType::Float => type_specifiers.make_float(parser, context),
            | KeywordTokenType::Double if type_specifiers.is_long_long() => {
                parser.report(
                    context,
                    ParserErrorType::LongLongDoubleSpecified,
                    Some(token),
                );
            },
            | KeywordTokenType::Double => type_specifiers.make_double(parser, context),
            | KeywordTokenType::Void => type_specifiers.make_void(parser, context),
            | KeywordTokenType::Bool => type_specifiers.make_bool(parser, context),
            | KeywordTokenType::Complex => type_specifiers.make_complex(parser, context),
            | KeywordTokenType::Imaginary => type_specifiers.make_imaginary(parser, context),
            | _ => unreachable!("only primitive type keywords reach this method"),
        }
    }
}

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

fn type_qualifier(token: TokenType) -> Option<TypeQualifiers> {
    match token {
        | TokenType::Keyword(KeywordTokenType::Const) => Some(TypeQualifiers::CONST),
        | TokenType::Keyword(KeywordTokenType::Volatile) => Some(TypeQualifiers::VOLATILE),
        | TokenType::Keyword(KeywordTokenType::Restrict) => Some(TypeQualifiers::RESTRICT),
        | _ => None,
    }
}

fn is_primitive_type_keyword(keyword: KeywordTokenType) -> bool {
    matches!(
        keyword,
        KeywordTokenType::Signed
            | KeywordTokenType::Unsigned
            | KeywordTokenType::Int
            | KeywordTokenType::Short
            | KeywordTokenType::Long
            | KeywordTokenType::Char
            | KeywordTokenType::Float
            | KeywordTokenType::Double
            | KeywordTokenType::Void
            | KeywordTokenType::Bool
            | KeywordTokenType::Complex
            | KeywordTokenType::Imaginary
    )
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
                    parser.report_unexpected(context, "declarator", "an identifier or `(`", token);
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
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.direct_declarators.push(DirectDeclarator::Function {
                        parameter_list: VectorSlice::empty(),
                        is_variadic:    false,
                    });
                    self.has_direct_declarator = true;
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
                    parser.report_unexpected(
                        context,
                        "declarator",
                        "a declarator after `(`",
                        token,
                    );
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Reprocess;
                };
                if let Some(source_vectors) = parser.syntax.declarator_sources.last().copied() {
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }));
                }
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
                    parser.report_unexpected(context, "parenthesized-declarator", "`)`", token);
                    ParseAction::Reprocess
                }
            },
            | DeclaratorPhase::Suffix => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
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
                    parser.report_eof(context, "array-declarator", "`]`");
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
                if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                    let token = token.expect("closing-square-bracket token exists");
                    self.push_array();
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else {
                    parser.report_unexpected(context, "array-declarator", "`]` after `*`", token);
                    self.phase = DeclaratorPhase::AwaitArrayBound;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::ArrayBound,
                        target: RecoveryTarget::CurrentFrame,
                    })
                }
            },
            | DeclaratorPhase::AwaitArrayBound => {
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
                } else if token.is_none() {
                    parser.report_eof(context, "array-declarator", "`]`");
                    self.push_array();
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    parser.report_unexpected(context, "array-declarator", "`]`", token);
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::ArrayBound,
                        target: RecoveryTarget::CurrentFrame,
                    })
                }
            },
            | DeclaratorPhase::FunctionStart => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let direct = if self.mode == DeclaratorMode::Named {
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
                    parser.report_eof(context, "function-declarator", "`)`");
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    self.phase = DeclaratorPhase::AwaitParameterList;
                    ParseAction::Push(ParseFrame::ParameterList(ParameterListFrame::new(
                        self.mode == DeclaratorMode::Named,
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
                if !self.has_pointer_level && !self.has_direct_declarator {
                    return ParseAction::Reduce(ParseValue::Declarator(None));
                }
                if self.mode == DeclaratorMode::Named && !self.has_direct_declarator {
                    parser.report_unexpected(
                        context,
                        "declarator",
                        "an identifier or parenthesized declarator",
                        token,
                    );
                    return ParseAction::Reduce(ParseValue::Declarator(None));
                }

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
                    pointer_declarator: PointerDeclarator {
                        type_qualifiers_list: VectorSlice::new(
                            pointer_start,
                            parser.syntax.type_qualifiers.len().to_u32(),
                        ),
                    },
                    kind:               VectorSlice::new(
                        direct_start,
                        parser.syntax.direct_declarators.len().to_u32(),
                    ),
                };
                parser
                    .syntax
                    .declarator_sources
                    .push(self.source_vectors.unwrap_or_default());
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
                    parser.report_eof(context, "K&R parameter-list", "an identifier and `)`");
                    self.phase = ParameterListPhase::FinishKAndR;
                    return ParseAction::Reprocess;
                };
                if token.kind != TokenType::Identifier || parser.scopes.is_typedef(token.contents) {
                    parser.report_unexpected(
                        context,
                        "K&R parameter-list",
                        "a non-typedef identifier",
                        Some(token),
                    );
                    self.phase = ParameterListPhase::KAndRSeparator;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::Parameter,
                        target: RecoveryTarget::CurrentFrame,
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
                } else if token.is_none() {
                    parser.report_eof(context, "K&R parameter-list", "`)`");
                    self.phase = ParameterListPhase::FinishKAndR;
                    ParseAction::Reprocess
                } else {
                    parser.report_unexpected(context, "K&R parameter-list", "`,` or `)`", token);
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::Parameter,
                        target: RecoveryTarget::CurrentFrame,
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
                if is_operator(token, OperatorTokenType::Comma)
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.parameters.push(ParameterDeclaration {
                        declaration_specifiers: specifiers,
                        declarator:             None,
                    });
                    self.parameter_sources.push(specifiers.source_vectors);
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
                let declarator_source =
                    declarator.and_then(|_| parser.syntax.declarator_sources.last().copied());
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
                });
                self.parameter_sources.push(parameter_source);
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
                } else if token.is_none() {
                    parser.report_eof(context, "prototype parameter-list", "`,` or `)`");
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else {
                    parser.report_unexpected(
                        context,
                        "prototype parameter-list",
                        "`,` or `)`",
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::Parameter,
                        target: RecoveryTarget::CurrentFrame,
                    })
                }
            },
            | ParameterListPhase::AfterComma => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::Ellipsis) {
                    self.is_variadic = true;
                    if let Some(token) = token {
                        parser.merge_source(context, &mut self.source_vectors, token);
                    }
                    self.phase = ParameterListPhase::ExpectCloseAfterEllipsis;
                    ParseAction::Consume
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
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report_eof(context, "variadic parameter-list", "`)`");
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else {
                    parser.report_unexpected(
                        context,
                        "variadic parameter-list",
                        "`)` after `...`",
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::Parameter,
                        target: RecoveryTarget::CurrentFrame,
                    })
                }
            },
            | ParameterListPhase::FinishKAndR => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
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
                let start = parser.syntax.parameter_declarations.len().to_u32();
                parser
                    .syntax
                    .parameter_declarations
                    .append(&mut self.parameters);
                parser
                    .syntax
                    .parameter_sources
                    .append(&mut self.parameter_sources);
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
                    parser.report_eof(context, "struct-or-union-specifier", "`struct` or `union`");
                    return self.finish(parser);
                };
                self.kind = match token.kind {
                    | TokenType::Keyword(KeywordTokenType::Struct) => Some(StructOrUnion::Struct),
                    | TokenType::Keyword(KeywordTokenType::Union) => Some(StructOrUnion::Union),
                    | _ => {
                        parser.report_unexpected(
                            context,
                            "struct-or-union-specifier",
                            "`struct` or `union`",
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
                    parser.report_unexpected(
                        context,
                        "struct-or-union-specifier",
                        "a tag name or `{`",
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
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report_eof(context, "struct-declaration-list", "`}`");
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
                self.phase = StructOrUnionPhase::AwaitMemberDeclarator;
                ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                    DeclaratorMode::Named,
                )))
            },
            | StructOrUnionPhase::AwaitMemberDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("member declarator returned an unexpected value: {returned:?}");
                };
                self.member_declarator = declarator;
                if let Some(source_vectors) =
                    declarator.and_then(|_| parser.syntax.declarator_sources.last().copied())
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
                if is_operator(token, OperatorTokenType::Colon) {
                    let token = token.expect("colon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    parser.merge_source(context, &mut self.current_member_declarator_source, token);
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else {
                    self.member_declarators.push(StructDeclarator {
                        declarator:     self.member_declarator.take(),
                        bitfield_width: None,
                    });
                    self.member_declarator_sources.push(
                        self.current_member_declarator_source
                            .take()
                            .unwrap_or_default(),
                    );
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
                self.member_declarators.push(StructDeclarator {
                    declarator:     self.member_declarator.take(),
                    bitfield_width: None,
                });
                self.member_declarator_sources.push(
                    self.current_member_declarator_source
                        .take()
                        .unwrap_or_default(),
                );
                self.phase = StructOrUnionPhase::AfterStructDeclarator;
                ParseAction::Reprocess
            },
            | StructOrUnionPhase::AfterStructDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
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
                    parser.report_unexpected(
                        context,
                        "struct-declarator-list",
                        "`;` before `}`",
                        token,
                    );
                    self.finish_member(parser, context);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report_eof(context, "struct-declarator-list", "`,` or `;`");
                    self.finish_member(parser, context);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report_unexpected(
                        context,
                        "struct-declarator-list",
                        "`,` or `;`",
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::StructMember,
                        target: RecoveryTarget::CurrentFrame,
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
        let start = parser.syntax.struct_declarators.len().to_u32();
        parser
            .syntax
            .struct_declarators
            .append(&mut self.member_declarators);
        parser
            .syntax
            .struct_declarator_sources
            .append(&mut self.member_declarator_sources);
        let specifiers = self
            .member_specifiers
            .take()
            .expect("a struct member has specifiers");
        self.declarations.push(StructDeclaration {
            type_qualifiers:        specifiers.type_qualifiers,
            type_specifiers:        specifiers.type_specifiers,
            struct_declarator_list: VectorSlice::new(
                start,
                parser.syntax.struct_declarators.len().to_u32(),
            ),
        });
        let source_vectors = self.member_source.take().unwrap_or_default();
        self.declaration_sources.push(source_vectors);
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
            parser
                .syntax
                .struct_declaration_sources
                .append(&mut self.declaration_sources);
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
            });
        parser
            .syntax
            .struct_specifier_sources
            .push(self.source_vectors.unwrap_or_default());
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
                    parser.report_eof(context, "enum-specifier", "`enum`");
                    return self.finish(parser);
                };
                if token.kind != TokenType::Keyword(KeywordTokenType::Enum) {
                    parser.report_unexpected(context, "enum-specifier", "`enum`", Some(token));
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
                    parser.report_unexpected(context, "enum-specifier", "a tag name or `{`", token);
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
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
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
                } else if token.is_none() {
                    parser.report_eof(context, "enumerator-list", "an enumerator or `}`");
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report_unexpected(
                        context,
                        "enumerator-list",
                        "an enumeration constant or `}`",
                        token,
                    );
                    self.phase = EnumPhase::AfterEnumerator;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::EnumeratorValue,
                        target: RecoveryTarget::CurrentFrame,
                    })
                }
            },
            | EnumPhase::AfterEnumeratorName => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
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
                } else if token.is_none() {
                    parser.report_eof(context, "enumerator-list", "`,` or `}`");
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report_unexpected(context, "enumerator-list", "`,` or `}`", token);
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::EnumeratorValue,
                        target: RecoveryTarget::CurrentFrame,
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
            });
            self.enumerator_sources
                .push(self.current_enumerator_source.take().unwrap_or_default());
            parser.scopes.publish(name.name, NameClass::Ordinary);
        }
    }

    fn finish(&mut self, parser: &mut Parser) -> ParseAction {
        let enumeration_list = self.body_started.then(|| {
            let start = parser.syntax.enumerators.len().to_u32();
            parser.syntax.enumerators.append(&mut self.enumerators);
            parser
                .syntax
                .enumerator_sources
                .append(&mut self.enumerator_sources);
            VectorSlice::new(start, parser.syntax.enumerators.len().to_u32())
        });
        let index = parser.syntax.enum_specifiers.len().to_u32();
        parser.syntax.enum_specifiers.push(EnumSpecifier {
            name: self.name,
            enumeration_list,
        });
        parser
            .syntax
            .enum_specifier_sources
            .push(self.source_vectors.unwrap_or_default());
        ParseAction::Reduce(ParseValue::EnumSpecifier(EnumSpecifierIndex(index)))
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ExternalDeclaration {
    Declaration(DeclarationIndex),
    Error(SourceVectors),
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct DeclarationIndex(u32);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct ExpressionIndex(u32);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct ConstantExpressionIndex(u32);

impl From<ConstantExpressionIndex> for ExpressionIndex {
    fn from(index: ConstantExpressionIndex) -> Self {
        Self(index.0)
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct StatementIndex(u32);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct StructOrUnionSpecifierIndex(u32);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct EnumSpecifierIndex(u32);

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Statement {
    pub(crate) kind: StatementType,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[expect(
    dead_code,
    reason = "The future statement frame will construct these syntax variants."
)]
pub(crate) enum StatementType {
    Compound(VectorSlice<Statement>),
    Expression(ExpressionIndex),
    If {
        condition_expression: ExpressionIndex,
        then_statement:       StatementIndex,
        else_statement:       Option<StatementIndex>,
    },
    While {
        condition_expression: ExpressionIndex,
        body_statement:       StatementIndex,
    },
    DoWhile {
        condition_expression: ExpressionIndex,
        body_statement:       StatementIndex,
    },
    For {
        initializer_statement: Option<ForInitializer>,
        condition_expression:  Option<ExpressionIndex>,
        post_expression:       Option<ExpressionIndex>,
        body_statement:        StatementIndex,
    },
    Return(ExpressionIndex),
    Break,
    Continue,
    Goto(Identifier),
    Label(Identifier, StatementIndex),
    Case(ConstantExpressionIndex, StatementIndex),
    Default(StatementIndex),
    Null,
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[expect(
    dead_code,
    reason = "The future statement frame will construct these initializer variants."
)]
pub(crate) enum ForInitializer {
    Expression(ExpressionIndex),
    Declaration(DeclarationIndex),
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Expression {
    pub(crate) kind:           ExpressionType,
    pub(crate) source_vectors: SourceVectors,
}

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

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Identifier {
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
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Default)]
pub(crate) enum StorageClass {
    #[default]
    Auto,
    Register,
    Static,
    Extern,
    Typedef,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ParserError {
    pub(crate) error_type:     ParserErrorType,
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

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ParserErrorType {
    UnexpectedEndOfFrame {
        frame:    &'static str,
        expected: &'static str,
    },
    UnexpectedToken {
        frame:    &'static str,
        expected: &'static str,
        found:    TokenType,
    },
    FutureChildNotImplemented(FutureChildKind),
    StorageClassRedefinition(StorageClass, TokenType),
    ConstSpecifiedTwice,
    VolatileSpecifiedTwice,
    RestrictSpecifiedTwice,
    InlineSpecifiedTwice,
    StaticSpecifiedTwice,
    TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
    ConflictingTypeSpecifiers(TypeSpecifiers, TokenType),
    TypeSpecifierSpecifiedTwice(TokenType),
    LongSpecifiedThrice,
    LongLongDoubleSpecified,
    EmptyDeclarationSpecifiers(TokenType),
    NoTypeSpecifiersInDeclarationSpecifiers(TokenType),
    BothStaticAndPointerInArrayDirectDeclarator,
    ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator,
    EmptyStructDeclarator,
}

impl GetSeverity for ParserErrorType {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::UnexpectedEndOfFrame { .. }
            | Self::UnexpectedToken { .. }
            | Self::FutureChildNotImplemented(..)
            | Self::EmptyDeclarationSpecifiers(..)
            | Self::NoTypeSpecifiersInDeclarationSpecifiers(..)
            | Self::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator
            | Self::BothStaticAndPointerInArrayDirectDeclarator
            | Self::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator
            | Self::EmptyStructDeclarator => ErrorSeverity::Error,
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
            | Self::UnexpectedEndOfFrame { frame, expected } => {
                write!(
                    f,
                    "Unexpected end of input in {frame}; expected {expected}!"
                )
            },
            | Self::UnexpectedToken {
                frame,
                expected,
                found,
            } => write!(
                f,
                "Unexpected token {found:?} in {frame}; expected {expected}!"
            ),
            | Self::FutureChildNotImplemented(kind) => write!(
                f,
                "The {} parser is not implemented yet; skipped this grammar child.",
                kind.frame_name()
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
            | Self::EmptyStructDeclarator => {
                write!(f, "Expected a declarator in the struct member declaration!")
            },
        }
    }
}

impl TranslationPhase for Parser {
    type Item = ExternalDeclaration;

    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item> {
        self.drive(context)
    }
}

// Keep the internal production entry points type-checked before a later phase
// wires the parser into the CLI pipeline. These signature guards make the
// implemented kernel reachable to dead-code analysis without suppressing the
// module wholesale.
const _: fn(Preprocessor) -> Parser = Parser::new;
const _: fn(&mut Parser, &mut Context) -> Option<ExternalDeclaration> =
    <Parser as TranslationPhase>::next_item;

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
        let ExternalDeclaration::Declaration(index) = parsed.items[item] else {
            panic!("expected a declaration item")
        };
        &parsed.parser.syntax.declarations[index.0 as usize]
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
            .pointer_declarator
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
        assert_eq!(
            parsed.parser.syntax.enumerator_sources.len(),
            parsed.parser.syntax.enumerators.len()
        );
        assert!(
            parsed
                .parser
                .syntax
                .enumerator_sources
                .iter()
                .all(|source| source.length > 0)
        );
        assert_eq!(
            parsed.parser.syntax.struct_declaration_sources.len(),
            parsed.parser.syntax.struct_declarations.len()
        );
        assert_eq!(
            parsed.parser.syntax.struct_declarator_sources.len(),
            parsed.parser.syntax.struct_declarators.len()
        );
        assert!(
            parsed
                .parser
                .syntax
                .struct_declaration_sources
                .iter()
                .chain(&parsed.parser.syntax.struct_declarator_sources)
                .all(|source| source.length > 0)
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
                | ParserErrorType::FutureChildNotImplemented(kind) => Some(*kind),
                | _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            future_children,
            [
                FutureChildKind::FunctionBody,
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
    fn malformed_external_declarations_reduce_to_error_nodes_and_continue() {
        let parsed = parse("}\nint after;\n");

        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::Error(_))
        ));
        assert!(matches!(
            parsed.items.get(1),
            Some(ExternalDeclaration::Declaration(_))
        ));
        assert!(parser_errors(&parsed).any(|error| matches!(
            error,
            ParserErrorType::EmptyDeclarationSpecifiers(..)
                | ParserErrorType::UnexpectedToken { .. }
        )));
    }

    #[test]
    fn malformed_parameter_recovery_stops_at_comma_and_keeps_the_next_parameter() {
        let parsed = parse("int f(int x +, char y);\nint after;\n");

        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::Error(_))
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
                .parameter_sources
                .iter()
                .map(|source| sourced_text(&parsed, *source))
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
    fn malformed_array_bound_recovery_stops_at_the_owning_bracket() {
        let parsed = parse("int a[(1];\nint after;\n");

        assert!(matches!(
            parsed.items.first(),
            Some(ExternalDeclaration::Error(_))
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
        let parenthesized_source = parsed
            .parser
            .syntax
            .declarator_sources
            .iter()
            .copied()
            .find(|source| sourced_text(&parsed, *source) == "(*value)")
            .expect("parenthesized declarator source");
        assert_eq!(sourced_text(&parsed, parenthesized_source), "(*value)");
        assert_eq!(
            identifier_name(&parsed, parenthesized).as_deref(),
            Some("value")
        );

        let parameter_text = parsed
            .parser
            .syntax
            .parameter_sources
            .iter()
            .map(|source| sourced_text(&parsed, *source))
            .collect::<Vec<_>>();
        assert_eq!(parameter_text, ["constchar*name", "unsignedcount"]);

        let member_text = parsed
            .parser
            .syntax
            .struct_declaration_sources
            .iter()
            .map(|source| sourced_text(&parsed, *source))
            .collect::<Vec<_>>();
        assert_eq!(member_text, ["intfirst,*second;", "unsignedbits:3;"]);
        let member_declarator_text = parsed
            .parser
            .syntax
            .struct_declarator_sources
            .iter()
            .map(|source| sourced_text(&parsed, *source))
            .collect::<Vec<_>>();
        assert_eq!(member_declarator_text, ["first", "*second", "bits:3"]);
    }

    #[test]
    fn malformed_and_eof_paths_terminate_with_source_backed_diagnostics() {
        for source in [
            "int\n",
            "int *\n",
            "int (value\n",
            "int (value;\n",
            "int array[\n",
            "int array[*;\n",
            "int function(int\n",
            "int function(int, ...;\n",
            "struct S { int member\n",
            "struct S { int first,\n",
            "struct S { int member }\n",
            "struct {\n",
            "enum E { A\n",
            "enum E { A,, B };\n",
            "enum {\n",
            "typedef ;\n",
            "}\nint after;\n",
        ] {
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
                !errors.is_empty(),
                "missing parser diagnostic for {source:?}"
            );
            assert!(errors.iter().all(|error| error.source_vectors.length > 0));
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
        }
    }
}
