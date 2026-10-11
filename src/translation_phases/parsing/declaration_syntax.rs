//! Declaration, specifier, declarator, and initializer syntax nodes.
//!
//! The retained syntax-tree shapes that translation phase 7 (§5.1.1.2,
//! p. 10; PDF p. 22) builds for declarations. C99: declarations §6.7,
//! pp. 97-98; PDF pp. 109-110; declaration specifiers §6.7.1-§6.7.4,
//! pp. 98-113; PDF pp. 110-125; declarators §6.7.5-§6.7.5.3, pp. 114-121;
//! PDF pp. 126-133; type names §6.7.6, p. 122; PDF p. 134; initializers
//! §6.7.8, pp. 125-130; PDF pp. 137-142; summarized in §A.2.2,
//! pp. 411-415; PDF pp. 423-427.
//!
//! Nodes record grammatical form. `TypeSpecifiers` folds the type-specifier
//! keywords into one of the sets of §6.7.2 paragraph 2 as they are read;
//! types, linkage (§6.2.2, pp. 30-31; PDF pp. 42-43), storage duration
//! (§6.2.4, p. 32; PDF p. 44), completeness, and the other constraints and
//! semantics of §6.7-§6.7.8 belong to semantic analysis.

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
        ConstantExpression,
        Expression,
        Identifier,
        StorageClass,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::Token,
    },
    util::{
        arena_list::ArenaList,
        bump::ArenaVec,
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
pub(crate) struct Declaration<'tu> {
    pub(crate) assertion:                   Option<&'tu super::modern::StaticAssertion<'tu>>,
    pub(crate) declaration_specifiers:      DeclarationSpecifiers<'tu>,
    /// init-declarator-list
    pub(crate) init_declarators:            ArenaList<'tu, InitDeclarator<'tu>>,
    pub(crate) source_vectors:              SourceVectors,
    /// Whether local syntax recovery repaired this declaration.
    pub(crate) recovered:                   bool,
    /// This declaration-shaped prefix transferred to a function definition
    /// before a declaration semicolon was consumed.
    pub(super) is_function_definition_head: bool,
}

impl<'tu> TypeSpecifiers<'tu> {
    /// Reports `conflicting` against the accumulated specifiers using
    /// source spellings, so rendered diagnostics never expose syntax nodes.
    #[cold]
    #[inline(never)]
    pub(super) fn report_conflict(
        self,
        parser: &mut Parser<'_, 'tu, '_>,
        conflicting: StringCacheId,
        token: Token,
    ) {
        let tagged = |keyword: &str, tag: Option<Identifier>| -> &'tu str {
            match tag {
                | Some(tag) => parser.context.diagnostic_format(format_args!(
                    "{keyword} {}",
                    parser.context.string_cache.at(tag.name)
                )),
                | None => parser
                    .context
                    .diagnostic_format(format_args!("{keyword} {{...}}")),
            }
        };
        let existing = match self {
            | TypeSpecifiers::StructOrUnion(index) => {
                let specifier = *index;
                let keyword = match specifier.struct_or_union {
                    | StructOrUnion::Struct => "struct",
                    | StructOrUnion::Union => "union",
                };
                tagged(keyword, specifier.identifier)
            },
            | TypeSpecifiers::Enum(index) => tagged("enum", index.name),
            | TypeSpecifiers::TypedefName(name) => parser
                .context
                .diagnostic_text(parser.context.string_cache.at(name.name)),
            | type_specifiers => parser
                .context
                .diagnostic_format(format_args!("{type_specifiers}")),
        };
        let error_type = ParserErrorType::ConflictingTypeSpecifiers {
            existing,
            conflicting: parser
                .context
                .diagnostic_text(parser.context.string_cache.at(conflicting)),
        };
        parser.report(error_type, Some(token));
    }
}

