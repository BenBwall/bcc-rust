//! Parser regression tests and the helpers they share.

mod aggregate_regressions;
mod declaration_recovery;
mod declaration_regressions;
mod declarations;
mod dialect_regressions;
mod diagnostics;
mod driver_regressions;
mod expression_recovery;
mod expression_regressions;
mod expressions;
mod gnu;
mod limits;
mod msvc;
mod node_sizes;
mod parameter_regressions;
mod standards;
mod statement_regressions;
mod statements;
mod translation_unit;

use std::path::PathBuf;

use super::{
    ParsedTranslationUnit,
    Parser,
    ParserLimits,
    declaration_syntax::{
        Declaration,
        Declarator,
    },
    errors::ParserErrorType,
    syntax::{
        BlockItem,
        ConstantExpression,
        Expression,
        ExpressionSlot,
        ExternalDeclaration,
        FunctionDefinition,
        Statement,
        StatementType,
    },
};
use crate::{
    configuration::CompilerConfiguration,
    translation_phases::{
        Context,
        SourceVectors,
        TranslationError,
        preprocessing::Preprocessor,
    },
    util::shared::SharedVec,
};

struct Parsed<'a, 'tu> {
    parser: Parser<'a, 'tu, 'a>,
    items:  Vec<ExternalDeclaration<'tu>>,
    errors: Vec<TranslationError<'tu>>,
    source: String,
}

fn with_parse<R>(source: &str, inspect: impl FnOnce(&mut Parsed<'_, '_>) -> R) -> R {
    with_parse_with(source, CompilerConfiguration::default(), None, inspect)
}

fn with_parse_configuration<R>(
    source: &str,
    configuration: CompilerConfiguration,
    inspect: impl FnOnce(&Parsed<'_, '_>) -> R,
) -> R {
    with_parse_with(source, configuration, None, |parsed| inspect(parsed))
}

fn with_parse_limits<R>(
    source: &str,
    limits: ParserLimits,
    inspect: impl FnOnce(&Parsed<'_, '_>) -> R,
) -> R {
    with_parse_with(
        source,
        CompilerConfiguration::default(),
        Some(limits),
        |parsed| inspect(parsed),
    )
}

fn with_parsed<R>(
    source: &str,
    inspect: impl FnOnce(&ParsedTranslationUnit<'_>, &mut Context<'_>) -> R,
) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let parse_arena = crate::util::bump::Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<syntax-tree-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit = Parser::new(preprocessor, &mut context, &parse_arena).parse_translation_unit();
    inspect(&unit, &mut context)
}

fn with_parse_with<R>(
    source: &str,
    configuration: CompilerConfiguration,
    limits: Option<ParserLimits>,
    inspect: impl FnOnce(&mut Parsed<'_, '_>) -> R,
) -> R {
    let source = source.to_owned();
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::with_configuration(&tu, configuration);
    let preprocess_arena = crate::util::bump::Bump::new();
    let parse_arena = crate::util::bump::Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<parser-test>").into_boxed_path(),
        &source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut parser = Parser::new_with_limits(
        preprocessor,
        &mut context,
        limits.unwrap_or_default(),
        &parse_arena,
    )
    .with_action_budget(source.len().saturating_mul(256).saturating_add(4_096));
    let mut items = Vec::new();
    while let Some(item) = parser.next_item() {
        items.push(item);
        assert!(
            items.len() < 10_000,
            "parser failed to make item-level progress"
        );
    }
    let mut errors = Vec::new();
    while let Some(error) = parser.context.pop_pending_error() {
        errors.push(error);
    }
    inspect(&mut Parsed {
        parser,
        items,
        errors,
        source,
    })
}

fn declaration<'tu>(parsed: &Parsed<'_, 'tu>, item: usize) -> &'tu Declaration<'tu> {
    match parsed.items[item] {
        | ExternalDeclaration::Declaration(declaration)
        | ExternalDeclaration::RecoveredDeclaration(declaration) => declaration,
        | ExternalDeclaration::FunctionDefinition(_)
        | ExternalDeclaration::RecoveredFunctionDefinition(_)
        | ExternalDeclaration::Asm(_)
        | ExternalDeclaration::Error(_) => panic!("expected a declaration item"),
    }
}

fn function_definition<'tu>(parsed: &Parsed<'_, 'tu>, item: usize) -> &'tu FunctionDefinition<'tu> {
    let (ExternalDeclaration::FunctionDefinition(definition)
    | ExternalDeclaration::RecoveredFunctionDefinition(definition)) = parsed.items[item]
    else {
        panic!("expected a function-definition item")
    };
    definition
}

fn return_expression<'tu>(statement: &Statement<'tu>) -> &'tu Expression<'tu> {
    let StatementType::Return(Some(ExpressionSlot::Parsed(expression))) = statement.kind else {
        panic!("expected a parsed return expression")
    };
    expression
}

fn block_items<'tu>(statement: &Statement<'tu>) -> &'tu [BlockItem<'tu>] {
    let StatementType::Compound { items } = statement.kind else {
        panic!("expected a compound statement")
    };
    items.as_slice()
}

fn identifier_name(parsed: &Parsed<'_, '_>, declarator: Declarator<'_>) -> Option<String> {
    declarator.identifier().map(|identifier| {
        parsed
            .parser
            .context
            .string_cache
            .at(identifier.name)
            .to_owned()
    })
}

fn parser_errors<'a, 'tu>(
    parsed: &'a Parsed<'_, 'tu>,
) -> impl Iterator<Item = &'a ParserErrorType<'tu>> {
    parsed.errors.iter().filter_map(|error| match error {
        | TranslationError::Parsing(error) => Some(&error.error_type),
        | _ => None,
    })
}

fn sourced_text(parsed: &Parsed<'_, '_>, source_vectors: SourceVectors) -> String {
    parsed
        .parser
        .context
        .get_source_vectors(source_vectors)
        .iter()
        .map(|vector| &parsed.source[vector.range()])
        .collect()
}

fn expression_text(parsed: &Parsed<'_, '_>, expression: &Expression<'_>) -> String {
    sourced_text(parsed, expression.source_vectors)
}

fn constant_expression_text(parsed: &Parsed<'_, '_>, expression: ConstantExpression<'_>) -> String {
    expression_text(parsed, expression.into())
}
