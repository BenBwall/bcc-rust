//! Maps each function definition's source lines to the lines its body
//! became in the IR before and after ABI lowering and in the assembly: a
//! function's header through its closing line in each text. Each line of
//! those texts also maps on its own ([`ir_lines`], [`asm_lines`]), from the
//! debug location the optimizer carries on each instruction, through inlining
//! too, and which the native back end follows to its listing lines.
//! Source lines are the editor's, physical lines of the input that `#line`
//! does not change; a function that a macro expansion begins starts at the
//! invocation, not in the macro's definition. A function is found in each
//! output under the assembler name it is emitted with, which
//! `__asm__("name")` can change.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "The explorer hands owned text and vectors across its interface, outside every arena."
)]

use super::{
    ExplorerLineMapping,
    ExplorerLineOrigin,
    ExplorerLineRange,
    compilation::INPUT,
};
use crate::{
    backend::native::{
        InstSource,
        ListingLine,
    },
    ir::{
        DebugFile,
        DebugInfo,
        DebugLoc,
        FuncId,
        Module,
        Scope,
        SourceLocation,
    },
    translation_phases::{
        Context,
        SourceVector,
        parsing::{
            ExternalDeclaration,
            ParsedTranslationUnit,
        },
        semantic_analysis::SemanticTranslationUnit,
    },
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

/// For each line of `text`, the IR `module` prints as, where it came from:
/// an instruction its debug location, and a block label or a declaration
/// nothing. [`only_within`] then keeps the lines of the functions the input
/// defines.
pub(super) fn ir_lines(module: &Module<'_>, text: &str) -> Vec<ExplorerLineOrigin> {
    let lines: Vec<&str> = text.lines().collect();
    let mut origins = vec![ExplorerLineOrigin::default(); lines.len()];
    let locations = Locations::of(module);
    // The printer writes the functions in module order, each from its
    // `function` line; one with a body follows it with one line per stack
    // slot and per block label, then each block's label line and one line
    // per instruction (instruction text escapes control characters).
    let mut headers = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("function "))
        .map(|(index, _)| index);
    for (_, function) in module.functions() {
        let Some(start) = headers.next() else {
            break;
        };
        let Some(body) = &function.body else {
            continue;
        };
        let mut at = start + 1 + body.stack_slots().count() + body.labels().count();
        for block in body.blocks() {
            at += 1;
            for &inst in body.block_insts(block) {
                if let Some(origin) = origins.get_mut(at) {
                    *origin = locations.origin(body.inst_location(inst));
                }
                at += 1;
            }
        }
        debug_assert_eq!(
            lines.get(at).copied(),
            Some("}"),
            "the IR layout of {}",
            lines[start]
        );
    }
    origins
}

/// For each line of `text`, the listing the native back end wrote for
/// `module`, where it came from, given what each line was `listed` for: an
/// instruction its IR instruction's debug location, the function's entry code
/// the line its body opens on (where a debugger reports the prologue, as with
/// Clang), a move the register allocator added nothing, and a label or a
/// directive nothing. [`only_within`] then keeps the lines of the functions
/// the input defines.
pub(super) fn asm_lines(
    module: &Module<'_>,
    text: &str,
    listed: &[ListingLine],
) -> Vec<ExplorerLineOrigin> {
    let lines: Vec<&str> = text.lines().collect();
    let mut origins = vec![ExplorerLineOrigin::default(); lines.len()];
    let locations = Locations::of(module);
    for &ListingLine {
        line,
        function,
        source,
    } in listed
    {
        let line = line as usize;
        if !lines.get(line).is_some_and(|text| is_instruction(text)) {
            continue;
        }
        origins[line] = match source {
            | InstSource::Allocator => locations.origin(None),
            | InstSource::Entry => locations.entry(function),
            | InstSource::Ir(inst) => locations.origin(
                module
                    .function(function)
                    .body
                    .as_ref()
                    .and_then(|body| body.inst_location(inst)),
            ),
        };
    }
    origins
}

/// Whether a line of a listing is an instruction, rather than a label, a
/// directive or a comment.
fn is_instruction(line: &str) -> bool {
    let line = line.trim();
    !(line.is_empty() || line.ends_with(':') || line.starts_with(['.', '#']))
}

/// A module's debug locations, read as lines of the input.
struct Locations<'m, 'ir> {
    debug: &'m DebugInfo<'ir>,
    /// The input's file, which is the only one the editor shows.
    input: Option<DebugFile>,
}

impl<'m, 'ir> Locations<'m, 'ir> {
    fn of(module: &'m Module<'ir>) -> Self {
        let debug = module.debug_info();
        Self {
            debug,
            input: debug
                .files()
                .find(|&(_, name)| name == INPUT)
                .map(|(file, _)| file),
        }
    }

    /// Where an instruction at `location` came from.
    fn origin(&self, location: Option<DebugLoc>) -> ExplorerLineOrigin {
        let mut origin = ExplorerLineOrigin {
            code: true,
            ..ExplorerLineOrigin::default()
        };
        let Some(mut location) = location.and_then(|id| self.debug.location_data(id)) else {
            return origin;
        };
        origin.line = self.line(&location);
        while let Some(outer) = location
            .inlined_at
            .and_then(|id| self.debug.location_data(id))
        {
            origin.call = self.line(&outer);
            location = outer;
        }
        if origin.line == 0 {
            origin.line = origin.call;
        }
        origin
    }

    /// Where `function`'s entry code came from: the line its body opens on.
    fn entry(&self, function: FuncId) -> ExplorerLineOrigin {
        let scope = self
            .debug
            .subprogram_of(function)
            .and_then(|scope| self.debug.scope(scope));
        let line = match scope {
            | Some(Scope::Subprogram(subprogram)) if Some(subprogram.file) == self.input =>
                subprogram.scope_line,
            | _ => 0,
        };
        ExplorerLineOrigin {
            line,
            code: true,
            ..ExplorerLineOrigin::default()
        }
    }

    /// `location`'s line, if it is in the input.
    fn line(&self, location: &SourceLocation) -> u32 {
        if Some(location.file) == self.input {
            location.line
        } else {
            0
        }
    }
}

/// Clears `lines` outside `functions`, the 1-based output ranges of the
/// functions defined in the input: a function from a header is not code the
/// editor shows.
pub(super) fn only_within(
    lines: &mut [ExplorerLineOrigin],
    functions: impl Iterator<Item = ExplorerLineRange>,
) {
    let mut keep = vec![false; lines.len()];
    for range in functions {
        for index in range.start.saturating_sub(1)..range.end {
            if let Some(kept) = usize::try_from(index)
                .ok()
                .and_then(|index| keep.get_mut(index))
            {
                *kept = true;
            }
        }
    }
    for (line, kept) in lines.iter_mut().zip(keep) {
        if !kept {
            *line = ExplorerLineOrigin::default();
        }
    }
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
