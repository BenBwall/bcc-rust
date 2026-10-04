//! Parser-machine frames, actions, and values exchanged between frames.

use std::fmt::Debug;

use super::{
    Parser,
    compound_statement::CompoundStatementFrame,
    declaration::DeclarationFrame,
    declaration_specifiers::DeclarationSpecifiersFrame,
    declaration_syntax::{
        DeclarationSpecifiers,
        Declarator,
        DirectDeclarator,
    },
    declarator::DeclaratorFrame,
    enum_specifier::EnumSpecifierFrame,
    expression::ExpressionFrame,
    external_declaration::ExternalDeclarationFrame,
    frame_pool::FramePools,
    function_definition::FunctionDefinitionFrame,
    initializer::InitializerFrame,
    parameter_list::ParameterListFrame,
    recovery::SynchronizationSet,
    statement::StatementFrame,
    struct_or_union::StructOrUnionSpecifierFrame,
    syntax::{
        ConstantExpressionIndex,
        DeclarationIndex,
        EnumSpecifierIndex,
        ExpressionIndex,
        ExternalDeclaration,
        FunctionDefinitionIndex,
        InitializerIndex,
        StatementIndex,
        StructOrUnionSpecifierIndex,
        TypeNameIndex,
    },
    type_name::TypeNameFrame,
};
#[cfg(test)]
use crate::translation_phases::preprocessing::TokenType;
use crate::translation_phases::{
    Context,
    SourceVectors,
    preprocessing::Token,
};

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
    /// Specifier-qualifier list and optional abstract declarator.
    TypeName,
    /// C99 expression with a caller-selected grammar entry and boundary.
    Expression,
    /// Scalar or brace-enclosed initializer.
    Initializer,
    /// Function definition continuation after a completed declaration head.
    FunctionDefinition,
    /// Brace-delimited ordered block-item sequence.
    CompoundStatement,
    /// One labeled, compound, expression, selection, iteration, or jump
    /// statement.
    Statement,
}

impl ParseFrameKind {
    /// Returns the stable kebab-case label used in traces and diagnostics.
    pub(super) fn label(self) -> &'static str {
        match self {
            | Self::ExternalDeclaration => "external-declaration",
            | Self::Declaration => "declaration",
            | Self::DeclarationSpecifiers => "declaration-specifiers",
            | Self::Declarator => "declarator",
            | Self::ParameterList => "parameter-list",
            | Self::StructOrUnionSpecifier => "struct-or-union-specifier",
            | Self::EnumSpecifier => "enum-specifier",
            | Self::TypeName => "type-name",
            | Self::Expression => "expression",
            | Self::Initializer => "initializer",
            | Self::FunctionDefinition => "function-definition",
            | Self::CompoundStatement => "compound-statement",
            | Self::Statement => "statement",
        }
    }
}

/// Instruction returned by the active frame to the parser driver.
///
/// C99: these actions implement the §6.1 grammar notation without Rust call
/// recursion and support the translation-limit requirements of §5.2.4.1,
/// pp. 20-21; PDF pp. 32-33.
#[derive(Debug)]
pub(super) enum ParseAction {
    /// Consume the current token and keep the active frame.
    Consume,
    /// Suspend the active frame and push an unstarted child.
    Push(ParseFrame),
    /// Complete the active frame and return a typed value to its parent.
    Reduce(ParseValue),
    /// Keep the current token and run the active frame again after a state
    /// change.
    Reprocess,
    /// Keep the current token and run the active frame again immediately.
    /// Frames return this for forward phase changes that need no driver
    /// work; [`ParseFrame::step`] handles it, so it never reaches the driver.
    Continue,
    /// Scan to a production-specific boundary before resuming the active frame.
    Recover(SynchronizationSet),
}

/// Typed value returned by a completed child frame.
///
/// C99: values correspond to completed nonterminals from §6.7-§6.9,
/// pp. 97-144; PDF pp. 109-156. Typed returns are an implementation mechanism.
#[derive(Debug, Clone, Copy)]
pub(super) enum ParseValue {
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
    /// Arena handle for a completed type name.
    TypeName(TypeNameIndex),
    Expression(ExpressionResult),
    ConstantExpression(ConstantExpressionResult),
    Initializer(InitializerResult),
    /// Arena handle for a completed declaration.
    Declaration(DeclarationIndex),
    FunctionDefinition(FunctionDefinitionIndex),
    CompoundStatement(StatementIndex),
    Statement(StatementIndex),
    /// External item ready to be yielded by the translation-phase seam.
    ExternalDeclaration(ExternalDeclaration),
}

