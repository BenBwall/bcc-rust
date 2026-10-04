//! Declaration, specifier, declarator, and initializer syntax nodes.

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
    Parser,
    errors::ParserErrorType,
    syntax::{
        ConstantExpressionIndex,
        DesignationIndex,
        EnumSpecifierIndex,
        ExpressionIndex,
        Identifier,
        InitializerIndex,
        ParenthesizedDeclaratorIndex,
        StorageClass,
        StructOrUnionSpecifierIndex,
        SyntaxList,
    },
};
use crate::{
    translation_phases::{
        Context,
        SourceVectors,
        preprocessing::Token,
    },
    util::{
        string_cache::StringCacheId,
        vector_slice::VectorSlice,
    },
};

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
    pub(crate) declaration_specifiers:      DeclarationSpecifiers,
    /// init-declarator-list
    pub(crate) init_declarators:            SyntaxList<InitDeclarator>,
    pub(crate) source_vectors:              SourceVectors,
    /// Whether local syntax recovery repaired this declaration.
    pub(crate) recovered:                   bool,
    /// This declaration-shaped prefix transferred to a function definition
    /// before a declaration semicolon was consumed.
    pub(super) is_function_definition_head: bool,
}

/// init-declarator:
/// - declarator
/// - declarator = initializer
///
/// C99: §6.7, p. 97; PDF p. 109.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct InitDeclarator {
    pub(crate) declarator:     Declarator,
    pub(crate) initializer:    Option<InitializerIndex>,
    pub(crate) source_vectors: SourceVectors,
}