impl TypeSpecifiers<'_> {
    /// Passes `bind` the ordinary identifiers these specifiers declare, the
    /// enumeration constants of nested enum bodies. `pending` is empty scan
    /// storage, reused across calls and left empty.
    ///
    /// C99: enumeration constants are ordinary identifiers, §6.2.3
    /// paragraph 1, p. 31; PDF p. 43, even inside a member declaration, whose
    /// member names live in the structure's own name space.
    pub(super) fn collect_bindings(
        self,
        pending: &mut ArenaVec<'_, Self>,
        mut bind: impl FnMut(StringCacheId),
    ) {
        debug_assert!(pending.is_empty(), "binding scan storage starts empty");
        pending.push(self);
        while let Some(type_specifiers) = pending.pop() {
            match type_specifiers {
                | TypeSpecifiers::Enum(specifier) => {
                    if let Some(enumeration_list) = specifier.enumeration_list {
                        for enumerator in enumeration_list {
                            bind(enumerator.name.name);
                        }
                    }
                },
                | TypeSpecifiers::StructOrUnion(specifier) => {
                    if let Some(declarations) = specifier.struct_declaration_list {
                        pending.extend(
                            declarations
                                .iter()
                                .map(|declaration| declaration.type_specifiers),
                        );
                    }
                },
                | _ => {},
            }
        }
    }
}

/// init-declarator:
/// - declarator
/// - declarator = initializer
///
/// C99: §6.7 paragraph 1, p. 97; PDF p. 109. The §6.7.8 paragraph 5
/// constraint on block-scope identifiers with linkage, p. 125; PDF p. 137,
/// is left to semantic analysis.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct InitDeclarator<'tu> {
    pub(crate) declarator:     Declarator<'tu>,
    pub(crate) initializer:    Option<&'tu Initializer<'tu>>,
    pub(crate) source_vectors: SourceVectors,
}

/// initializer:
/// - assignment-expression
/// - { initializer-list }
/// - { initializer-list , }
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137. The constraints of
/// paragraphs 2-7, p. 125; PDF p. 137, and the initialization semantics of
/// paragraphs 8-23, pp. 126-128; PDF pp. 138-140, are left to semantic
/// analysis.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Initializer<'tu> {
    pub(crate) kind:           InitializerType<'tu>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

/// The two `initializer` alternatives: an `assignment-expression` or a
/// braced `initializer-list`.
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum InitializerType<'tu> {
    AssignmentExpression(&'tu Expression<'tu>),
    InitializerList(&'tu BracedInitializerList<'tu>),
}

/// `{ initializer-list }` or `{ initializer-list , }`: the elements and the
/// braces around them. Only a braced list has braces, so their locations
/// live here rather than in every [`Initializer`].
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137; §A.2.2, p. 414; PDF p. 426.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct BracedInitializerList<'tu> {
    pub(crate) elements:                     ArenaList<'tu, InitializerElement<'tu>>,
    pub(crate) opening_brace_source_vectors: Option<SourceVectors>,
    /// `None` when recovery found the closing brace missing.
    pub(crate) closing_brace_source_vectors: Option<SourceVectors>,
}

/// One `designation? initializer` element of an `initializer-list`.
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct InitializerElement<'tu> {
    pub(crate) designation:          Option<&'tu Designation<'tu>>,
    pub(crate) initializer:          &'tu Initializer<'tu>,
    pub(crate) comma_source_vectors: Option<SourceVectors>,
    pub(crate) source_vectors:       SourceVectors,
}

/// designation:
/// - designator-list =
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137. Resolving the current
/// object a designator list names (paragraphs 17-18, pp. 126-127;
/// PDF pp. 138-139) is semantic analysis.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Designation<'tu> {
    pub(crate) designators:           ArenaList<'tu, Designator<'tu>>,
    pub(crate) equals_source_vectors: Option<SourceVectors>,
    pub(crate) source_vectors:        SourceVectors,
    pub(crate) recovered:             bool,
}

