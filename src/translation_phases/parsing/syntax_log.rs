//! The node kinds the parser allocates in the translation-unit arena, and a
//! test-only log of every allocated node by kind.
//!
//! Infrastructure for the syntax tree that translation phase 7 builds
//! (§5.1.1.2 paragraph 1, p. 10; PDF p. 22). It encodes no rule of the
//! standard.

use super::{
    declaration_syntax::{
        Declaration,
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
        PointerLevel,
        RangeDesignator,
        StructDeclaration,
        StructDeclarator,
        StructOrUnionSpecifier,
        TypeName,
    },
    extensions::{
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
    },
    syntax::{
        AttributedStatement,
        BlockItem,
        CaseRange,
        Expression,
        FunctionDefinition,
        Identifier,
        SelectionHeader,
        Statement,
    },
};
use crate::translation_phases::preprocessing::Token;

/// A node kind allocated in the translation-unit arena.
///
/// [`Parser::alloc_syntax`](super::Parser::alloc_syntax) and
/// [`Parser::alloc_syntax_list`](super::Parser::alloc_syntax_list) accept only
/// these kinds, so test builds can log each node by kind.
pub(super) trait TreeNode<'tu>: Sized + 'tu {
    /// Records `node` in the test log.
    #[cfg(test)]
    fn log(log: &mut SyntaxLog<'tu>, node: &'tu Self);
}

#[cfg(test)]
impl<'tu> SyntaxLog<'tu> {
    /// Counts `node` and logs it by kind.
    pub(super) fn record<T: TreeNode<'tu>>(&mut self, node: &'tu T) {
        self.nodes += 1;
        T::log(self, node);
    }

    /// Puts `new` where the log holds `old`, which it replaces in the tree.
    /// The node count does not change.
    pub(super) fn replace_expression(
        &mut self,
        old: &'tu Expression<'tu>,
        new: &'tu Expression<'tu>,
    ) {
        if let Some(slot) = self
            .expressions
            .iter_mut()
            .rev()
            .find(|logged| std::ptr::eq(**logged, old))
        {
            *slot = new;
        }
    }
}

/// Every syntax node the parser allocated, by kind and in allocation order,
/// so tests can count and visit nodes that the roots may not reach (such as
/// syntax abandoned during recovery).
#[cfg(test)]
#[derive(Debug, Default)]
#[expect(
    clippy::disallowed_types,
    reason = "A test-only log of every allocated node, compiled only under `cfg(test)`."
)]
pub(super) struct SyntaxLog<'tu> {
    ms_asm:                Vec<&'tu MsAsm<'tu>>,
    seh:                   Vec<&'tu Seh<'tu>>,
    asm:                   Vec<&'tu Asm<'tu>>,
    asm_operands:          Vec<&'tu AsmOperand<'tu>>,
    builtins:              Vec<&'tu Builtin<'tu>>,
    offset_members:        Vec<&'tu OffsetMember<'tu>>,
    syntax_operands:       Vec<&'tu SyntaxOperand<'tu>>,
    ranges:                Vec<&'tu RangeDesignator<'tu>>,
    extended_types:        Vec<&'tu ExtendedType<'tu>>,
    specifier_extensions:  Vec<&'tu SpecifierExtension<'tu>>,
    attributes:            Vec<&'tu AttributeSpecifier<'tu>>,
    generics:              Vec<&'tu GenericSelection<'tu>>,
    associations:          Vec<&'tu GenericAssociation<'tu>>,
    assertions:            Vec<&'tu StaticAssertion<'tu>>,
    attributed_statements: Vec<&'tu AttributedStatement<'tu>>,
    selection_headers:     Vec<&'tu SelectionHeader<'tu>>,
    case_ranges:           Vec<&'tu CaseRange<'tu>>,
    nodes:                 usize,
    expressions:           Vec<&'tu Expression<'tu>>,
    type_names:            Vec<&'tu TypeName<'tu>>,
    initializers:          Vec<&'tu Initializer<'tu>>,
    initializer_elements:  Vec<&'tu InitializerElement<'tu>>,
    designations:          Vec<&'tu Designation<'tu>>,
    designators:           Vec<&'tu Designator<'tu>>,
    declarations:          Vec<&'tu Declaration<'tu>>,
    init_declarators:      Vec<&'tu InitDeclarator<'tu>>,
    direct_declarators:    Vec<&'tu DirectDeclarator<'tu>>,
    parenthesized:         Vec<&'tu ParenthesizedDeclarator<'tu>>,
    parameters:            Vec<&'tu ParameterDeclaration<'tu>>,
    struct_or_unions:      Vec<&'tu StructOrUnionSpecifier<'tu>>,
    struct_declarations:   Vec<&'tu StructDeclaration<'tu>>,
    struct_declarators:    Vec<&'tu StructDeclarator<'tu>>,
    enum_specifiers:       Vec<&'tu EnumSpecifier<'tu>>,
    enumerators:           Vec<&'tu Enumerator<'tu>>,
    pointer_levels:        Vec<&'tu PointerLevel<'tu>>,
    identifiers:           Vec<&'tu Identifier>,
    statements:            Vec<&'tu Statement<'tu>>,
    block_items:           Vec<&'tu BlockItem<'tu>>,
    function_definitions:  Vec<&'tu FunctionDefinition<'tu>>,
}

