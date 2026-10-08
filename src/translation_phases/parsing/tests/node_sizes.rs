//! Syntax-tree node sizes.
//!
//! Every node lives in the translation-unit arena until the unit ends, so
//! these sizes set most of the tree's memory. A change here should be a
//! deliberate one.

use super::super::{
    ParseFrame,
    ParseValue,
    declaration_syntax::{
        Declaration,
        DeclarationSpecifiers,
        Declarator,
        Designation,
        Designator,
        DirectDeclarator,
        EnumSpecifier,
        Enumerator,
        InitDeclarator,
        Initializer,
        InitializerElement,
        ParameterDeclaration,
        ParenthesizedDeclarator,
        RangeDesignator,
        StructDeclaration,
        StructDeclarator,
        StructOrUnionSpecifier,
        TypeName,
    },
    gnu::{
        Asm,
        AsmOperand,
        Builtin,
        OffsetMember,
    },
    modern::{
        AttributeSpecifier,
        ExtendedType,
        GenericAssociation,
        GenericSelection,
        SpecifierExtension,
        StaticAssertion,
        SyntaxOperand,
    },
    msvc::{
        MsAsm,
        Seh,
    },
    syntax::{
        AttributedStatement,
        BlockItem,
        CaseRange,
        Expression,
        ExpressionType,
        ExternalDeclaration,
        FunctionDefinition,
        SelectionHeader,
        Statement,
        StatementType,
    },
};

/// `(type, size_of, expected)` for each listed type.
macro_rules! sizes {
    ($($node:ty => $bytes:literal),+ $(,)?) => {
        [$((stringify!($node), size_of::<$node>(), $bytes)),+]
    };
}

#[cfg(target_pointer_width = "64")]
#[test]
fn syntax_nodes_keep_their_sizes() {
    let sizes = sizes![
        Asm<'_> => 40,
        MsAsm<'_> => 24,
        Seh<'_> => 56,
        AsmOperand<'_> => 56,
        Builtin<'_> => 32,
        OffsetMember<'_> => 16,
        RangeDesignator<'_> => 16,
        SyntaxOperand<'_> => 16,
        ExtendedType<'_> => 24,
        SpecifierExtension<'_> => 32,
        AttributeSpecifier<'_> => 24,
        GenericSelection<'_> => 40,
        GenericAssociation<'_> => 16,
        StaticAssertion<'_> => 56,
        AttributedStatement<'_> => 16,
        SelectionHeader<'_> => 24,
        CaseRange<'_> => 40,
        Expression<'_> => 48,
        ExpressionType<'_> => 24,
        Statement<'_> => 48,
        StatementType<'_> => 32,
        FunctionDefinition<'_> => 96,
        Declaration<'_> => 72,
        InitDeclarator<'_> => 40,
        Initializer<'_> => 32,
        InitializerElement<'_> => 32,
        Designation<'_> => 32,
        Designator<'_> => 48,
        TypeName<'_> => 80,
        Declarator<'_> => 24,
        DirectDeclarator<'_> => 16,
        ParameterDeclaration<'_> => 72,
        StructDeclaration<'_> => 56,
        StructDeclarator<'_> => 48,
        StructOrUnionSpecifier<'_> => 40,
        EnumSpecifier<'_> => 48,
        Enumerator<'_> => 40,
        DeclarationSpecifiers<'_> => 40,
        ParenthesizedDeclarator<'_> => 32,
        BlockItem<'_> => 16,
        ExternalDeclaration<'_> => 16,
        ParseValue<'_> => 40,
        ParseFrame<'_, '_> => 176,
    ];
    let changed: Vec<_> = sizes
        .into_iter()
        .filter(|&(_, actual, expected)| actual != expected)
        .collect();
    assert!(
        changed.is_empty(),
        "node sizes changed (node, bytes, expected bytes): {changed:?}"
    );
}
