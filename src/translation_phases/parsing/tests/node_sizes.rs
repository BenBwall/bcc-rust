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
        StructDeclaration,
        StructDeclarator,
        StructOrUnionSpecifier,
        TypeName,
    },
    syntax::{
        BlockItem,
        Expression,
        ExpressionType,
        ExternalDeclaration,
        FunctionDefinition,
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
        Expression<'_> => 48,
        ExpressionType<'_> => 24,
        Statement<'_> => 48,
        StatementType<'_> => 32,
        FunctionDefinition<'_> => 88,
        Declaration<'_> => 56,
        InitDeclarator<'_> => 40,
        Initializer<'_> => 32,
        InitializerElement<'_> => 32,
        Designation<'_> => 32,
        Designator<'_> => 48,
        TypeName<'_> => 72,
        Declarator<'_> => 24,
        DirectDeclarator<'_> => 16,
        ParameterDeclaration<'_> => 64,
        StructDeclaration<'_> => 40,
        StructDeclarator<'_> => 40,
        StructOrUnionSpecifier<'_> => 32,
        EnumSpecifier<'_> => 32,
        Enumerator<'_> => 32,
        DeclarationSpecifiers<'_> => 32,
        ParenthesizedDeclarator<'_> => 32,
        BlockItem<'_> => 16,
        ExternalDeclaration<'_> => 16,
        ParseValue<'_> => 32,
        ParseFrame<'_, '_> => 144,
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
