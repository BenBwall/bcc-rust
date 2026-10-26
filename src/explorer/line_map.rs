//! Maps each function definition's source lines to the lines its body
//! became in the IR before and after ABI lowering and in the assembly. The
//! IR keeps no source locations, so this is the granularity the outputs
//! support: a function's header through its closing line in each text.
//! Source lines are the editor's, physical lines of the input that `#line`
//! does not change.

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
    parsing::{
        ExternalDeclaration,
        ParsedTranslationUnit,
    },
};

/// The mapping for every function defined in `input`, the source file, in
/// source order. `outputs` are the IR before and after ABI lowering and the
/// assembly, each possibly missing.
pub(super) fn map(
    unit: &ParsedTranslationUnit<'_>,
    context: &Context<'_>,
    input: &str,
    [ir, ir_abi, asm]: [Option<&str>; 3],
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
        let mut lines = context
            .get_source_vectors(definition.source_vectors)
            .iter()
            .filter(|vector| {
                let physical = context.physical_source_file(vector.source_file_index);
                context.get_source_file(physical).as_os_str() == input
            })
            .filter_map(|vector| {
                context.physical_position(vector.source_file_index, vector.index as usize)
            })
            .map(|(line, _)| line);
        let Some(first) = lines.next() else {
            continue;
        };
        let (start, end) = lines.fold((first, first), |(start, end), line| {
            (start.min(line), end.max(line))
        });
        mappings.push(ExplorerLineMapping {
            function: name.to_owned(),
            source:   ExplorerLineRange { start, end },
            ir:       ir.and_then(|text| ir_function(text, name)),
            ir_abi:   ir_abi.and_then(|text| ir_function(text, name)),
            asm:      asm.and_then(|text| asm_function(text, name)),
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