/// designator:
/// - [ constant-expression ]
/// - . identifier
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137; §A.2.2, p. 415; PDF p. 427.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Designator<'tu> {
    pub(crate) kind: DesignatorType<'tu>,
    pub(crate) operator_source_vectors: SourceVectors,
    pub(crate) closing_bracket_source_vectors: Option<SourceVectors>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered: bool,
}

/// The `designator` alternatives, plus an error node for recovery.
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137. That an array designator is
/// an integer constant expression for an array object (paragraph 6) and a
/// field designator names a member (paragraph 7), p. 125; PDF p. 137, is
/// left to semantic analysis.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum DesignatorType<'tu> {
    GnuField(Identifier),
    Range(&'tu RangeDesignator<'tu>),
    Array(ConstantExpression<'tu>),
    Field(Identifier),
    Error,
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
/// specifier shall be given” (§6.7.2 paragraph 2, p. 99; PDF p. 111). The
/// core variants are the multisets that paragraph 2 lists, pp. 99-100;
/// PDF pp. 111-112, kept distinct per spelling although paragraph 5, p. 100;
/// PDF p. 112, makes each comma-separated set designate one type.
/// `Complex` and `ComplexLong` are incomplete intermediate states that the
/// specifier frame diagnoses if the list ends in them.
#[derive(Debug, PartialEq, Clone, Copy, Default)]
pub(crate) enum TypeSpecifiers<'tu> {
    #[default]
    Empty,
    Extended(&'tu super::modern::ExtendedType<'tu>),
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
    StructOrUnion(&'tu StructOrUnionSpecifier<'tu>),
    Enum(&'tu EnumSpecifier<'tu>),
    TypedefName(Identifier),
}

/// struct-or-union-specifier:
/// - struct-or-union identifier? { struct-declaration-list }
/// - struct-or-union identifier
///
/// C99: §6.7.2.1 paragraph 1, p. 101; PDF p. 113. A tag-only specifier
/// declares or refers to a tag under §6.7.2.3 paragraphs 7-9, p. 107;
/// PDF p. 119; tag resolution, completeness, and the member constraints of
/// §6.7.2.1 paragraphs 2-4, p. 101; PDF p. 113, are left to semantic
/// analysis.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct StructOrUnionSpecifier<'tu> {
    pub(crate) attributes:              Option<&'tu super::modern::SpecifierExtension<'tu>>,
    pub(crate) struct_or_union:         StructOrUnion,
    pub(crate) identifier:              Option<Identifier>,
    /// None indicates that the body is missing. An empty vector indicates an
    /// empty body. `struct Foo;` has no body. `struct Foo {};` has an empty
    /// body.
    pub(crate) struct_declaration_list: Option<ArenaList<'tu, StructDeclaration<'tu>>>,
    pub(crate) source_vectors:          SourceVectors,
}

/// struct-or-union:
/// - struct
/// - union
///
/// C99: §6.7.2.1 paragraph 1, p. 101; PDF p. 113.
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
/// C99: §6.7.2.1 paragraph 1, p. 101; PDF p. 113.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct StructDeclaration<'tu> {
    pub(crate) assertion:              Option<&'tu super::modern::StaticAssertion<'tu>>,
    pub(crate) extensions:             Option<&'tu super::modern::SpecifierExtension<'tu>>,
    pub(crate) type_qualifiers:        TypeQualifiers,
    pub(crate) type_specifiers:        TypeSpecifiers<'tu>,
    pub(crate) struct_declarator_list: ArenaList<'tu, StructDeclarator<'tu>>,
    pub(crate) source_vectors:         SourceVectors,
}

