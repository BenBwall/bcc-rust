//! Maps each function definition's source lines to the lines its body
//! became in the IR before and after ABI lowering and in the assembly. The
//! IR keeps no source locations, so this is the granularity the outputs
//! support: a function's header through its closing line in each text.
//! Source lines are the editor's, physical lines of the input that `#line`
//! does not change; a function that a macro expansion begins starts at the
//! invocation, not in the macro's definition. A function is found in each
//! output under the assembler name it is emitted with, which
//! `__asm__("name")` can change.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "The explorer hands owned text and vectors across its interface, outside every arena."
)]

use super::{
    ExplorerLineMapping,
    ExplorerLineRange,
};
use crate::translation_phases::{
    Context,
    SourceVector,
    parsing::{
        ExternalDeclaration,
        ParsedTranslationUnit,
    },
    semantic_analysis::SemanticTranslationUnit,
};

/// The mapping for every function defined in `input`, the source file, in
/// source order. `outputs` are the IR before and after ABI lowering and the
/// assembly, each possibly missing.
pub(super) fn map<'tu>(
    unit: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    context: &Context<'tu>,
    input: &str,
    [ir, ir_abi, asm, wasm]: [Option<&str>; 4],
) -> Vec<ExplorerLineMapping> {
    let mut mappings = Vec::new();
    for declaration in unit.external_declarations() {
        let (ExternalDeclaration::FunctionDefinition(definition)
        | ExternalDeclaration::RecoveredFunctionDefinition(definition)) = declaration
        else {
            continue;
        };
        let Some(identifier) = definition.declarator.identifier() else {
            continue;
        };
        let name = context
            .string_cache
            .at(identifier.name)
            .trim_end_matches('\0');
        let symbol = sema
            .functions
            .iter()
            .find(|record| std::ptr::eq(record.syntax, *definition))
            .and_then(|record| sema.symbol_name(record.binding?, context))
            .unwrap_or(name);
        let in_input = |file: u32| {
            context
                .get_source_file(context.physical_source_file(file))
                .as_os_str()
                == input
        };
        let line = |vector: &SourceVector| {
            context
                .physical_position(vector.source_file_index, vector.index as usize)
                .map(|(line, _)| line)
        };
        // A definition starts where its first token is in the file: a macro
        // that expands to its first tokens starts it at the invocation.
        if definition.source_vectors.length() == 0 {
            continue;
        }
        let first = context.file_vector(definition.source_vectors);
        let Some(start) = line(first).filter(|_| in_input(first.source_file_index)) else {
            continue;
        };
        // The tokens of the macros it expands are spelled in their
        // definitions, which come before it, so they never raise its end.
        let end = context
            .get_source_vectors(definition.source_vectors)
            .iter()
            .filter(|vector| in_input(vector.source_file_index))
            .filter_map(line)
            .fold(start, u32::max);
        mappings.push(ExplorerLineMapping {
            function: name.to_owned(),
            source:   ExplorerLineRange { start, end },
            ir:       ir.and_then(|text| ir_function(text, symbol)),
            ir_abi:   ir_abi.and_then(|text| ir_function(text, symbol)),
            asm:      asm.and_then(|text| asm_function(text, symbol)),
            wasm:     wasm.and_then(|text| wasm_function(text, symbol)),
        });
    }
    mappings
}

/// A function's lines in the IR's text: from `function @name(…) … {` to
/// its closing `}`.
fn ir_function(text: &str, name: &str) -> Option<ExplorerLineRange> {
    let header = |line: &str| {
        line.strip_prefix("function @")
            .and_then(|rest| rest.strip_prefix(name))
            .is_some_and(|rest| rest.starts_with('(') && line.ends_with('{'))
    };
    range(text, header, |line| line == "}")
}

/// A function's lines in the assembly: from its label to the directive that
/// ends it (`.size` on ELF, `.seh_endproc` on COFF).
fn asm_function(text: &str, name: &str) -> Option<ExplorerLineRange> {
    let label = |line: &str| line.trim().strip_suffix(':') == Some(name);
    let end = |line: &str| {
        let line = line.trim();
        line.strip_prefix(".size").is_some_and(|rest| {
            rest.trim_start()
                .strip_prefix(name)
                .is_some_and(|rest| rest.trim_start().starts_with(','))
        }) || line == ".seh_endproc"
    };
    range(text, label, end)
}

/// A function's lines in the WebAssembly text: from `(func $name` to the
/// line that closes it, `  )`.
fn wasm_function(text: &str, name: &str) -> Option<ExplorerLineRange> {
    let header = |line: &str| {
        line.trim_start()
            .strip_prefix("(func $")
            .and_then(|rest| rest.strip_prefix(name))
            .is_some_and(|rest| rest.starts_with(' '))
    };
    range(text, header, |line| line == "  )")
}

/// The 1-based lines from the first that `start` accepts to the next that
/// `end` accepts, or to the last line.
fn range(
    text: &str,
    start: impl Fn(&str) -> bool,
    end: impl Fn(&str) -> bool,
) -> Option<ExplorerLineRange> {
    let mut lines = text.lines().enumerate();
    let (first, _) = lines.by_ref().find(|&(_, line)| start(line))?;
    let last = lines
        .find(|&(_, line)| end(line))
        .map_or(text.lines().count(), |(index, _)| index + 1);
    Some(ExplorerLineRange {
        start: u32::try_from(first + 1).ok()?,
        end:   u32::try_from(last).ok()?,
    })
}
