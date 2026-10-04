//! Parser regression tests and the helpers they share.

mod aggregate_regressions;
mod declaration_recovery;
mod declaration_regressions;
mod declarations;
mod diagnostics;
mod driver_regressions;
mod expression_recovery;
mod expression_regressions;
mod expressions;
mod limits;
mod parameter_regressions;
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
        InitDeclarator,
    },
    errors::ParserErrorType,
    syntax::{
        BlockItem,
        ConstantExpressionIndex,
        ExpressionIndex,
        ExpressionSlot,
        ExternalDeclaration,
        FunctionDefinition,
        StatementIndex,
        StatementType,
    },
};
use crate::{
    configuration::CompilerConfiguration,
    translation_phases::{
        Context,
        SourceVectors,
        TranslationError,
        TranslationPhase,
        preprocessing::Preprocessor,
    },
    util::shared::SharedVec,
};

struct Parsed<'a, 'tu, 'tok> {
    parser:  Parser<'tok>,
    context: &'a mut Context<'tu>,
    items:   Vec<ExternalDeclaration>,
    errors:  Vec<TranslationError<'tu>>,
    source:  String,
}

fn with_parse<R>(source: &str, inspect: impl FnOnce(&mut Parsed<'_, '_, '_>) -> R) -> R {
    with_parse_with(source, CompilerConfiguration::default(), None, inspect)
}

fn with_parse_configuration<R>(
    source: &str,
    configuration: CompilerConfiguration,
    inspect: impl FnOnce(&Parsed<'_, '_, '_>) -> R,
) -> R {
    with_parse_with(source, configuration, None, |parsed| inspect(parsed))
}

fn with_parse_limits<R>(
    source: &str,
    limits: ParserLimits,
    inspect: impl FnOnce(&Parsed<'_, '_, '_>) -> R,
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
    inspect: impl FnOnce(&ParsedTranslationUnit, &mut Context<'_>) -> R,
) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let tok = crate::util::bump::Bump::new();
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<syntax-tree-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit = Parser::new(preprocessor, &mut context, &tok).parse_translation_unit(&mut context);
    inspect(&unit, &mut context)
}

fn with_parse_with<R>(
    source: &str,
    configuration: CompilerConfiguration,
    limits: Option<ParserLimits>,
    inspect: impl FnOnce(&mut Parsed<'_, '_, '_>) -> R,
) -> R {
    let source = source.to_owned();
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::with_configuration(&tu, configuration);
    let tok = crate::util::bump::Bump::new();
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<parser-test>").into_boxed_path(),
        &source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut parser =
        Parser::new_with_limits(preprocessor, &mut context, limits.unwrap_or_default(), &tok)
            .with_action_budget(source.len().saturating_mul(256).saturating_add(4_096));
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
    inspect(&mut Parsed {
        parser,
        context: &mut context,
        items,
        errors,
        source,
    })
}

fn declaration<'a>(parsed: &'a Parsed<'_, '_, '_>, item: usize) -> &'a Declaration {
    let index = match parsed.items[item] {
        | ExternalDeclaration::Declaration(index)
        | ExternalDeclaration::RecoveredDeclaration(index) => index,
        | ExternalDeclaration::FunctionDefinition(_)
        | ExternalDeclaration::RecoveredFunctionDefinition(_)
        | ExternalDeclaration::Error(_) => panic!("expected a declaration item"),
    };
    &parsed.parser.syntax[index]
}

fn function_definition<'a>(parsed: &'a Parsed<'_, '_, '_>, item: usize) -> &'a FunctionDefinition {
    let (ExternalDeclaration::FunctionDefinition(index)
    | ExternalDeclaration::RecoveredFunctionDefinition(index)) = parsed.items[item]
    else {
        panic!("expected a function-definition item")
    };
    &parsed.parser.syntax[index]
}

fn return_expression(parsed: &Parsed<'_, '_, '_>, statement: StatementIndex) -> ExpressionIndex {
    let StatementType::Return(Some(ExpressionSlot::Parsed(expression))) =
        parsed.parser.syntax[statement].kind
    else {
        panic!("expected a parsed return expression")
    };
    expression
}

fn block_items<'a>(parsed: &'a Parsed<'_, '_, '_>, statement: StatementIndex) -> &'a [BlockItem] {
    let StatementType::Compound { items } = parsed.parser.syntax[statement].kind else {
        panic!("expected a compound statement")
    };
    &parsed.parser.syntax[items]
}

fn init_declarators<'a>(
    parsed: &'a Parsed<'_, '_, '_>,
    declaration: &Declaration,
) -> &'a [InitDeclarator] {
    &parsed.parser.syntax[declaration.init_declarators]
}

fn identifier_name(parsed: &Parsed<'_, '_, '_>, declarator: Declarator) -> Option<String> {
    parsed
        .parser
        .declarator_identifier(declarator)
        .map(|identifier| parsed.context.string_cache.at(identifier.name).to_owned())
}

fn parser_errors<'a, 'tu>(
    parsed: &'a Parsed<'_, 'tu, '_>,
) -> impl Iterator<Item = &'a ParserErrorType<'tu>> {
    parsed.errors.iter().filter_map(|error| match error {
        | TranslationError::Parsing(error) => Some(&error.error_type),
        | _ => None,
    })
}

fn sourced_text(parsed: &Parsed<'_, '_, '_>, source_vectors: SourceVectors) -> String {
    parsed
        .context
        .get_source_vectors(source_vectors)
        .iter()
        .map(|vector| &parsed.source[vector.range()])
        .collect()
}

fn expression_text(parsed: &Parsed<'_, '_, '_>, expression: ExpressionIndex) -> String {
    sourced_text(parsed, parsed.parser.syntax[expression].source_vectors)
}

fn constant_expression_text(
    parsed: &Parsed<'_, '_, '_>,
    expression: ConstantExpressionIndex,
) -> String {
    expression_text(parsed, expression.into())
}