/// struct-declarator:
/// - declarator
/// - declarator? : constant-expression
///
/// C99: §6.7.2.1 paragraph 1, p. 101; PDF p. 113. The bit-field width and
/// type constraints of paragraphs 3-4, p. 101; PDF p. 113, are left to
/// semantic analysis.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct StructDeclarator<'tu> {
    pub(crate) attributes:     Option<&'tu super::modern::SpecifierExtension<'tu>>,
    pub(crate) declarator:     Option<Declarator<'tu>>,
    pub(crate) bitfield_width: Option<ConstantExpression<'tu>>,
    pub(crate) source_vectors: SourceVectors,
}

/// enum-specifier:
/// - enum identifier? { enumerator-list }
/// - enum identifier? { enumerator-list , }
/// - enum identifier
///
/// C99: §6.7.2.2 paragraph 1, p. 105; PDF p. 117; tags §6.7.2.3, pp. 106-107;
/// PDF pp. 118-119. The §6.7.2.3 paragraph 3 rule that a bare `enum
/// identifier` follow the complete type, p. 106; PDF p. 118, is left to
/// semantic analysis.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct EnumSpecifier<'tu> {
    pub(crate) underlying_type:  Option<&'tu TypeName<'tu>>,
    pub(crate) attributes:       Option<&'tu super::modern::SpecifierExtension<'tu>>,
    pub(crate) name:             Option<Identifier>,
    pub(crate) enumeration_list: Option<ArenaList<'tu, Enumerator<'tu>>>,
    pub(crate) source_vectors:   SourceVectors,
}

/// enumerator:
/// - enumeration-constant
/// - enumeration-constant = constant-expression
///
/// C99: §6.7.2.2 paragraph 1, p. 105; PDF p. 117. The paragraph 2 value
/// constraint and paragraph 3 value assignment, p. 105; PDF p. 117, are left
/// to semantic analysis.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Enumerator<'tu> {
    pub(crate) attributes:     Option<&'tu super::modern::SpecifierExtension<'tu>>,
    pub(crate) name:           Identifier,
    pub(crate) expression:     Option<ConstantExpression<'tu>>,
    pub(crate) source_vectors: SourceVectors,
}

/// function-specifier:
/// - inline
///
/// C99: §6.7.4 paragraph 1, p. 112; PDF p. 124. A flag suffices because a
/// repeated `inline` behaves as if it appeared once (paragraph 5, p. 112;
/// PDF p. 124). The constraints of paragraphs 2-4, p. 112; PDF p. 124, are
/// left to semantic analysis.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Default)]
pub(crate) struct FunctionSpecifiers {
    pub(crate) is_noreturn: bool,
    pub(crate) is_inline:   bool,
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
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct DeclarationSpecifiers<'tu> {
    pub(crate) implicit_int:            bool,
    pub(crate) extensions:              Option<&'tu super::modern::SpecifierExtension<'tu>>,
    /// `None` preserves the grammatical absence of a storage-class specifier;
    /// it is not equivalent to an explicitly written `auto`.
    pub(crate) storage_class:           Option<StorageClass>,
    /// Whether `auto` was written beside the specifier in `storage_class`.
    ///
    /// C23 (N3220): §6.7.2 paragraph 2, p. 99; PDF p. 112 lets `auto`
    /// appear with every other storage-class specifier except `typedef`.
    /// Paragraph 4 restricts that pairing to inferred types, a constraint
    /// left to semantic analysis.
    pub(crate) auto_with_storage_class: bool,
    pub(crate) type_qualifiers:         TypeQualifiers,
    pub(crate) type_specifiers:         TypeSpecifiers<'tu>,
    pub(crate) function_specifiers:     FunctionSpecifiers,
    pub(crate) source_vectors:          SourceVectors,
}

/// pointer:
/// - \* type-qualifier-list?
/// - \* type-qualifier-list? pointer
///
/// Each element in `levels` is one level of indirection, outermost first. For
/// example, `*const *volatile *x` has the qualifiers `[TypeQualifiers::CONST,
/// TypeQualifiers::VOLATILE, TypeQualifiers::empty()]`.
///
/// C99: §6.7.5 paragraph 1, p. 114; PDF p. 126, and pointer derivation
/// §6.7.5.1 paragraph 1, p. 115; PDF p. 127.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct PointerDeclarator<'tu> {
    pub(crate) levels: ArenaList<'tu, PointerLevel<'tu>>,
}

