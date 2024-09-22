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
    preprocessing::{
        CharacterTokenType,
        FloatTokenType,
        IntegerTokenType,
        KeywordTokenType,
        Preprocessor,
        Token,
        TokenType,
    },
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

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Parser {
    pub(crate) preprocessor:               Preprocessor,
    pub(crate) type_names:                 Vec<TypeName>,
    pub(crate) pending_token:              Option<Token>,
    pub(crate) expressions:                Vec<Expression>,
    pub(crate) statements:                 Vec<Statement>,
    pub(crate) type_qualifiers:            Vec<TypeQualifiers>,
    pub(crate) declarator_types:           Vec<DirectDeclarator>,
    pub(crate) identifiers:                Vec<Identifier>,
    pub(crate) typedef_names:              HashMap<StringCacheId, TypeIndex>,
    pub(crate) struct_names:               HashMap<StringCacheId, TypeIndex>,
    pub(crate) enum_names:                 HashMap<StringCacheId, TypeIndex>,
    pub(crate) parameter_declarations:     Vec<ParameterDeclaration>,
    pub(crate) struct_or_union_specifiers: Vec<StructOrUnionSpecifier>,
    pub(crate) struct_declarations:        Vec<StructDeclaration>,
    pub(crate) struct_declarators:         Vec<StructDeclarator>,
    pub(crate) enum_specifiers:            Vec<EnumSpecifier>,
    pub(crate) enumerators:                Vec<Enumerator>,
}

impl GetPosition for Parser {
    fn position(&self, context: &Context) -> SourcePosition {
        self.preprocessor.position(context)
    }
}

impl SetPosition for Parser {
    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        self.preprocessor.set_position(context, position);
    }
}

impl GetSourceFileIndex for Parser {
    fn source_file_index(&self) -> u32 {
        self.preprocessor.source_file_index()
    }
}

impl SetSourceFileIndex for Parser {
    fn set_source_file_index(&mut self, context: &mut Context, source_file_index: u32) {
        self.preprocessor
            .set_source_file_index(context, source_file_index);
    }
}

/// declaration:
/// - declaration-specifiers init-declarator-list? ;
pub(crate) struct Declaration {
    pub(crate) declaration_specifiers: DeclarationSpecifiers,
    /// init-declarator-list
    pub(crate) init_declarators:       VectorSlice<InitDeclarator>,
}

/// init-declarator:
/// - declarator
/// - declarator = initializer
pub(crate) struct InitDeclarator {
    pub(crate) declarator:  Declarator,
    pub(crate) initializer: Option<Initializer>,
}

/// initializer:
/// - assignment-expression
/// - { initializer-list }
/// - { initializer-list , }
pub(crate) enum Initializer {
    AssignmentExpression(ExpressionIndex),
    InitializerList(VectorSlice<Initializer>),
}

bitfield::bitfield! {
    #[derive(PartialEq, Eq, Hash, Clone, Copy, Default)]
    /// type-qualifier:
    /// - const
    /// - restrict
    /// - volatile
    ///
    /// Represents all the type-qualifiers in a declaration. For example, the declaration `const volatile int foo;` would be represented as `TypeQualifiers(Const | Volatile)`.
    pub(crate) struct TypeQualifiers(u8);
    impl Debug;
    pub is_const, set_is_const: 0;
    pub is_volatile, set_is_volatile: 1;
    pub is_restrict, set_is_restrict: 2;
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
/// type_qualifiers and type_specifiers are split into two fields.
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
/// example, the type_qualifiers field contains all the type qualifiers in the
/// declaration.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct DeclarationSpecifiers {
    pub(crate) storage_class:       StorageClass,
    pub(crate) type_qualifiers:     TypeQualifiers,
    pub(crate) type_specifiers:     TypeSpecifiers,
    pub(crate) function_specifiers: FunctionSpecifiers,
}

impl Default for DeclarationSpecifiers {
    fn default() -> Self {
        Self::new()
    }
}

impl DeclarationSpecifiers {
    const fn new() -> Self {
        Self {
            storage_class:       StorageClass::Auto,
            type_qualifiers:     TypeQualifiers(0),
            type_specifiers:     TypeSpecifiers::Empty,
            function_specifiers: FunctionSpecifiers { is_inline: false },
        }
    }
}

/// pointer:
/// - type-qualifier-list?
/// - type-qualifier-list? pointer
///
/// Each element in the type_qualifiers_list represents the type qualifiers for
/// one level of indirection. For example, this declaration: `*const *volatile
/// *x` would be parsed as: `[TypeQualifiers(const), TypeQualifiers(volatile),
/// TypeQualifiers(0)]`
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

// FIXME: Change this to an enum when using enums as const generics is
// supported.
const IS_ABSTRACT_DECLARATOR: u8 = 0;
const IS_NOT_ABSTRACT_DECLARATOR: u8 = 1;
const IS_MAYBE_ABSTRACT_DECLARATOR: u8 = 2;

// FIXME: Change this to an enum when using enums as const generics is
// supported.
const IS_FUNCTION_DEFINITION: u8 = 0;
const IS_NOT_FUNCTION_DEFINITION: u8 = 1;
const IS_MAYBE_FUNCTION_DEFINITION: u8 = 2;

impl Parser {
    pub(crate) fn new(preprocessor: Preprocessor) -> Self {
        Self {
            preprocessor,
            type_names: Vec::new(),
            pending_token: None,
            expressions: Vec::new(),
            statements: Vec::new(),
            type_qualifiers: Vec::new(),
            identifiers: Vec::new(),
            declarator_types: Vec::new(),
            enum_names: HashMap::default(),
            struct_names: HashMap::default(),
            typedef_names: HashMap::default(),
            parameter_declarations: Vec::new(),
            struct_or_union_specifiers: Vec::new(),
            struct_declarations: Vec::new(),
            struct_declarators: Vec::new(),
            enum_specifiers: Vec::new(),
            enumerators: Vec::new(),
        }
    }

    fn next_token(&mut self, context: &mut Context) -> Option<Token> {
        if let Some(token) = self.pending_token.take() {
            return Some(token);
        }
        self.preprocessor.next_item(context)
    }

    fn parse_statement(&mut self, _context: &mut Context) -> Statement {
        todo!();
    }

    fn parse_expression<const IS_CONSTANT_EXPRESSION: bool>(
        &mut self,
        _context: &mut Context,
    ) -> ExpressionIndex {
        todo!();
    }

    fn parse_assignment_expression(&mut self, _context: &mut Context) -> ExpressionIndex {
        todo!();
    }