/// A node kind that tests can count and visit.
#[cfg(test)]
pub(super) trait LoggedNode<'tu>: TreeNode<'tu> {
    fn logged<'a>(log: &'a SyntaxLog<'tu>) -> &'a [&'tu Self];
}

macro_rules! tree_nodes {
    ($($node:ident => $field:ident),* $(,)?) => {$(
        impl<'tu> TreeNode<'tu> for $node<'tu> {
            #[cfg(test)]
            fn log(log: &mut SyntaxLog<'tu>, node: &'tu Self) {
                log.$field.push(node);
            }
        }

        #[cfg(test)]
        impl<'tu> LoggedNode<'tu> for $node<'tu> {
            fn logged<'a>(log: &'a SyntaxLog<'tu>) -> &'a [&'tu Self] {
                &log.$field
            }
        }
    )*};
}

tree_nodes! {
    RangeDesignator => ranges,
    Asm => asm,
    MsAsm => ms_asm,
    Seh => seh,
    AsmOperand => asm_operands,
    Builtin => builtins,
    OffsetMember => offset_members,
    SyntaxOperand => syntax_operands,
    ExtendedType => extended_types,
    SpecifierExtension => specifier_extensions,
    AttributeSpecifier => attributes,
    GenericSelection => generics,
    GenericAssociation => associations,
    StaticAssertion => assertions,
    AttributedStatement => attributed_statements,
    SelectionHeader => selection_headers,
    CaseRange => case_ranges,
    Expression => expressions,
    TypeName => type_names,
    Initializer => initializers,
    InitializerElement => initializer_elements,
    Designation => designations,
    Designator => designators,
    Declaration => declarations,
    InitDeclarator => init_declarators,
    DirectDeclarator => direct_declarators,
    PointerLevel => pointer_levels,
    ParenthesizedDeclarator => parenthesized,
    ParameterDeclaration => parameters,
    StructOrUnionSpecifier => struct_or_unions,
    StructDeclaration => struct_declarations,
    StructDeclarator => struct_declarators,
    EnumSpecifier => enum_specifiers,
    Enumerator => enumerators,
    Statement => statements,
    BlockItem => block_items,
    FunctionDefinition => function_definitions,
}

macro_rules! plain_tree_nodes {
    ($($node:ident => $field:ident),* $(,)?) => {$(
        #[cfg_attr(
            not(test),
            expect(single_use_lifetimes, reason = "Only the test log names the lifetime.")
        )]
        impl<'tu> TreeNode<'tu> for $node {
            #[cfg(test)]
            fn log(log: &mut SyntaxLog<'tu>, node: &'tu Self) {
                log.$field.push(node);
            }
        }

        #[cfg(test)]
        impl<'tu> LoggedNode<'tu> for $node {
            fn logged<'a>(log: &'a SyntaxLog<'tu>) -> &'a [&'tu Self] {
                &log.$field
            }
        }
    )*};
}

plain_tree_nodes! {
    Identifier => identifiers,
}

#[cfg(test)]
impl<'tu> SyntaxLog<'tu> {
    /// Nodes of every kind.
    pub(super) fn node_count(&self) -> usize {
        self.nodes
    }

    /// Nodes of kind `T`.
    pub(super) fn count<T: LoggedNode<'tu>>(&self) -> usize {
        T::logged(self).len()
    }

    /// Every node of kind `T`, in allocation order.
    pub(super) fn iter<T: LoggedNode<'tu>>(&self) -> impl Iterator<Item = &'tu T> + '_ {
        T::logged(self).iter().copied()
    }

    /// The `n`th allocated node of kind `T`.
    pub(super) fn nth<T: LoggedNode<'tu>>(&self, n: usize) -> &'tu T {
        T::logged(self)
            .get(n)
            .copied()
            .expect("the syntax log holds that many nodes of this kind")
    }
}

impl TreeNode<'_> for Token {
    #[cfg(test)]
    fn log(_: &mut SyntaxLog<'_>, _: &Self) {}
}

/// Call arguments are lists of expression references; the expressions
/// themselves are logged when allocated.
impl<'tu> TreeNode<'tu> for &'tu Expression<'tu> {
    #[cfg(test)]
    fn log(_: &mut SyntaxLog<'tu>, _: &'tu Self) {}
}

/// Old-style declaration lists are lists of declaration references.
impl<'tu> TreeNode<'tu> for &'tu Declaration<'tu> {
    #[cfg(test)]
    fn log(_: &mut SyntaxLog<'tu>, _: &'tu Self) {}
}