/// One `*` with the qualifiers and attributes written after it.
///
/// C99: §6.7.5.1 paragraph 1, p. 115; PDF p. 127. C23: the attributes after
/// a `*` appertain to that pointer, §6.7.7.2 paragraph 1, p. 127;
/// PDF p. 140. GNU and MSVC attributes in the same place share it.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct PointerLevel<'tu> {
    pub(crate) qualifiers: TypeQualifiers,
    /// Attribute specifiers in reverse source order, as in
    /// [`super::modern::SpecifierExtension`] chains.
    pub(crate) attributes: Option<&'tu super::modern::SpecifierExtension<'tu>>,
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
/// C99: declarators are §6.7.5 paragraph 1, p. 114; PDF p. 126. Abstract
/// declarators are §6.7.6 paragraph 1, p. 122; PDF p. 134.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Declarator<'tu> {
    pub(crate) pointer:        PointerDeclarator<'tu>,
    pub(crate) kind:           ArenaList<'tu, DirectDeclarator<'tu>>,
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
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum DirectDeclarator<'tu> {
    /// MSVC calling convention / modifier at this declarator position.
    MsModifier(
        crate::translation_phases::preprocessing::KeywordTokenType,
        SourceVectors,
    ),
    AsmLabel(&'tu super::gnu::Asm<'tu>),
    Attributes(&'tu super::modern::AttributeSpecifier<'tu>),
    Identifier(Identifier),
    Parenthesized(&'tu ParenthesizedDeclarator<'tu>),
    /// `( identifier-list? )`: C99 §6.7.5.3 paragraph 14, p. 119; PDF p. 131.
    /// The paragraph 3 rule that a non-empty list appear only in a
    /// definition, p. 118; PDF p. 130, is left to semantic analysis.
    KAndRStyleFunction {
        parameters: ArenaList<'tu, Identifier>,
    },
    /// The four `[...]` suffixes: C99 §6.7.5.2 paragraph 3, p. 116;
    /// PDF p. 128. `is_pointer` records `[*]`. The paragraph 1-2 constraints,
    /// p. 116; PDF p. 128, are left to semantic analysis.
    Array {
        type_qualifiers:       TypeQualifiers,
        is_static:             bool,
        is_pointer:            bool,
        assignment_expression: Option<&'tu Expression<'tu>>,
    },
    /// `( parameter-type-list )`, or an empty list: C99 §6.7.5.3
    /// paragraph 5, p. 118; PDF p. 130; §6.7.6 paragraph 1, p. 122;
    /// PDF p. 134.
    Function {
        parameter_list: ArenaList<'tu, ParameterDeclaration<'tu>>,
        is_variadic:    bool,
    },
}

/// Grouping syntax lives in the arena so its child and delimiter span do not
/// enlarge every direct-declarator variant.
///
/// C99: `( declarator )` binds as the unparenthesized declarator, §6.7.5
/// paragraph 6, p. 115; PDF p. 127; `( abstract-declarator )` is §6.7.6
/// paragraph 1, p. 122; PDF p. 134.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct ParenthesizedDeclarator<'tu> {
    pub(crate) declarator: Declarator<'tu>,
    /// Only the parentheses; child provenance stays on the child.
    pub(crate) delimiters: SourceVectors,
}

/// parameter-declaration:
/// - declaration-specifiers declarator
/// - declaration-specifiers abstract-declarator?
///
/// C99: §6.7.5, p. 114; PDF p. 126, and function declarators §6.7.5.3,
/// pp. 118-121; PDF pp. 130-133. The paragraph 2 storage-class constraint,
/// p. 118; PDF p. 130, and the adjustments of paragraphs 7-8, p. 119;
/// PDF p. 131, are left to semantic analysis.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct ParameterDeclaration<'tu> {
    pub(crate) declaration_specifiers: DeclarationSpecifiers<'tu>,
    /// Could be a declarator or an abstract declarator or neither.
    pub(crate) declarator:             Option<Declarator<'tu>>,
    pub(crate) source_vectors:         SourceVectors,
}