/// initializer:
/// - assignment-expression
/// - { initializer-list }
/// - { initializer-list , }
///
/// C99: §6.7.8, p. 125; PDF p. 137.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Initializer {
    pub(crate) kind: InitializerType,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) opening_brace_source_vectors: Option<SourceVectors>,
    pub(crate) closing_brace_source_vectors: Option<SourceVectors>,
    pub(crate) recovered: bool,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum InitializerType {
    AssignmentExpression(ExpressionIndex),
    InitializerList(SyntaxList<InitializerElement>),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct InitializerElement {
    pub(crate) designation:          Option<DesignationIndex>,
    pub(crate) initializer:          InitializerIndex,
    pub(crate) comma_source_vectors: Option<SourceVectors>,
    pub(crate) source_vectors:       SourceVectors,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Designation {
    pub(crate) designators:           SyntaxList<Designator>,
    pub(crate) equals_source_vectors: Option<SourceVectors>,
    pub(crate) source_vectors:        SourceVectors,
    pub(crate) recovered:             bool,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Designator {
    pub(crate) kind: DesignatorType,
    pub(crate) operator_source_vectors: SourceVectors,
    pub(crate) closing_bracket_source_vectors: Option<SourceVectors>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered: bool,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum DesignatorType {
    Array(ConstantExpressionIndex),
    Field(Identifier),
    Error,
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
/// - struct-or-union-specifier
/// - enum-specifier
/// - typedef-name
///
/// Core variants represent valid combinations of type specifiers. For example,
/// the declaration `unsigned int foo` is represented as
/// `TypeSpecifiers::UnsignedInt`. `_Imaginary` is diagnosed before this model
/// is constructed and therefore has no valid-looking type variant.
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
            | TypeSpecifiers::Void => write!(f, "void"),
            | TypeSpecifiers::StructOrUnion(_) => write!(f, "<struct-or-union-declarator>"),
            | TypeSpecifiers::Enum(_) => write!(f, "<enum-declarator>"),
            | TypeSpecifiers::TypedefName(_) => write!(f, "<typedef-name>"),
        }
    }
}

macro_rules! map_fn {
    ($map_fn_name:ident, $make_fn_name:ident, $is_fn_name:ident, $is_match_pattern:pat $(if $is_guard:expr)?, $($map_match_pattern:pat $(if $map_guard:expr)? => $map_result:expr $(,)?)+) => {
        pub(super) fn $is_fn_name(self) -> bool {
            match self {
                $is_match_pattern $(if $is_guard)? => true,
                | _ => false,
            }
        }

        /// Returns `None` when the keyword conflicts with the accumulated
        /// specifiers.
        fn $map_fn_name(
            self,
        ) -> Option<Self> {
            match self {
                $($map_match_pattern $(if $map_guard)? => Some($map_result),)+
                | _ => None,
            }
        }

        pub(super) fn $make_fn_name(
            &mut self,
            parser: &mut Parser,
            context: &mut Context<'_>,
            token: Token,
        ) {
            match self.$map_fn_name() {
                | Some(mapped) => *self = mapped,
                | None => self.report_conflict(parser, context, token.contents, token),
            }
        }
    }
}

impl TypeSpecifiers {
    map_fn!(map_signed, make_signed, is_signed,
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

    map_fn!(map_unsigned, make_unsigned, is_unsigned,
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

    map_fn!(map_int, make_int, is_int,
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
        | TypeSpecifiers::SignedShort => TypeSpecifiers::SignedShortInt,
        | TypeSpecifiers::UnsignedShort => TypeSpecifiers::UnsignedShortInt,
        | TypeSpecifiers::Signed => TypeSpecifiers::SignedInt,
        | TypeSpecifiers::Unsigned => TypeSpecifiers::UnsignedInt,
        | TypeSpecifiers::Long => TypeSpecifiers::LongInt,
        | TypeSpecifiers::SignedLong => TypeSpecifiers::SignedLongInt,
        | TypeSpecifiers::UnsignedLong => TypeSpecifiers::UnsignedLongInt,
        | TypeSpecifiers::LongLong => TypeSpecifiers::LongLongInt,
        | TypeSpecifiers::SignedLongLong => TypeSpecifiers::SignedLongLongInt,
        | TypeSpecifiers::UnsignedLongLong => TypeSpecifiers::UnsignedLongLongInt,
    );

    map_fn!(map_short, make_short, is_short,
        | TypeSpecifiers::Short
        | TypeSpecifiers::SignedShort
        | TypeSpecifiers::UnsignedShort
        | TypeSpecifiers::ShortInt
        | TypeSpecifiers::SignedShortInt
        | TypeSpecifiers::UnsignedShortInt,

        | TypeSpecifiers::Empty => TypeSpecifiers::Short,
        | TypeSpecifiers::Int => TypeSpecifiers::ShortInt,
        | TypeSpecifiers::SignedInt => TypeSpecifiers::SignedShortInt,
        | TypeSpecifiers::UnsignedInt => TypeSpecifiers::UnsignedShortInt,
        | TypeSpecifiers::Signed => TypeSpecifiers::SignedShort,
        | TypeSpecifiers::Unsigned => TypeSpecifiers::UnsignedShort,
    );

    map_fn!(map_long, make_long, is_long,
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
        | TypeSpecifiers::ComplexLongDouble,

        | TypeSpecifiers::Empty => TypeSpecifiers::Long,
        | TypeSpecifiers::Int => TypeSpecifiers::LongInt,
        | TypeSpecifiers::SignedInt => TypeSpecifiers::SignedLongInt,
        | TypeSpecifiers::UnsignedInt => TypeSpecifiers::UnsignedLongInt,
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
    );

    map_fn!(map_char, make_char, is_char,
        | TypeSpecifiers::Char
        | TypeSpecifiers::SignedChar
        | TypeSpecifiers::UnsignedChar,

        | TypeSpecifiers::Empty => TypeSpecifiers::Char,
        | TypeSpecifiers::Signed => TypeSpecifiers::SignedChar,
        | TypeSpecifiers::Unsigned => TypeSpecifiers::UnsignedChar,
    );

    map_fn!(map_float, make_float, is_float,
        | TypeSpecifiers::Float
        | TypeSpecifiers::ComplexFloat,

        | TypeSpecifiers::Empty => TypeSpecifiers::Float,
        | TypeSpecifiers::Complex => TypeSpecifiers::ComplexFloat,
    );

    map_fn!(map_double, make_double, is_double,
        | TypeSpecifiers::Double
        | TypeSpecifiers::ComplexDouble
        | TypeSpecifiers::ComplexLongDouble
        | TypeSpecifiers::LongDouble,

        | TypeSpecifiers::Empty => TypeSpecifiers::Double,
        | TypeSpecifiers::Complex => TypeSpecifiers::ComplexDouble,
        | TypeSpecifiers::ComplexLong => TypeSpecifiers::ComplexLongDouble,
        | TypeSpecifiers::Long => TypeSpecifiers::LongDouble
    );

    map_fn!(map_void, make_void, is_void,
        | TypeSpecifiers::Void,

        | TypeSpecifiers::Empty => TypeSpecifiers::Void,
    );

    map_fn!(map_bool, make_bool, is_bool,
        | TypeSpecifiers::Bool,

        | TypeSpecifiers::Empty => TypeSpecifiers::Bool,
    );

    map_fn!(map_complex, make_complex, is_complex,
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

    pub(super) fn is_long_long(self) -> bool {
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

    pub(super) fn is_long_double(self) -> bool {
        match self {
            | TypeSpecifiers::LongDouble | TypeSpecifiers::ComplexLongDouble => true,
            | _ => false,
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Retained for the future semantic type builder.")
    )]
    pub(super) fn is_struct_or_union(self) -> bool {
        match self {
            | TypeSpecifiers::StructOrUnion(_) => true,
            | _ => false,
        }
    }

    pub(super) fn make_struct_or_union(
        &mut self,
        parser: &mut Parser,
        context: &mut Context<'_>,
        index: StructOrUnionSpecifierIndex,
        token: Token,
    ) {
        match self {
            | TypeSpecifiers::Empty => *self = TypeSpecifiers::StructOrUnion(index),
            | _ => self.report_conflict(parser, context, token.contents, token),
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Retained for the future semantic type builder.")
    )]
    pub(super) fn is_enum(self) -> bool {
        match self {
            | TypeSpecifiers::Enum(_) => true,
            | _ => false,
        }
    }

    pub(super) fn make_enum(
        &mut self,
        parser: &mut Parser,
        context: &mut Context<'_>,
        index: EnumSpecifierIndex,
        token: Token,
    ) {
        match self {
            | TypeSpecifiers::Empty => *self = TypeSpecifiers::Enum(index),
            | _ => self.report_conflict(parser, context, token.contents, token),
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Retained for the future semantic type builder.")
    )]
    pub(super) fn is_typedef_name(self) -> bool {
        match self {
            | TypeSpecifiers::TypedefName(_) => true,
            | _ => false,
        }
    }

    pub(super) fn make_typedef_name(
        &mut self,
        parser: &mut Parser,
        context: &mut Context<'_>,
        name: Identifier,
        token: Token,
    ) {
        match self {
            | TypeSpecifiers::Empty => *self = TypeSpecifiers::TypedefName(name),
            | _ => self.report_conflict(parser, context, name.name, token),
        }
    }

    /// Reports `conflicting` against the accumulated specifiers using
    /// source spellings, so rendered diagnostics never expose arena handles.
    fn report_conflict(
        self,
        parser: &mut Parser,
        context: &mut Context<'_>,
        conflicting: StringCacheId,
        token: Token,
    ) {
        let tagged = |keyword: &str, tag: Option<Identifier>| -> Box<str> {
            match tag {
                | Some(tag) => format!("{keyword} {}", context.string_cache.at(tag.name)).into(),
                | None => format!("{keyword} {{...}}").into(),
            }
        };
        let existing = match self {
            | TypeSpecifiers::StructOrUnion(index) => {
                let specifier = parser.syntax[index];
                let keyword = match specifier.struct_or_union {
                    | StructOrUnion::Struct => "struct",
                    | StructOrUnion::Union => "union",
                };
                tagged(keyword, specifier.identifier)
            },
            | TypeSpecifiers::Enum(index) => tagged("enum", parser.syntax[index].name),
            | TypeSpecifiers::TypedefName(name) => context.string_cache.at(name.name).into(),
            | type_specifiers => type_specifiers.to_string().into_boxed_str(),
        };
        let error_type = ParserErrorType::ConflictingTypeSpecifiers {
            existing,
            conflicting: context.string_cache.at(conflicting).into(),
        };
        parser.report(context, error_type, Some(token));
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
    pub(crate) struct_declaration_list: Option<SyntaxList<StructDeclaration>>,
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
    pub(crate) type_qualifiers:        TypeQualifiers,
    pub(crate) type_specifiers:        TypeSpecifiers,
    pub(crate) struct_declarator_list: SyntaxList<StructDeclarator>,
    pub(crate) source_vectors:         SourceVectors,
}

/// struct-declarator:
/// - declarator
/// - declarator? : constant-expression
///
/// C99: §6.7.2.1, p. 101; PDF p. 113.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct StructDeclarator {
    pub(crate) declarator:     Option<Declarator>,
    pub(crate) bitfield_width: Option<ConstantExpressionIndex>,
    pub(crate) source_vectors: SourceVectors,
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
    pub(crate) enumeration_list: Option<SyntaxList<Enumerator>>,
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
    /// `None` preserves the grammatical absence of a storage-class specifier;
    /// it is not equivalent to an explicitly written `auto`.
    pub(crate) storage_class:       Option<StorageClass>,
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
    pub(super) fn new() -> Self {
        Self {
            storage_class:       None,
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
    pub(crate) type_qualifiers_list: SyntaxList<TypeQualifiers>,
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
    pub(crate) kind:           SyntaxList<DirectDeclarator>,
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
    Parenthesized(ParenthesizedDeclaratorIndex),
    KAndRStyleFunction {
        parameters: SyntaxList<Identifier>,
    },
    Array {
        type_qualifiers:       TypeQualifiers,
        is_static:             bool,
        is_pointer:            bool,
        assignment_expression: Option<ExpressionIndex>,
    },
    Function {
        parameter_list: SyntaxList<ParameterDeclaration>,
        is_variadic:    bool,
    },
}

/// Grouping syntax lives in the arena so its child and delimiter span do not
/// enlarge every direct-declarator variant.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct ParenthesizedDeclarator {
    pub(crate) declarator: Declarator,
    /// Only the parentheses; child provenance stays on the child.
    pub(crate) delimiters: SourceVectors,
}

/// parameter-declaration:
/// - declaration-specifiers declarator
/// - declaration-specifiers abstract-declarator?
///
/// C99: §6.7.5, p. 114; PDF p. 126, and function declarators §6.7.5.3,
/// pp. 118-121; PDF pp. 130-133.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ParameterDeclaration {
    pub(crate) declaration_specifiers: DeclarationSpecifiers,
    /// Could be a declarator or an abstract declarator or neither.
    pub(crate) declarator:             Option<Declarator>,
    pub(crate) source_vectors:         SourceVectors,
}

// The Phase 03 parser machine is implemented below the retained syntax model.

/// A parsed type-name syntax node.
///
/// C99: §6.7.6, p. 122; PDF p. 134.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TypeName {
    /// Specifiers and qualifiers that establish the base type.
    pub(crate) declaration_specifiers: DeclarationSpecifiers,
    /// Optional abstract declarator deriving pointer, array, or function shape.
    pub(crate) declarator:             Option<Declarator>,
    pub(crate) source_vectors:         SourceVectors,
    pub(crate) recovered:              bool,
}