/// Parameter-list child result before it is appended to a declarator frame.
///
/// C99: parameter-type-list, parameter-list, and identifier-list are specified
/// by §6.7.5, p. 114; PDF p. 126, with semantics in §6.7.5.3,
/// pp. 118-121; PDF pp. 130-133.
#[derive(Debug, Clone, Copy)]
pub(super) struct ParameterListResult {
    /// Function suffix constructed from the parsed list.
    pub(super) direct_declarator: DirectDeclarator,
    /// Exact source provenance owned by the parameter-list frame.
    pub(super) source_vectors:    SourceVectors,
}

/// Enum child result before its type specifier is merged into the parent.
#[derive(Debug, Clone, Copy)]
pub(super) struct EnumSpecifierResult {
    /// Arena handle for the completed enum specifier.
    pub(super) index: EnumSpecifierIndex,
    /// Whether recovery stopped before a following declaration.
    pub(super) stopped_before_declaration: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ExpressionResult {
    pub(super) index:     ExpressionIndex,
    pub(super) recovered: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ConstantExpressionResult {
    pub(super) index:     ConstantExpressionIndex,
    pub(super) recovered: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct InitializerResult {
    pub(super) index:     InitializerIndex,
    pub(super) recovered: bool,
}

/// Sum type for every grammar frame currently implemented by the parser.
///
/// C99: the represented grammar families currently cover declarations through
/// external definitions, §6.7-§6.9, pp. 97-144; PDF pp. 109-156.
#[derive(Debug)]
pub(super) enum ParseFrame {
    ExternalDeclaration(ExternalDeclarationFrame),
    Declaration(DeclarationFrame),
    DeclarationSpecifiers(DeclarationSpecifiersFrame),
    Declarator(DeclaratorFrame),
    // Parameter lists and struct/union bodies already own growing lists, so
    // boxing them keeps every other frame push small.
    ParameterList(Box<ParameterListFrame>),
    StructOrUnionSpecifier(Box<StructOrUnionSpecifierFrame>),
    EnumSpecifier(EnumSpecifierFrame),
    TypeName(TypeNameFrame),
    Expression(ExpressionFrame),
    Initializer(InitializerFrame),
    FunctionDefinition(FunctionDefinitionFrame),
    CompoundStatement(CompoundStatementFrame),
    Statement(StatementFrame),
}

#[cfg(test)]
/// One observable driver action used to prove ownership and progress in tests.
///
/// C99: implementation instrumentation for the iterative grammar strategy;
/// deep nesting requirements are §5.2.4.1, pp. 20-21; PDF pp. 32-33.
#[derive(Debug, Clone, Copy)]
pub(super) struct FrameTraceEvent {
    pub(super) frame:  &'static str,
    pub(super) action: &'static str,
    pub(super) token:  Option<TokenType>,
    pub(super) depth:  usize,
}

impl ParseAction {
    #[cfg(test)]
    pub(super) fn name(&self) -> &'static str {
        match self {
            | Self::Consume => "consume",
            | Self::Push(_) => "push",
            | Self::Reduce(_) => "reduce",
            | Self::Reprocess => "reprocess",
            | Self::Continue => "continue",
            | Self::Recover(_) => "recover",
        }
    }
}

impl ParseFrame {
    /// Counts arena entries retained by a frame before their final bulk insert.
    pub(super) fn retained_node_count(&self) -> usize {
        match self {
            | Self::Declarator(frame) => frame
                .pointer_qualifiers
                .len()
                .saturating_add(frame.direct_declarators.len()),
            | Self::ParameterList(frame) => frame
                .parameters
                .len()
                .saturating_add(frame.identifiers.len()),
            | Self::StructOrUnionSpecifier(frame) => frame
                .declarations
                .len()
                .saturating_add(frame.member_declarators.len()),
            | Self::EnumSpecifier(frame) => frame.enumerators.len(),
            | Self::Expression(frame) => frame.call.as_ref().map_or(0, |call| call.arguments.len()),
            | Self::Initializer(frame) => frame.elements.len().saturating_add(
                frame
                    .designation
                    .as_ref()
                    .map_or(0, |designation| designation.current_designators.len()),
            ),
            | Self::FunctionDefinition(frame) => frame.declaration_list.len(),
            | Self::CompoundStatement(frame) => frame.items.len(),
            | Self::Declaration(frame) => frame.init_declarators.len(),
            | Self::ExternalDeclaration(_)
            | Self::DeclarationSpecifiers(_)
            | Self::TypeName(_)
            | Self::Statement(_) => 0,
        }
    }

    /// Gives a newly pushed frame spare vectors for the lists it grows.
    pub(super) fn lend_pooled(&mut self, pools: &mut FramePools) {
        match self {
            | Self::Declaration(frame) => {
                pools.init_declarators.lend(&mut frame.init_declarators);
                pools.source_vectors.lend(&mut frame.source_vectors);
            },
            | Self::Declarator(frame) => {
                pools.pointer_qualifiers.lend(&mut frame.pointer_qualifiers);
                pools.direct_declarators.lend(&mut frame.direct_declarators);
            },
            | Self::Expression(frame) => frame.lend_pooled(pools),
            | Self::Initializer(frame) => {
                pools.initializers.lend(&mut frame.elements);
                pools.source_vectors.lend(&mut frame.source_vectors);
            },
            | Self::CompoundStatement(frame) => {
                pools.block_items.lend(&mut frame.items);
                pools.source_vectors.lend(&mut frame.source_vectors);
            },
            | Self::ParameterList(frame) => {
                pools.parameters.lend(&mut frame.parameters);
                pools.identifiers.lend(&mut frame.identifiers);
                pools.source_vectors.lend(&mut frame.source_vectors);
            },
            | Self::StructOrUnionSpecifier(frame) => {
                pools.struct_members.lend(&mut frame.declarations);
                pools.struct_declarators.lend(&mut frame.member_declarators);
                pools.source_vectors.lend(&mut frame.source_vectors);
            },
            | Self::EnumSpecifier(frame) => {
                pools.enumerators.lend(&mut frame.enumerators);
                pools.source_vectors.lend(&mut frame.source_vectors);
            },
            | Self::ExternalDeclaration(_)
            | Self::DeclarationSpecifiers(_)
            | Self::TypeName(_)
            | Self::FunctionDefinition(_)
            | Self::Statement(_) => {},
        }
    }

    /// Returns a popped frame's vectors to the pools.
    pub(super) fn reclaim_pooled(&mut self, pools: &mut FramePools) {
        match self {
            | Self::Declaration(frame) => {
                pools.init_declarators.reclaim(&mut frame.init_declarators);
                pools.source_vectors.reclaim(&mut frame.source_vectors);
            },
            | Self::Declarator(frame) => {
                pools
                    .pointer_qualifiers
                    .reclaim(&mut frame.pointer_qualifiers);
                pools
                    .direct_declarators
                    .reclaim(&mut frame.direct_declarators);
            },
            | Self::Expression(frame) => frame.reclaim_pooled(pools),
            | Self::Initializer(frame) => {
                pools.initializers.reclaim(&mut frame.elements);
                pools.source_vectors.reclaim(&mut frame.source_vectors);
            },
            | Self::CompoundStatement(frame) => {
                pools.block_items.reclaim(&mut frame.items);
                pools.source_vectors.reclaim(&mut frame.source_vectors);
            },
            | Self::ParameterList(frame) => {
                pools.parameters.reclaim(&mut frame.parameters);
                pools.identifiers.reclaim(&mut frame.identifiers);
                pools.source_vectors.reclaim(&mut frame.source_vectors);
            },
            | Self::StructOrUnionSpecifier(frame) => {
                pools.struct_members.reclaim(&mut frame.declarations);
                pools
                    .struct_declarators
                    .reclaim(&mut frame.member_declarators);
                pools.source_vectors.reclaim(&mut frame.source_vectors);
            },
            | Self::EnumSpecifier(frame) => {
                pools.enumerators.reclaim(&mut frame.enumerators);
                pools.source_vectors.reclaim(&mut frame.source_vectors);
            },
            | Self::ExternalDeclaration(_)
            | Self::DeclarationSpecifiers(_)
            | Self::TypeName(_)
            | Self::FunctionDefinition(_)
            | Self::Statement(_) => {},
        }
    }

    pub(super) fn kind(&self) -> ParseFrameKind {
        match self {
            | Self::ExternalDeclaration(_) => ParseFrameKind::ExternalDeclaration,
            | Self::Declaration(_) => ParseFrameKind::Declaration,
            | Self::DeclarationSpecifiers(_) => ParseFrameKind::DeclarationSpecifiers,
            | Self::Declarator(_) => ParseFrameKind::Declarator,
            | Self::ParameterList(_) => ParseFrameKind::ParameterList,
            | Self::StructOrUnionSpecifier(_) => ParseFrameKind::StructOrUnionSpecifier,
            | Self::EnumSpecifier(_) => ParseFrameKind::EnumSpecifier,
            | Self::TypeName(_) => ParseFrameKind::TypeName,
            | Self::Expression(_) => ParseFrameKind::Expression,
            | Self::Initializer(_) => ParseFrameKind::Initializer,
            | Self::FunctionDefinition(_) => ParseFrameKind::FunctionDefinition,
            | Self::CompoundStatement(_) => ParseFrameKind::CompoundStatement,
            | Self::Statement(_) => ParseFrameKind::Statement,
        }
    }

    /// Adds tokens consumed by recovery to the syntax object owned by this
    /// frame; frames without an owned syntax range intentionally ignore them.
    pub(super) fn merge_recovered_sources(
        &mut self,
        context: &mut Context<'_>,
        recovered: Option<SourceVectors>,
    ) {
        let Some(recovered) = recovered else {
            return;
        };
        let destination = match self {
            | Self::ExternalDeclaration(_)
            | Self::DeclarationSpecifiers(_)
            | Self::TypeName(_)
            | Self::Expression(_) => return,
            | Self::Declaration(frame) => {
                frame.source_vectors.push(recovered);
                return;
            },
            | Self::Declarator(frame) => &mut frame.source_vectors,
            | Self::ParameterList(frame) => {
                frame.source_vectors.push(recovered);
                return;
            },
            | Self::StructOrUnionSpecifier(frame) => {
                frame.source_vectors.push(recovered);
                return;
            },
            | Self::EnumSpecifier(frame) => {
                frame.source_vectors.push(recovered);
                return;
            },
            | Self::Initializer(frame) => {
                frame.source_vectors.push(recovered);
                return;
            },
            | Self::FunctionDefinition(frame) => &mut frame.source_vectors,
            | Self::CompoundStatement(frame) => {
                frame.source_vectors.push(recovered);
                return;
            },
            | Self::Statement(frame) => &mut frame.source_vectors,
        };
        *destination = Some(destination.map_or(recovered, |existing| {
            context.merge_vectors(existing, recovered)
        }));
    }

    /// Runs the active frame until it returns an action for the driver,
    /// repeating [`ParseAction::Continue`] transitions in place.
    pub(super) fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context<'_>,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        let mut returned = returned;
        loop {
            let action = self.step_once(parser, context, token, returned.take());
            if !matches!(action, ParseAction::Continue) {
                return action;
            }
        }
    }

    /// Dispatches one transition to the concrete active frame. The frame's
    /// kind cannot change during a step, so the driver reads it beforehand.
    fn step_once(
        &mut self,
        parser: &mut Parser,
        context: &mut Context<'_>,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self {
            | Self::ExternalDeclaration(frame) => frame.step(parser, context, token, returned),
            | Self::Declaration(frame) => frame.step(parser, context, token, returned),
            | Self::DeclarationSpecifiers(frame) => frame.step(parser, context, token, returned),
            | Self::Declarator(frame) => frame.step(parser, context, token, returned),
            | Self::ParameterList(frame) => frame.step(parser, context, token, returned),
            | Self::StructOrUnionSpecifier(frame) => frame.step(parser, context, token, returned),
            | Self::EnumSpecifier(frame) => frame.step(parser, context, token, returned),
            | Self::TypeName(frame) => frame.step(parser, context, token, returned),
            | Self::Expression(frame) => frame.step(parser, context, token, returned),
            | Self::Initializer(frame) => frame.step(parser, context, token, returned),
            | Self::FunctionDefinition(frame) => frame.step(parser, context, token, returned),
            | Self::CompoundStatement(frame) => frame.step(parser, context, token, returned),
            | Self::Statement(frame) => frame.step(parser, context, token, returned),
        }
    }
}

pub(super) fn expression_value(returned: Option<ParseValue>) -> ExpressionIndex {
    let Some(ParseValue::Expression(ExpressionResult { index, .. })) = returned else {
        panic!("expression child returned an unexpected value: {returned:?}");
    };
    index
}

pub(super) fn any_expression_value(returned: Option<ParseValue>) -> ExpressionIndex {
    match returned {
        | Some(ParseValue::Expression(ExpressionResult { index, .. })) => index,
        | Some(ParseValue::ConstantExpression(ConstantExpressionResult { index, .. })) =>
            index.into(),
        | returned => panic!("expression child returned an unexpected value: {returned:?}"),
    }
}