/// A parsed type-name syntax node.
///
/// type-name:
/// - specifier-qualifier-list abstract-declarator?
///
/// C99: §6.7.6 paragraph 1, p. 122; PDF p. 134. The
/// `specifier-qualifier-list` (§6.7.2.1 paragraph 1, p. 101; PDF p. 113)
/// reuses [`DeclarationSpecifiers`], whose storage-class and function
/// specifiers stay empty.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct TypeName<'tu> {
    /// Specifiers and qualifiers that establish the base type.
    pub(crate) declaration_specifiers: DeclarationSpecifiers<'tu>,
    /// Optional abstract declarator deriving pointer, array, or function shape.
    pub(crate) declarator:             Option<Declarator<'tu>>,
    pub(crate) source_vectors:         SourceVectors,
    pub(crate) recovered:              bool,
}

/// GNU inclusive array designator range; evaluation belongs to later analysis.
/// C99: extension to §6.7.8, p. 125; PDF p. 137.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct RangeDesignator<'tu> {
    pub(crate) lower: ConstantExpression<'tu>,
    pub(crate) upper: ConstantExpression<'tu>,
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
            parser: &mut Parser<'_, 'tu, '_>,
            token: Token,
        ) {
            match self.$map_fn_name() {
                | Some(mapped) => *self = mapped,
                | None => self.report_conflict(parser, token.contents, token),
            }
        }
    }
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
    /// C99: §6.7.3 paragraph 1, p. 108; PDF p. 120. A set suffices because a
    /// repeated qualifier behaves as if it appeared once (paragraph 4,
    /// p. 108; PDF p. 120). The `restrict` constraint of paragraph 2,
    /// p. 108; PDF p. 120, is left to semantic analysis.
    pub(crate) struct TypeQualifiers: u16 {
        const CONST = 1 << 0;
        const VOLATILE = 1 << 1;
        const RESTRICT = 1 << 2;
        const ATOMIC = 1 << 3;
        const PTR32 = 1 << 4;
        const PTR64 = 1 << 5;
        const UNALIGNED = 1 << 6;
        const W64 = 1 << 7;
        const SPTR = 1 << 8;
        const UPTR = 1 << 9;
    }
}

impl<'tu> TypeSpecifiers<'tu> {
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

    pub(super) fn make_struct_or_union(
        &mut self,
        parser: &mut Parser<'_, 'tu, '_>,
        index: &'tu StructOrUnionSpecifier<'tu>,
        token: Token,
    ) {
        match self {
            | TypeSpecifiers::Empty => *self = TypeSpecifiers::StructOrUnion(index),
            | _ => self.report_conflict(parser, token.contents, token),
        }
    }

    pub(super) fn make_enum(
        &mut self,
        parser: &mut Parser<'_, 'tu, '_>,
        index: &'tu EnumSpecifier<'tu>,
        token: Token,
    ) {
        match self {
            | TypeSpecifiers::Empty => *self = TypeSpecifiers::Enum(index),
            | _ => self.report_conflict(parser, token.contents, token),
        }
    }

    pub(super) fn make_typedef_name(
        &mut self,
        parser: &mut Parser<'_, 'tu, '_>,
        name: Identifier,
        token: Token,
    ) {
        match self {
            | TypeSpecifiers::Empty => *self = TypeSpecifiers::TypedefName(name),
            | _ => self.report_conflict(parser, name.name, token),
        }
    }
}