    fn parse_struct_or_union_declaration(
        &mut self,
        context: &mut Context,
        token: Token,
    ) -> StructOrUnionSpecifierIndex {
        assert!(
            matches!(
                token.kind,
                TokenType::Keyword(KeywordTokenType::Struct | KeywordTokenType::Union)
            ),
            "parse_struct_or_union_declaration called with non-struct-or-union token."
        );
        let start_index = self.struct_or_union_specifiers.len().to_u32();
        let name = self.parse_maybe_identifier(
            context,
            "while parsing struct-or-union-declarator",
            |_| None,
        );
        let Some(next) = self.next_token(context) else {
            let source_vectors =
                context.create_source_vectors(self.position(context), self.source_file_index(), 0);
            context.parser_error(ParserError {
                error_type: ParserErrorType::UnexpectedEndOfInput(
                    "while parsing struct-or-union-declarator",
                ),
                source_vectors,
            });
            if let Some(v) = name {
                self.struct_or_union_specifiers
                    .push(StructOrUnionSpecifier {
                        struct_or_union:         StructOrUnion::Struct,
                        identifier:              Some(v),
                        struct_declaration_list: None,
                    });
                return StructOrUnionSpecifierIndex(start_index);
            }
            return StructOrUnionSpecifierIndex(u32::MAX);
        };
        if next.kind != TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) {
            if let Some(v) = name {
                self.struct_or_union_specifiers
                    .push(StructOrUnionSpecifier {
                        struct_or_union:         StructOrUnion::Struct,
                        identifier:              Some(v),
                        struct_declaration_list: None,
                    });
                return StructOrUnionSpecifierIndex(start_index);
            }
            return StructOrUnionSpecifierIndex(u32::MAX);
        }
        // Parse struct-declaration-list
        // struct-declaration-list:
        // - struct-declaration
        // - struct-declaration-list struct-declaration
        // struct-declaration:
        // - specifier-qualifier-list struct-declarator-list ;
        // specifier-qualifier-list:
        // - type-specifier specifier-qualifier-list?
        // - type-qualifier specifier-qualifier-list?
        // struct-declarator-list:
        // - struct-declarator
        // - struct-declarator-list , struct-declarator
        // struct-declarator:
        // - declarator
        // - declarator? : constant-expression
        let struct_declarations_start_index = self.struct_declarations.len().to_u32();
        'struct_declaration_list: loop {
            let struct_declarators_start_index = self.struct_declarators.len().to_u32();
            let mut type_qualifiers = TypeQualifiers(0);
            let mut type_specifiers = TypeSpecifiers::Empty;
            // Parse specifiers-qualifier-list:
            #[allow(unused_labels)]
            'struct_qualifier_list: loop {
                let Some(token) = self.next_token(context) else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.parser_error(ParserError {
                        error_type: ParserErrorType::UnexpectedEndOfInput(
                            "while parsing struct-qualifier-list",
                        ),
                        source_vectors,
                    });
                    break;
                };
                if let Some(()) = self.handle_type_specifier(context, &mut type_specifiers, token) {
                    continue;
                }
                if let Some(()) = self.handle_type_qualifier(context, &mut type_qualifiers, token) {
                    continue;
                }
                break;
            }
            // If type_specifiers and type_qualifiers are both empty, we must have reached
            // the end of the struct-declaration-list.
            if type_specifiers == TypeSpecifiers::Empty && type_qualifiers == TypeQualifiers(0) {
                let Some(closing_curly_brace) = self.next_token(context) else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.parser_error(ParserError {
                        error_type: ParserErrorType::UnexpectedEndOfInput(
                            "while parsing struct-declaration-list",
                        ),
                        source_vectors,
                    });
                    break 'struct_declaration_list;
                };
                if closing_curly_brace.kind
                    != TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
                {
                    let start_position = closing_curly_brace.source_vectors.position(context);
                    let source_vectors =
                        context.create_source_vectors(start_position, self.source_file_index(), 0);
                    context.parser_error(ParserError {
                        error_type:
                            ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(
                                closing_curly_brace.kind,
                            ),
                        source_vectors,
                    });
                    self.pending_token = Some(closing_curly_brace);
                }
                break 'struct_declaration_list;
            }
            'struct_declarator_list: loop {
                let declarator = self.parse_declarator::<IS_NOT_ABSTRACT_DECLARATOR>(context);
                let mut bitfield_width = None;
                #[allow(unused_labels)]
                'bitfield: loop {
                    let Some(maybe_colon_or_comma) = self.next_token(context) else {
                        let source_vectors = context.create_source_vectors(
                            self.position(context),
                            self.source_file_index(),
                            0,
                        );
                        context.parser_error(ParserError {
                            error_type: ParserErrorType::UnexpectedEndOfInput(
                                "while parsing struct-declarator-list",
                            ),
                            source_vectors,
                        });
                        break 'struct_declaration_list;
                    };
                    match maybe_colon_or_comma.kind {
                        | TokenType::Operator(OperatorTokenType::Comma) => {
                            self.struct_declarators.push(StructDeclarator {
                                declarator,
                                bitfield_width,
                            });
                            continue 'struct_declarator_list;
                        },
                        | TokenType::Operator(OperatorTokenType::Colon) => {
                            if bitfield_width.is_some() {
                                let source_vectors = context.create_source_vectors(
                                    self.position(context),
                                    self.source_file_index(),
                                    0,
                                );
                                context.parser_error(ParserError {
                                    error_type:
                                        ParserErrorType::MultipleBitfieldWidthsInStructDeclarator,
                                    source_vectors,
                                });
                            }
                            bitfield_width = Some(self.parse_expression::<true>(context));
                        },
                        | TokenType::Operator(OperatorTokenType::Semicolon) => {
                            if declarator.is_none() && bitfield_width.is_none() {
                                let source_vectors = context.create_source_vectors(
                                    self.position(context),
                                    self.source_file_index(),
                                    0,
                                );
                                context.parser_error(ParserError {
                                    error_type: ParserErrorType::EmptyStructDeclarator,
                                    source_vectors,
                                });
                            }
                            self.struct_declarators.push(StructDeclarator {
                                declarator,
                                bitfield_width,
                            });
                            break 'struct_declarator_list;
                        },
                        | _ => {
                            let source_vectors = context.create_source_vectors(
                                self.position(context),
                                self.source_file_index(),
                                0,
                            );
                            context.parser_error(ParserError {
                                error_type: ParserErrorType::ExpectedCommaColonOrSemicolonInStructDeclarator(maybe_colon_or_comma.kind),
                                source_vectors,
                            });
                            self.pending_token = Some(maybe_colon_or_comma);
                            break 'struct_declarator_list;
                        },
                    }
                }
            }
            self.struct_declarations.push(StructDeclaration {
                type_qualifiers,
                type_specifiers,
                struct_declarator_list: VectorSlice::new(
                    struct_declarators_start_index,
                    self.struct_declarators.len().to_u32(),
                ),
            });
        }
        self.struct_or_union_specifiers
            .push(StructOrUnionSpecifier {
                struct_or_union:         StructOrUnion::Struct,
                identifier:              name,
                struct_declaration_list: Some(VectorSlice::new(
                    struct_declarations_start_index,
                    self.struct_declarations.len().to_u32(),
                )),
            });
        StructOrUnionSpecifierIndex(start_index)
    }

    fn parse_enum_declaration(
        &mut self,
        context: &mut Context,
        token: Token,
    ) -> EnumSpecifierIndex {
        assert!(matches!(
            token.kind,
            TokenType::Keyword(KeywordTokenType::Enum)
        ),);
        let maybe_name =
            self.parse_maybe_identifier(context, "while parsing enum-declarator", |_| None);
        let Some(maybe_opening_curly_brace) = self.next_token(context) else {
            let source_vectors =
                context.create_source_vectors(self.position(context), self.source_file_index(), 0);
            context.parser_error(ParserError {
                error_type: ParserErrorType::UnexpectedEndOfInput("while parsing enum-declarator"),
                source_vectors,
            });
            if let Some(v) = maybe_name {
                self.enum_specifiers.push(EnumSpecifier {
                    name:             Some(v),
                    enumeration_list: None,
                });
                return EnumSpecifierIndex(self.enum_specifiers.len().to_u32() - 1);
            }
            return EnumSpecifierIndex(u32::MAX);
        };
        if maybe_opening_curly_brace.kind
            != TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
        {
            if maybe_name.is_none() {
                let source_vectors = context.create_source_vectors(
                    maybe_opening_curly_brace.source_vectors.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::EnumSpecifierWithoutNameAndBody(
                        maybe_opening_curly_brace.kind,
                    ),
                    source_vectors,
                });
                self.pending_token = Some(maybe_opening_curly_brace);
                return EnumSpecifierIndex(u32::MAX);
            }
            self.enum_specifiers.push(EnumSpecifier {
                name:             maybe_name,
                enumeration_list: None,
            });
            return EnumSpecifierIndex(self.enum_specifiers.len().to_u32() - 1);
        }
        let enumerator_list_start_index = self.identifiers.len().to_u32();
        'enumerator_list: loop {
            let mut enumeration_constant = None;
            let mut constant_expression = None;
            let Some(maybe_enumeration_constant) = self.next_token(context) else {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "while parsing enumerator-list",
                    ),
                    source_vectors,
                });
                break 'enumerator_list;
            };
            if maybe_enumeration_constant.kind
                == TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
            {
                break 'enumerator_list;
            }
            if maybe_enumeration_constant.kind != TokenType::Identifier {
                let source_vectors = context.create_source_vectors(
                    maybe_enumeration_constant.source_vectors.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type:
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            maybe_enumeration_constant.kind,
                        ),
                    source_vectors,
                });
                self.pending_token = Some(maybe_enumeration_constant);
                break 'enumerator_list;
            }
            enumeration_constant = Some(Identifier::new(maybe_enumeration_constant.contents));
            let Some(maybe_assignment_operator) = self.next_token(context) else {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "while parsing enumerator-list",
                    ),
                    source_vectors,
                });
                break 'enumerator_list;
            };
            if maybe_assignment_operator.kind == TokenType::Operator(OperatorTokenType::Equals) {
                constant_expression = Some(self.parse_expression::<true>(context));
            } else {
                self.pending_token = Some(maybe_assignment_operator);
            }
            self.enumerators.push(Enumerator {
                name:       enumeration_constant.unwrap(),
                expression: constant_expression,
            });
            let Some(maybe_comma) = self.next_token(context) else {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "while parsing enumerator-list",
                    ),
                    source_vectors,
                });
                break 'enumerator_list;
            };
            if maybe_comma.kind != TokenType::Operator(OperatorTokenType::Comma) {
                self.pending_token = Some(maybe_comma);
            }
        }
        let ret = self.enum_specifiers.len().to_u32();
        self.enum_specifiers.push(EnumSpecifier {
            name:             maybe_name,
            enumeration_list: Some(VectorSlice::new(
                enumerator_list_start_index,
                self.enumerators.len().to_u32(),
            )),
        });
        EnumSpecifierIndex(ret)
    }

    fn parse_identifier(
        &mut self,
        context: &mut Context,
        eof_message: &'static str,
        on_error: impl FnOnce(Token) -> ParserErrorType,
    ) -> Identifier {
        self.parse_maybe_identifier(context, eof_message, |token| Some(on_error(token)))
            .expect("parse_maybe_identifier returned None when called with infallible on_error.")
    }

    fn parse_maybe_identifier(
        &mut self,
        context: &mut Context,
        eof_message: &'static str,
        on_error: impl FnOnce(Token) -> Option<ParserErrorType>,
    ) -> Option<Identifier> {
        let Some(token) = self.next_token(context) else {
            let source_vectors =
                context.create_source_vectors(self.position(context), self.source_file_index(), 0);
            context.parser_error(ParserError {
                error_type: ParserErrorType::UnexpectedEndOfInput(eof_message),
                source_vectors,
            });
            return Some(Identifier {
                name: context.string_cache.intern("<non-existent-identifier>"),
            });
        };
        match token.kind {
            | TokenType::Identifier => Some(Identifier {
                name: token.contents,
            }),
            | _tt => {
                self.pending_token = Some(token);
                context.parser_error(ParserError {
                    error_type:     on_error(token)?,
                    source_vectors: token.source_vectors,
                });
                Some(Identifier {
                    name: context.string_cache.intern("<non-existent-identifier>"),
                })
            },
        }
    }

    #[inline(always)]
    fn parse_declaration_or_function_definition<const IS_FUNCTION: u8>(
        &mut self,
        context: &mut Context,
    ) -> ExternalDeclaration {
        let statement_start_index = self.statements.len().to_u32();
        loop {
            match self.next_token(context) {
                | Some(token)
                    if token.kind == TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) =>
                    break,
                | None => break,
                | Some(token) => {
                    self.pending_token = Some(token);
                    let statement = self.parse_statement(context);
                    self.statements.push(statement);
                },
            }
        }
        let end_index = self.statements.len().to_u32();
        ExternalDeclaration::FunctionDefinition(FunctionDefinition {
            declaration: FunctionDeclaration {
                name,
                parameters,
                return_type,
            },
            statements:  VectorSlice::new(statement_start_index, end_index),
        })
    }

    fn parse_declaration(&mut self, context: &mut Context) -> DeclarationIndex {
        match self.parse_declaration_or_function_definition::<IS_NOT_FUNCTION_DEFINITION>(context) {
            | ExternalDeclaration::Declaration(declaration) => declaration,
            | _ => unreachable!(
                "parse_declaration_or_function_definition returned non-declaration when called \
                 with IS_NOT_FUNCTION_DEFINITION."
            ),
        }
    }

    fn handle_storage_class(
        &mut self,
        context: &mut Context,
        storage_class: &mut StorageClass,
        token: Token,
        storage_class_specified: &mut bool,
    ) -> Option<()> {
        if *storage_class_specified {
            context.parser_error(ParserError {
                error_type:     ParserErrorType::StorageClassRedefinition(
                    *storage_class,
                    token.kind,
                ),
                source_vectors: token.source_vectors,
            });
        }
        let new = match token.kind {
            | TokenType::Keyword(KeywordTokenType::Auto) => StorageClass::Auto,
            | TokenType::Keyword(KeywordTokenType::Register) => StorageClass::Register,
            | TokenType::Keyword(KeywordTokenType::Static) => StorageClass::Static,
            | TokenType::Keyword(KeywordTokenType::Extern) => StorageClass::Extern,
            | TokenType::Keyword(KeywordTokenType::Typedef) => StorageClass::Typedef,
            | _ => {
                self.pending_token = Some(token);
                return None;
            },
        };
        *storage_class = new;
        *storage_class_specified = true;
        Some(())
    }

    fn handle_keyword_type_specifier(
        &mut self,
        context: &mut Context,
        type_specifiers: &mut TypeSpecifiers,
        token: Token,
    ) {
        let TokenType::Keyword(keyword_token_type) = token.kind else {
            unreachable!("set_type_specifiers called with non-keyword token.");
        };
        match keyword_token_type {
            | KeywordTokenType::Int =>
                if type_specifiers.is_int() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_int(self, context);
                },
            | KeywordTokenType::Long =>
                if type_specifiers.is_long_long() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::LongSpecifiedThrice,
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_long_double() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::LongLongDoubleSpecified,
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_long(self, context);
                },
            | KeywordTokenType::Short =>
                if type_specifiers.is_short() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_short(self, context);
                },
            | KeywordTokenType::Signed =>
                if type_specifiers.is_signed() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_signed(self, context);
                },
            | KeywordTokenType::Unsigned =>
                if type_specifiers.is_unsigned() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_unsigned(self, context);
                },
            | KeywordTokenType::Float =>
                if type_specifiers.is_float() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_float(self, context);
                },
            | KeywordTokenType::Double =>
                if type_specifiers.is_double() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_long_double() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::LongLongDoubleSpecified,
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_double(self, context);
                },
            | KeywordTokenType::Void =>
                if type_specifiers.is_void() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_void(self, context);
                },
            | KeywordTokenType::Bool =>
                if type_specifiers.is_bool() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_bool(self, context);
                },
            | KeywordTokenType::Complex =>
                if type_specifiers.is_complex() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_complex(self, context);
                },
            | KeywordTokenType::Imaginary =>
                if type_specifiers.is_imaginary() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_imaginary(self, context);
                },
            | KeywordTokenType::Char =>
                if type_specifiers.is_char() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    type_specifiers.make_char(self, context);
                },
            | _ => unreachable!("set_type_specifiers called with non-type-specifier token."),
        }
    }

    fn handle_type_specifier(
        &mut self,
        context: &mut Context,
        type_specifiers: &mut TypeSpecifiers,
        token: Token,
    ) -> Option<()> {
        match token.kind {
            | TokenType::Keyword(
                KeywordTokenType::Int
                | KeywordTokenType::Short
                | KeywordTokenType::Long
                | KeywordTokenType::Char
                | KeywordTokenType::Signed
                | KeywordTokenType::Unsigned
                | KeywordTokenType::Float
                | KeywordTokenType::Double
                | KeywordTokenType::Void
                | KeywordTokenType::Bool
                | KeywordTokenType::Complex
                | KeywordTokenType::Imaginary,
            ) => self.handle_keyword_type_specifier(context, type_specifiers, token),
            | TokenType::Keyword(KeywordTokenType::Struct | KeywordTokenType::Union) => {
                let struct_declaration = self.parse_struct_or_union_declaration(context, token);
                type_specifiers.make_struct_or_union(self, context, struct_declaration)
            },
            | TokenType::Keyword(KeywordTokenType::Enum) => {
                let enum_declaration = self.parse_enum_declaration(context, token);
                type_specifiers.make_enum(self, context, enum_declaration)
            },
            | TokenType::Identifier if self.typedef_names.contains_key(&token.contents) =>
                type_specifiers.make_typedef_name(self, context, Identifier::new(token.contents)),
            | _ => {
                self.pending_token = Some(token);
                return None;
            },
        }
        Some(())
    }

    fn handle_type_qualifier(
        &mut self,
        context: &mut Context,
        type_qualifiers: &mut TypeQualifiers,
        token: Token,
    ) -> Option<()> {
        match token.kind {
            | TokenType::Keyword(KeywordTokenType::Const) => {
                if type_qualifiers.is_const() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConstSpecifiedTwice,
                        source_vectors: token.source_vectors,
                    });
                }
                type_qualifiers.set_is_const(true);
            },
            | TokenType::Keyword(KeywordTokenType::Volatile) => {
                if type_qualifiers.is_volatile() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::VolatileSpecifiedTwice,
                        source_vectors: token.source_vectors,
                    });
                }
                type_qualifiers.set_is_volatile(true);
            },
            | TokenType::Keyword(KeywordTokenType::Restrict) => {
                if type_qualifiers.is_restrict() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::RestrictSpecifiedTwice,
                        source_vectors: token.source_vectors,
                    });
                }
                type_qualifiers.set_is_restrict(true);
            },
            | _ => {
                self.pending_token = Some(token);
                return None;
            },
        }
        Some(())
    }

    fn handle_function_specifier(
        &mut self,
        context: &mut Context,
        function_specifiers: &mut FunctionSpecifiers,
        token: Token,
    ) -> Option<()> {
        match token.kind {
            | TokenType::Keyword(KeywordTokenType::Inline) => {
                if function_specifiers.is_inline {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::InlineSpecifiedTwice,
                        source_vectors: token.source_vectors,
                    });
                }
                function_specifiers.is_inline = true;
            },
            | _ => {
                self.pending_token = Some(token);
                return None;
            },
        }
        Some(())
    }

    fn parse_declaration_specifiers(&mut self, context: &mut Context) -> DeclarationSpecifiers {
        let mut specifiers = DeclarationSpecifiers::default();
        let mut storage_class_specified = false;
        loop {
            let Some(token) = self.next_token(context) else {
                if specifiers == DeclarationSpecifiers::default() {
                    return specifiers;
                }
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "parsing declaration specifiers. Expected a type specifier.",
                    ),
                    source_vectors,
                });
                return specifiers;
            };
            if let Some(()) =
                self.handle_type_qualifier(context, &mut specifiers.type_qualifiers, token)
            {
                continue;
            }
            if let Some(()) =
                self.handle_type_specifier(context, &mut specifiers.type_specifiers, token)
            {
                continue;
            }
            if let Some(()) =
                self.handle_function_specifier(context, &mut specifiers.function_specifiers, token)
            {
                continue;
            }
            if let Some(()) = self.handle_storage_class(
                context,
                &mut specifiers.storage_class,
                token,
                &mut storage_class_specified,
            ) {
                continue;
            }
            self.pending_token = Some(token);
            if specifiers == DeclarationSpecifiers::default() {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::EmptyDeclarationSpecifiers(token.kind),
                    source_vectors,
                });
            } else if specifiers.type_specifiers == TypeSpecifiers::Empty {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::NoTypeSpecifiersInDeclarationSpecifiers(
                        token.kind,
                    ),
                    source_vectors,
                });
            }
            return specifiers;
        }
    }

    fn parse_declarator<const IS_ABSTRACT: u8>(
        &mut self,
        context: &mut Context,
    ) -> Option<Declarator> {
        let pointer_declarator = self.parse_pointer_declarator(context);
        let vector_slice = self.parse_direct_declarator::<IS_ABSTRACT>(context);
        if vector_slice.length == 0 {
            // Non-abstract declarators must have at least one direct declarator.
            if IS_ABSTRACT == IS_NOT_ABSTRACT_DECLARATOR
                && pointer_declarator.type_qualifiers_list.length != 0
            {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::TypeQualifiersWithoutDeclarator,
                    source_vectors,
                });
            }
            return None;
        }
        Some(Declarator {
            pointer_declarator,
            kind: vector_slice,
        })
    }

    // Question marks indicate optional parts of the grammar.
    // C99 standard:
    //      direct-declarator:
    //      identifier
    //      ( declarator )
    //      direct-declarator [ type-qualifier-list? assignment-expression? ]
    //      direct-declarator [ static type-qualifier-list? assignment-expression ]
    //      direct-declarator [ type-qualifier-list static assignment-expression ]
    //      direct-declarator [ type-qualifier-list? *]
    //      direct-declarator ( parameter-type-list )
    //      direct-declarator ( identifier-list? )
    fn parse_direct_declarator<const IS_ABSTRACT: u8>(
        &mut self,
        context: &mut Context,
    ) -> VectorSlice<DirectDeclarator> {
        let start_index = self.declarator_types.len().to_u32();
        // The first part of a declarator is only required to be an identifier or a
        // parenthesized declarator if it is not abstract.
        if IS_ABSTRACT == IS_NOT_ABSTRACT_DECLARATOR
            && self
                .parse_first_direct_declarator::<IS_ABSTRACT>(context)
                .is_none()
        {
            return VectorSlice::new(start_index, self.declarator_types.len().to_u32());
        }
        loop {
            if self
                .parse_nested_direct_declarator::<IS_ABSTRACT>(context)
                .is_none()
            {
                return VectorSlice::new(start_index, self.declarator_types.len().to_u32());
            }
        }
    }

    // None indicates a nested direct declarator was not parsed.
    // Parses a nested direct declarator, AKA the part after the initial identifier
    // or the parenthesized declarator in a direct declarator.
    fn parse_nested_direct_declarator<const IS_ABSTRACT: u8>(
        &mut self,
        context: &mut Context,
    ) -> Option<()> {
        let token = self.next_token(context)?;
        if token.kind == TokenType::Operator(OperatorTokenType::OpeningSquareBracket) {
            self.parse_array_direct_declarator::<IS_ABSTRACT>(context)
        } else if token.kind == TokenType::Operator(OperatorTokenType::OpeningParenthesis) {
            self.parse_function_direct_declarator::<IS_ABSTRACT>(context)
        } else {
            self.pending_token = Some(token);
            None
        }
    }

    fn parse_array_direct_declarator<const IS_ABSTRACT: u8>(
        &mut self,
        context: &mut Context,
    ) -> Option<()> {
        let mut is_static = false;
        let mut is_pointer = false;
        let mut type_qualifiers = TypeQualifiers(0);
        let mut type_qualifiers_before_static = false;
        let mut assignment_expression = None;
        macro_rules! push {
            () => {
                self.declarator_types.push(DeclaratorType::Array {
                    type_qualifiers,
                    is_static,
                    is_pointer,
                    assignment_expression,
                });
            };
        }
        loop {
            let Some(token) = self.next_token(context) else {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "parsing array direct declarator. Expected a closing square bracket.",
                    ),
                    source_vectors,
                });
                push!();
                return Some(());
            };
            match token.kind {
                | TokenType::Keyword(KeywordTokenType::Static) => {
                    if is_static {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::StaticSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    is_static = true;
                },
                | TokenType::Keyword(KeywordTokenType::Const) => {
                    if type_qualifiers.is_const() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::ConstSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if is_static && type_qualifiers_before_static {
                        context.parser_error(ParserError {
                            error_type: ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if !is_static {
                        type_qualifiers_before_static = true;
                    }
                    type_qualifiers.set_is_const(true);
                },
                | TokenType::Keyword(KeywordTokenType::Volatile) => {
                    if type_qualifiers.is_volatile() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::VolatileSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if is_static && type_qualifiers_before_static {
                        context.parser_error(ParserError {
                            error_type: ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if !is_static {
                        type_qualifiers_before_static = true;
                    }
                    type_qualifiers.set_is_volatile(true);
                },
                | TokenType::Keyword(KeywordTokenType::Restrict) => {
                    if type_qualifiers.is_restrict() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::RestrictSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if is_static && type_qualifiers_before_static {
                        context.parser_error(ParserError {
                            error_type: ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if !is_static {
                        type_qualifiers_before_static = true;
                    }
                    type_qualifiers.set_is_restrict(true);
                },
                | TokenType::Operator(OperatorTokenType::Asterisk) => {
                    if is_pointer {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::PointerSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if is_static {
                        context.parser_error(ParserError {
                            error_type:
                                ParserErrorType::BothStaticAndPointerInArrayDirectDeclarator,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if assignment_expression.is_some() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::PointerAfterAssignmentExpressionInArrayDirectDeclarator,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if type_qualifiers != TypeQualifiers(0) && IS_ABSTRACT == IS_ABSTRACT_DECLARATOR
                    {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator,
                            source_vectors: token.source_vectors,
                        });
                    }
                    is_pointer = true;
                    if !matches!(
                        self.next_token(context),
                        Some(Token {
                            kind: TokenType::Operator(OperatorTokenType::ClosingSquareBracket),
                            ..
                        })
                    ) {
                        // We know '*' can't be used as a unary operator in constant expressions, so
                        // we don't have to consider that case.
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(token.kind),
                            source_vectors: token.source_vectors,
                        });
                    }
                    push!();
                    return Some(());
                },
                | TokenType::Operator(OperatorTokenType::ClosingSquareBracket) => {
                    if assignment_expression.is_none() && is_static {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator,
                            source_vectors: token.source_vectors,
                        });
                    }
                    if assignment_expression.is_some() && is_pointer {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::AssignmentExpressionAfterPointerInArrayDirectDeclarator,
                            source_vectors: token.source_vectors,
                        });
                    }
                    push!();
                    return Some(());
                },
                | _ =>
                    if assignment_expression.is_none() && !is_pointer {
                        self.pending_token = Some(token);
                        assignment_expression = Some(self.parse_assignment_expression(context));
                    } else {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::ExpectedClosingSquareBracket(
                                token.kind,
                            ),
                            source_vectors: token.source_vectors,
                        });
                        self.pending_token = Some(token);
                        return None;
                    },
            }
        }
    }

    #[inline(never)]
    #[cold]
    fn parse_k_and_r_function_direct_declarator_eof_error(
        &mut self,
        context: &mut Context,
    ) -> Option<Result<(), ()>> {
        let position = self.position(context);
        let source_vectors = context.create_source_vectors(position, self.source_file_index(), 0);
        context.parser_error(ParserError {
            error_type: ParserErrorType::UnexpectedEndOfInput(
                "parsing K&R function direct declarator. Expected a closing parenthesis.",
            ),
            source_vectors,
        });
        Some(Err(()))
    }

    #[inline(never)]
    #[cold]
    fn mixed_k_and_r_and_modern_function_declarator_error(
        &mut self,
        context: &mut Context,
    ) -> Option<Result<(), ()>> {
        let source_vectors =
            context.create_source_vectors(self.position(context), self.source_file_index(), 0);
        context.parser_error(ParserError {
            error_type: ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator,
            source_vectors,
        });
        // Skip over tokens until we a closing brace.
        let mut brace_balance = 0isize;
        loop {
            let Some(next) = self.next_token(context) else {
                return self.parse_k_and_r_function_direct_declarator_eof_error(context);
            };
            match next.kind {
                | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => brace_balance += 1,
                | TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    if brace_balance == 1 =>
                    break,
                | TokenType::Operator(OperatorTokenType::ClosingParenthesis) => brace_balance -= 1,
                | _ => (),
            }
        }
        Some(Err(()))
    }

    /// Returns number of identifiers parsed on failure.
    fn parse_k_and_r_function_direct_declarator<const IS_ABSTRACT: u8>(
        &mut self,
        context: &mut Context,
    ) -> Option<Result<(), ()>> {
        let mut has_parsed_identifier = false;
        // K&R declarations are not supported in abstract declarators.
        if IS_ABSTRACT == IS_ABSTRACT_DECLARATOR {
            return None;
        }
        let start_index = self.identifiers.len().to_u32();
        loop {
            let Some(next) = self.next_token(context) else {
                return self.parse_k_and_r_function_direct_declarator_eof_error(context);
            };
            // We reached the end of the parameter list.
            if next.kind == TokenType::Operator(OperatorTokenType::ClosingParenthesis) {
                break;
            }
            // Turns out that this wasn't a K&R-style function declarator. If we haven't
            // parsed anything yet, we're all good and can just return Err(0).
            // Otherwise we have to generate an error because the parameter list
            // is half K&R style and half modern style which is not allowed.
            // The standard says that if an identifier could be a typedef name, it IS a
            // typedef name.
            if next.kind != TokenType::Identifier || self.typedef_names.contains_key(&next.contents)
            {
                // We create a parser error here, because the identifiers we previously parsed
                // are syntax errors since they are declarators with only an identifier, that is
                // not valid typedef.
                if has_parsed_identifier {
                    return self.mixed_k_and_r_and_modern_function_declarator_error(context);
                }
                // This is the first token, and we now know that this isn't a K&R-style function
                // declarator, so we return None to indicate this.
                self.pending_token = Some(next);
                return None;
            }
            has_parsed_identifier = true;
            let identifier = Identifier {
                name: next.contents,
            };
            self.identifiers.push(identifier);
        }
        self.declarator_types
            .push(DirectDeclarator::KAndRStyleFunction {
                parameters: VectorSlice::new(start_index, self.identifiers.len().to_u32()),
            });
        Some(Ok(()))
    }

    fn parse_function_direct_declarator<const IS_ABSTRACT: u8>(
        &mut self,
        context: &mut Context,
    ) -> Option<()> {
        match self.parse_k_and_r_function_direct_declarator::<IS_ABSTRACT>(context) {
            | None => (),
            | Some(Ok(())) => return Some(()),
            | Some(Err(())) => return None,
        }
        let start_index = self.parameter_declarations.len().to_u32();
        let mut is_variadic = false;
        // Parses this rule in the standard:
        // parameter-type-list:
        // - parameter-list
        // - parameter-list , ...
        // parameter-list:
        // - parameter-declaration
        // - parameter-list , parameter-declaration
        // parameter-declaration:
        // - declaration-specifiers declarator
        // - declaration-specifiers abstract-declarator?
        loop {
            let declaration_specifiers = self.parse_declaration_specifiers(context);
            let declarator = self.parse_declarator::<IS_MAYBE_ABSTRACT_DECLARATOR>(context);
            let parameter_declaration = ParameterDeclaration {
                declaration_specifiers,
                declarator,
            };
            self.parameter_declarations.push(parameter_declaration);
            let Some(next) = self.next_token(context) else {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "parsing function direct declarator. Expected a closing parenthesis.",
                    ),
                    source_vectors,
                });
                return None;
            };
            if next.kind == TokenType::Operator(OperatorTokenType::ClosingParenthesis) {
                break;
            }
            if next.kind == TokenType::Operator(OperatorTokenType::Comma) {
                let Some(maybe_ellipsis) = self.next_token(context) else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.parser_error(ParserError {
                        error_type: ParserErrorType::UnexpectedEndOfInput(
                            "parsing function direct declarator. Expected an ellipses or a \
                             closing parenthesis.",
                        ),
                        source_vectors,
                    });
                    self.pending_token = Some(next);
                    return None;
                };
                if maybe_ellipsis.kind == TokenType::Operator(OperatorTokenType::Ellipsis) {
                    is_variadic = true;
                    let Some(should_be_closing_parenthesis) = self.next_token(context) else {
                        let source_vectors = context.create_source_vectors(
                            self.position(context),
                            self.source_file_index(),
                            0,
                        );
                        context.parser_error(ParserError {
                            error_type: ParserErrorType::UnexpectedEndOfInput(
                                "parsing function direct declarator. Expected a closing \
                                 parenthesis.",
                            ),
                            source_vectors,
                        });
                        return None;
                    };
                    if should_be_closing_parenthesis.kind
                        == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    {
                        break;
                    }
                    let source_vectors = should_be_closing_parenthesis.source_vectors;
                    context.parser_error(ParserError {
                            error_type: ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                                should_be_closing_parenthesis.kind,
                            ),
                            source_vectors,
                        });
                    self.pending_token = Some(should_be_closing_parenthesis);
                } else {
                    self.pending_token = Some(maybe_ellipsis);
                }
                continue;
            }
            if next.kind != TokenType::Operator(OperatorTokenType::Comma) {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(next.kind),
                    source_vectors,
                });
                self.pending_token = Some(next);
                return None;
            }
        }
        self.declarator_types.push(DirectDeclarator::Function {
            parameter_list: VectorSlice::new(
                start_index,
                self.parameter_declarations.len().to_u32(),
            ),
            is_variadic,
        });
        Some(())
    }

    // Parses the first two rules of direct-declarator.
    fn parse_first_direct_declarator<const IS_ABSTRACT: u8>(
        &mut self,
        context: &mut Context,
    ) -> Option<()> {
        let token = self.next_token(context)?;
        // Matches this rule: direct-declarator: identifier
        if token.kind == TokenType::Identifier && IS_ABSTRACT == IS_NOT_ABSTRACT_DECLARATOR {
            self.declarator_types
                .push(DirectDeclarator::Identifier(Identifier {
                    name: token.contents,
                }));
            return Some(());
        }
        if token.kind == TokenType::Operator(OperatorTokenType::OpeningParenthesis) {
            let declarator = self.parse_declarator::<IS_ABSTRACT>(context);
            if declarator.is_none() {
                context.parser_error(ParserError {
                    error_type:     ParserErrorType::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator,
                    source_vectors: token.source_vectors,
                });
            }
            let token = self.next_token(context);
            if !matches!(
                token,
                Some(Token {
                    kind: TokenType::Operator(OperatorTokenType::ClosingParenthesis),
                    ..
                })
            ) {
                let source_vectors = token.map_or_else(
                    || {
                        context.create_source_vectors(
                            self.position(context),
                            self.source_file_index(),
                            0,
                        )
                    },
                    |t| t.source_vectors,
                );
                context.parser_error(ParserError {
                    error_type:
                        ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(
                            token.map(|t| t.kind),
                        ),
                    source_vectors,
                });
                self.pending_token = token;
            }
            let declarator = declarator?;
            self.declarator_types
                .push(DirectDeclarator::Parenthesized(declarator));
            return Some(());
        }
        // Declarators are only required to start with an identifier or a parenthesized
        // declarator if they're not abstract.
        if IS_ABSTRACT == IS_NOT_ABSTRACT_DECLARATOR {
            context.parser_error(ParserError {
                error_type:
                    ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(
                        token.kind,
                    ),
                source_vectors: token.source_vectors,
            });
        }
        // Returning None here is correct because either we've already parsed the first
        // part of the declarator, we haven't and we're parsing a non-abstract
        // declarator so we've reached an unrecoverable error, or we haven't and we're
        // parsing an abstract declarator so we haven't parsed anything and we should
        // return None to indicate this.
        self.pending_token = Some(token);
        None
    }

    fn parse_type_qualifiers(&mut self, context: &mut Context) -> TypeQualifiers {
        let mut ret = TypeQualifiers(0);
        loop {
            let Some(token) = self.next_token(context) else {
                break;
            };
            match token.kind {
                | TokenType::Keyword(KeywordTokenType::Const) => {
                    if ret.is_const() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::ConstSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    ret.set_is_const(true);
                },
                | TokenType::Keyword(KeywordTokenType::Volatile) => {
                    if ret.is_volatile() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::VolatileSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    ret.set_is_volatile(true);
                },
                | TokenType::Keyword(KeywordTokenType::Restrict) => {
                    if ret.is_restrict() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::RestrictSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    ret.set_is_restrict(true);
                },
                | _ => {
                    self.pending_token = Some(token);
                    break;
                },
            }
        }
        ret
    }

    fn parse_pointer_declarator(&mut self, context: &mut Context) -> PointerDeclarator {
        let start_index = self.type_qualifiers.len().to_u32();
        loop {
            let Some(token) = self.next_token(context) else {
                break;
            };
            if token.kind != TokenType::Operator(OperatorTokenType::Asterisk) {
                self.pending_token = Some(token);
                break;
            }
            let type_qualifiers = self.parse_type_qualifiers(context);
            self.type_qualifiers.push(type_qualifiers);
        }
        let type_qualifiers_list =
            VectorSlice::new(start_index, self.type_qualifiers.len().to_u32());
        PointerDeclarator {
            type_qualifiers_list,
        }
    }

    /// Parses either a declaration or a function definition.
    fn parse_external_declaration(&mut self, context: &mut Context) -> Option<ExternalDeclaration> {
        let token = self.next_token(context)?;
        self.pending_token = Some(token);
        Some(self.parse_declaration_or_function_definition::<IS_MAYBE_FUNCTION_DEFINITION>(context))
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum State {}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ExternalDeclaration {
    FunctionDefinition(FunctionDefinitionIndex),
    Declaration(DeclarationIndex),
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct FunctionDefinitionIndex(u32);

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
pub(crate) struct TypeIndex(u32);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct DeclaratorTypeIndex(u32);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct StructOrUnionSpecifierIndex(u32);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct EnumSpecifierIndex(u32);

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Statement {
    pub(crate) kind: StatementType,
}

#[derive(Debug, PartialEq, Eq, Clone)]
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
pub(crate) enum ForInitializer {
    Expression(ExpressionIndex),
    Declaration(DeclarationIndex),
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Expression {
    pub(crate) result_type: TypeIndex,
    pub(crate) kind:        ExpressionType,
}

#[derive(Debug, PartialEq, Clone)]
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
        arguments:           VectorSlice<Expression>,
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
pub(crate) enum Constant {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Char(CharacterTokenType),
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
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
    UnexpectedEndOfInput(&'static str),
    ExpectedIdentifierInTypedef(TokenType),
    ExpectedSemicolonAfterTypedef(TokenType),
    ExpectedSemicolonOrOpeningCurlyBraceAfterFunctionDeclaration(TokenType),
    StorageClassRedefinition(StorageClass, TokenType),
    ConstSpecifiedTwice,
    VolatileSpecifiedTwice,
    RestrictSpecifiedTwice,
    InlineSpecifiedTwice,
    StaticSpecifiedTwice,
    ConflictingTypeSpecifiers(TypeSpecifiers, TokenType),
    TypeSpecifierSpecifiedTwice(TokenType),
    LongSpecifiedThrice,
    LongLongDoubleSpecified,
    PointerSpecifiedTwice,
    TypeQualifiersWithoutDeclarator,
    EmptyDeclarationSpecifiers(TokenType),
    NoTypeSpecifiersInDeclarationSpecifiers(TokenType),
    ExpectedClosingCurlyBraceInStructDeclarationList(TokenType),
    MultipleBitfieldWidthsInStructDeclarator,
    EmptyStructDeclarator,
    ExpectedCommaColonOrSemicolonInStructDeclarator(TokenType),
    EnumSpecifierWithoutNameAndBody(TokenType),
    ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(TokenType),
    ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator,
    ExpectedClosingParenthesisAfterParenthesizedDeclarator(Option<TokenType>),
    DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(TokenType),
    BothStaticAndPointerInArrayDirectDeclarator,
    ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(TokenType),
    ExpectedClosingSquareBracket(TokenType),
    PointerAfterAssignmentExpressionInArrayDirectDeclarator,
    ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator,
    AssignmentExpressionAfterPointerInArrayDirectDeclarator,
    TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator,
    TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
    KAndRFunctionDeclaratorMixedWithModernDeclarator,
    ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(TokenType),
    ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(TokenType),
}

impl GetSeverity for ParserErrorType {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | ParserErrorType::UnexpectedEndOfInput(..)
            | ParserErrorType::ExpectedIdentifierInTypedef(..)
            | ParserErrorType::ExpectedSemicolonOrOpeningCurlyBraceAfterFunctionDeclaration(..)
            | ParserErrorType::TypeQualifiersWithoutDeclarator
            | ParserErrorType::EmptyDeclarationSpecifiers(..)
            | ParserErrorType::NoTypeSpecifiersInDeclarationSpecifiers(..)
            | ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(..)
            | ParserErrorType::EmptyStructDeclarator
            | ParserErrorType::ExpectedCommaColonOrSemicolonInStructDeclarator(..)
            | ParserErrorType::EnumSpecifierWithoutNameAndBody(..)
            | ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(..)
            | ParserErrorType::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator
            | ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(..)
            | ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(..)
            | ParserErrorType::BothStaticAndPointerInArrayDirectDeclarator
            | ParserErrorType::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator
            | ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator
            | ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator
            | ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator
            | ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(..)
            | ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(..) =>
                ErrorSeverity::Error,
            | ParserErrorType::ExpectedSemicolonAfterTypedef(..)
            | ParserErrorType::StorageClassRedefinition(..)
            | ParserErrorType::ConstSpecifiedTwice
            | ParserErrorType::VolatileSpecifiedTwice
            | ParserErrorType::RestrictSpecifiedTwice
            | ParserErrorType::InlineSpecifiedTwice
            | ParserErrorType::StaticSpecifiedTwice
            | ParserErrorType::ConflictingTypeSpecifiers(..)
            | ParserErrorType::TypeSpecifierSpecifiedTwice(..)
            | ParserErrorType::LongSpecifiedThrice
            | ParserErrorType::LongLongDoubleSpecified
            | ParserErrorType::PointerSpecifiedTwice
            | ParserErrorType::MultipleBitfieldWidthsInStructDeclarator
            | ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(
                ..,
            )
            | ParserErrorType::ExpectedClosingSquareBracket(..)
            | ParserErrorType::PointerAfterAssignmentExpressionInArrayDirectDeclarator
            | ParserErrorType::AssignmentExpressionAfterPointerInArrayDirectDeclarator =>
                ErrorSeverity::Warning,
        }
    }
}

impl Display for ParserErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | ParserErrorType::UnexpectedEndOfInput(message) =>
                write!(f, "Unexpected end of input while {message}!"),
            | ParserErrorType::ExpectedIdentifierInTypedef(tt) => write!(
                f,
                "Expected an identifier in typedef, found instead {tt:?}!",
            ),
            | ParserErrorType::ExpectedSemicolonOrOpeningCurlyBraceAfterFunctionDeclaration(tt) =>
                write!(
                    f,
                    "Expected a semicolon or an opening curly brace after function declaration, \
                     found instead {tt:?}!",
                ),
            | ParserErrorType::ExpectedSemicolonAfterTypedef(tt) => write!(
                f,
                "Expected a semicolon after typedef, found instead {tt:?}!"
            ),
            | ParserErrorType::StorageClassRedefinition(last, new) =>
                write!(f, "Redefinition of storage class {last:?} with {new:?}!"),
            | ParserErrorType::ConstSpecifiedTwice =>
                write!(f, "`const` keyword specified twice in type declaration!"),
            | ParserErrorType::VolatileSpecifiedTwice =>
                write!(f, "`volatile` keyword specified twice in type declaration!"),
            | ParserErrorType::RestrictSpecifiedTwice =>
                write!(f, "`restrict` keyword specified twice in type declaration!"),
            | ParserErrorType::InlineSpecifiedTwice => write!(
                f,
                "`inline` keyword specified twice in function declaration or definition!"
            ),
            | ParserErrorType::StaticSpecifiedTwice => write!(
                f,
                "`static` keyword specified twice in array direct declarator!"
            ),
            | ParserErrorType::ConflictingTypeSpecifiers(specifiers, tt) =>
                write!(f, "Conflicting type specifiers {specifiers:?} and {tt:?}!"),
            | ParserErrorType::TypeSpecifierSpecifiedTwice(tt) =>
                write!(f, "Type specifier {tt:?} specified twice!"),
            | ParserErrorType::LongSpecifiedThrice =>
                write!(f, "`long` keyword specified thrice in type declaration!"),
            | ParserErrorType::LongLongDoubleSpecified => write!(
                f,
                "`long long` and `double` keywords specified together in type declaration!"
            ),
            | ParserErrorType::PointerSpecifiedTwice =>
                write!(f, "Pointer specified twice in array direct declarator!"),
            | ParserErrorType::TypeQualifiersWithoutDeclarator =>
                write!(f, "Type qualifiers specified without a declarator!"),
            | ParserErrorType::EmptyDeclarationSpecifiers(tt) => write!(
                    f,
                    "Empty declaration specifiers! Got instead: {tt:?}"
                ),
            | ParserErrorType::NoTypeSpecifiersInDeclarationSpecifiers(tt) => write!(f, "No type specifiers in declaration specifiers! Got instead: {tt:?}"),
            | ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(tt) => write!(f,"Expected a closing curly brace in struct declaration list! Got instead: {tt:?}"),
            | ParserErrorType::MultipleBitfieldWidthsInStructDeclarator =>
                write!(f, "Multiple bitfield widths specified in struct declarator!"),
            | ParserErrorType::EmptyStructDeclarator => write!(f, "Empty struct declarator specified in struct declaration!"),
            | ParserErrorType::ExpectedCommaColonOrSemicolonInStructDeclarator(tt) => write!(f, "Expected a comma, colon, or semicolon in struct declarator! Got instead: {tt:?}"),
            | ParserErrorType::EnumSpecifierWithoutNameAndBody(tt) => write!(f, "Enum specifier without name and body! Got instead: {tt:?}"),
            | ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(tt) => write!(f, "Expected an enumeration constant or closing curly brace in enumerator list! Got instead: {tt:?}"),
            | ParserErrorType::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator =>
                write!(
                    f,
                    "Expected a declarator after opening parenthesis in direct declarator!"
                ),
            | ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(tt) =>
                write!(
                    f,
                    "Expected a closing parenthesis after parenthesized declarator! Got instead: \
                     {tt:?}"
                ),
            | ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(tt) =>
                write!(
                    f,
                    "Direct declarator must start with an identifier or an opening parenthesis! \
                     Got instead: {tt:?}"
                ),
            | ParserErrorType::BothStaticAndPointerInArrayDirectDeclarator => write!(
                f,
                "Both `static` and pointer specified in array direct declarator!"
            ),
            | ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(tt,) => write!(
                f,
                "Expected a closing square bracket after pointer in array direct declarator! Got \
                 instead: {tt:?}"
            ),
            | ParserErrorType::ExpectedClosingSquareBracket(tt) =>
                write!(f, "Expected a closing square bracket! Got instead: {tt:?}"),
            | ParserErrorType::PointerAfterAssignmentExpressionInArrayDirectDeclarator => write!(
                f,
                "Pointer specified after assignment expression in array direct declarator!"
            ),
            | ParserErrorType::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator =>
                write!(
                    f,
                    "Expected an assignment expression after `static` in array direct declarator!"
                ),
            | ParserErrorType::AssignmentExpressionAfterPointerInArrayDirectDeclarator => write!(
                f,
                "Assignment expression specified after pointer in array direct declarator!"
            ),
            | ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator =>
                write!(
                    f,
                    "Type qualifiers specified before pointer in array abstract declarator!"
                ),
            | ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator =>
                write!(
                    f,
                    "Type qualifiers specified both before and after `static` in array direct \
                     declarator!"
                ),
            | ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator =>
                write!(
                    f,
                    "K&R function declarator mixed with modern declarator in function declarator!"
                ),
            | ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(tt) => write!(
                f,
                "Expected a comma or closing parenthesis in function declarator parameter list! \
                    Got instead: {tt:?}"
            ),
            | ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(tt) => write!(
                f,
                "Expected a closing parenthesis after ellipsis in function declarator parameter \
                    list! Got instead: {tt:?}"
            ),
        }
    }
}

impl TranslationPhase for Parser {
    type Item = ExternalDeclaration;

    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item> {
        self.parse_external_declaration(context)
    }
}