impl DeclarationSpecifiers<'_> {
    /// The storage-class specifiers as written: `none`, one keyword, or a
    /// C23 pairing such as `auto static`.
    /// Recovered `auto` and `typedef` use their own spelling even if the
    /// pairing flag is set.
    pub(super) fn storage_spelling(&self) -> &'static str {
        match (self.storage_class, self.auto_with_storage_class) {
            | (None, _) => "none",
            | (Some(StorageClass::Register), true) => "auto register",
            | (Some(StorageClass::Static), true) => "auto static",
            | (Some(StorageClass::Extern), true) => "auto extern",
            | (Some(class), _) => class.spelling(),
        }
    }

    pub(super) fn new() -> Self {
        Self {
            extensions:              None,
            implicit_int:            false,
            storage_class:           None,
            auto_with_storage_class: false,
            type_qualifiers:         TypeQualifiers::empty(),
            type_specifiers:         TypeSpecifiers::Empty,
            function_specifiers:     FunctionSpecifiers {
                is_inline:   false,
                is_noreturn: false,
            },
            source_vectors:          VectorSlice::empty(),
        }
    }
}

impl<'tu> Declaration<'tu> {
    /// The declarator of a declaration that could head a function
    /// definition: its only init-declarator, without an initializer.
    ///
    /// C99: `function-definition` takes one `declarator` and no initializer,
    /// §6.9.1 paragraph 1, p. 141; PDF p. 153.
    pub(super) fn head_declarator(&self) -> Option<Declarator<'tu>> {
        let [init] = self.init_declarators.as_slice() else {
            return None;
        };
        init.initializer.is_none().then_some(init.declarator)
    }

    /// Whether this declaration-shaped prefix became the head of a function
    /// definition.
    pub(super) fn is_definition_head(&self) -> bool {
        self.is_function_definition_head && self.head_declarator().is_some()
    }

    /// Whether any part of this declaration was written, as opposed to a
    /// declaration that recovery produced from nothing.
    pub(super) fn is_meaningful(&self) -> bool {
        let specifiers = self.declaration_specifiers;
        !self.init_declarators.is_empty()
            || specifiers.storage_class.is_some()
            || !specifiers.type_qualifiers.is_empty()
            || specifiers.type_specifiers != TypeSpecifiers::Empty
            || specifiers.function_specifiers.is_inline
    }
}

impl<'tu> Declarator<'tu> {
    /// Finds the identifier declared by nested parenthesized direct
    /// declarators.
    ///
    /// C99: each declarator declares one identifier, §6.7.5 paragraph 2,
    /// p. 114; PDF p. 126, and a parenthesized declarator binds as the
    /// unparenthesized one, paragraph 6, p. 115; PDF p. 127.
    pub(crate) fn identifier(self) -> Option<Identifier> {
        let mut declarator = self;
        loop {
            let mut nested = None;
            for direct in declarator.kind {
                match *direct {
                    | DirectDeclarator::Identifier(identifier) => return Some(identifier),
                    | DirectDeclarator::Parenthesized(parenthesized) =>
                        nested = Some(parenthesized.declarator),
                    | _ => {},
                }
            }
            declarator = nested?;
        }
    }

    /// Whether the first derivation from the identifier is an unsized array.
    /// C99: flexible members §6.7.2.1 paragraph 16, p. 103; PDF p. 115.
    /// Pointer-to-array members do not use this later-standard form.
    pub(super) fn is_unsized_array(self) -> bool {
        let mut declarator = self;
        let mut suffix = None;
        loop {
            let mut direct = declarator.kind.iter().filter(|x| {
                !matches!(
                    x,
                    DirectDeclarator::Attributes(_) | DirectDeclarator::MsModifier(..)
                )
            });
            let first = direct.next();
            if let Some(candidate) = direct.next() {
                suffix = Some(*candidate);
            } else if !declarator.pointer.levels.is_empty() {
                suffix = None;
            }
            let Some(DirectDeclarator::Parenthesized(parenthesized)) = first else {
                return matches!(
                    suffix,
                    Some(DirectDeclarator::Array {
                        assignment_expression: None,
                        is_pointer: false,
                        ..
                    })
                );
            };
            declarator = parenthesized.declarator;
        }
    }

    /// The function suffix that applies to the declared identifier, looking
    /// through parenthesized declarators.
    ///
    /// C99: function declarators are §6.7.5.3 paragraph 5, pp. 118-119;
    /// PDF pp. 130-131.
    pub(super) fn function_suffix(self) -> Option<DirectDeclarator<'tu>> {
        let mut declarator = self;
        let mut suffix = None;
        loop {
            let mut direct = declarator.kind.iter().filter(|x| {
                !matches!(
                    x,
                    DirectDeclarator::Attributes(_) | DirectDeclarator::MsModifier(..)
                )
            });
            let first = direct.next();
            if let Some(candidate) = direct.next().copied()
                && matches!(
                    candidate,
                    DirectDeclarator::Function { .. } | DirectDeclarator::KAndRStyleFunction { .. }
                )
            {
                suffix = Some(candidate);
            }
            let Some(DirectDeclarator::Parenthesized(parenthesized)) = first else {
                return suffix;
            };
            declarator = parenthesized.declarator;
        }
    }
}

impl TypeSpecifiers<'_> {
    /// Whether a declaration of these specifiers alone can declare something:
    /// a tag, or the constants of an enumeration body.
    ///
    /// C99: §6.7 paragraph 2, p. 97; PDF p. 109. A tagged struct or union
    /// specifier declares its tag unless another declaration of the tag is
    /// visible (§6.7.2.3 paragraphs 6-9, pp. 106-107; PDF pp. 118-119),
    /// which syntax parsing does not track, so it is taken to declare one.
    /// An `enum identifier` without a body never declares its tag
    /// (paragraphs 3 and 9).
    pub(super) fn may_declare_tag_or_enumerators(self) -> bool {
        match self {
            | TypeSpecifiers::StructOrUnion(specifier) => specifier.identifier.is_some(),
            | TypeSpecifiers::Enum(specifier) => specifier.enumeration_list.is_some(),
            | _ => false,
        }
    }
}

impl Display for TypeSpecifiers<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | TypeSpecifiers::Extended(x) => f.write_str(match x {
                | super::modern::ExtendedType::Atomic(_) => "_Atomic",
                | super::modern::ExtendedType::Typeof {
                    unqualified: true, ..
                } => "typeof_unqual",
                | super::modern::ExtendedType::Typeof { .. } => "typeof",
                | super::modern::ExtendedType::BitInt {
                    signedness: Some(false),
                    ..
                } => "unsigned _BitInt",
                | super::modern::ExtendedType::BitInt { .. } => "_BitInt",
                | super::modern::ExtendedType::Decimal32 => "_Decimal32",
                | super::modern::ExtendedType::Decimal64 => "_Decimal64",
                | super::modern::ExtendedType::Decimal128 => "_Decimal128",
                | super::modern::ExtendedType::Inferred => "<inferred>",
                | super::modern::ExtendedType::AutoType => "__auto_type",
                | super::modern::ExtendedType::MsInteger { width, signedness } =>
                    return write!(
                        f,
                        "{}__int{width}",
                        if *signedness == Some(false) {
                            "unsigned "
                        } else if *signedness == Some(true) {
                            "signed "
                        } else {
                            ""
                        }
                    ),
                | super::modern::ExtendedType::Int128 {
                    signedness: Some(false),
                } => "unsigned __int128",
                | super::modern::ExtendedType::Int128 { .. } => "__int128",
                | super::modern::ExtendedType::Float128 { complex: false } => "__float128",
                | super::modern::ExtendedType::Float128 { complex: true } => "__float128 _Complex",
            }),
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

impl Default for DeclarationSpecifiers<'_> {
    fn default() -> Self {
        Self::new()
    }
}
