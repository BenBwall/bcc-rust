//! The compiler Worker's side: run bcc's real compiler
//! (`bcc_rust::explorer::compile`, its pure-Rust build without LLVM) over the
//! page's source and present its output as a [`Compilation`].
//!
//! The text stages carry over unchanged, with the syntax tree's locations on
//! so it links to the source line by line. The front end's phases become
//! views of their own: the logical lines of phases 1-2, one line each; phase
//! 3's preprocessing tokens as an aligned listing; and the preprocessed
//! text with what phase 4 did in the source. Each function's machine code
//! becomes one listing in the format `web/pipeline/generate.sh` used to write
//! from `objdump`. Diagnostics move from the compiler's character columns to
//! the editor's UTF-16 columns, and the compiler's per-function line map
//! gains the listing's lines. `llvm` stays `None`: the page adds the
//! precomputed LLVM IR for the bundled lessons.

use std::fmt::Write;

use bcc_rust::explorer::{
    self,
    ExplorerDiagnostic,
    ExplorerExpansion,
    ExplorerFunctionBytes,
    ExplorerLineRange,
    ExplorerLogicalLine,
    ExplorerOptions,
    ExplorerPpToken,
    ExplorerPreprocessing,
};

use crate::{
    pipeline::{
        Compilation,
        CompileDiagnostic,
        Conditional,
        Definition,
        Directive,
        Expansion,
        ExpansionStep,
        FunctionLines,
        Include,
        LogicalLine,
        OutputLine,
        Preprocessing,
        SourceChange,
        SourceSpan,
    },
    protocol::{
        Answer,
        Command,
    },
    wasm::{
        Bcc,
        WasmCompiled,
    },
};

/// Run one request from the page.
///
/// # Errors
/// For the abstract machine's commands, which its own Worker runs.
pub fn run(command: Command) -> Result<Answer, String> {
    match command {
        | Command::Compile { source, optimize } =>
            Ok(Answer::Compiled(Box::new(compile(&source, optimize)))),
        | Command::CompileWasm(source) => Ok(Answer::CompiledWasm(Box::new(compile_wasm(&source)))),
        | Command::Load(_) | Command::Step | Command::WasmLoad(_) | Command::Wasm(_) =>
            Err("The compiler Worker does not run the abstract machine.".into()),
    }
}

/// Compile `source` to WebAssembly with bcc (`wasm/bcc.rs`), its
/// diagnostics placed in the editor.
pub fn compile_wasm(source: &str) -> WasmCompiled {
    let (module, diagnostics) = Bcc.compile_with_diagnostics(source);
    let columns = Columns::new(source);
    WasmCompiled {
        module,
        diagnostics: diagnostics
            .iter()
            .map(|found| diagnostic(found, &columns))
            .collect(),
    }
}

/// Compile `source` with bcc, at `-O2` when `optimize`. Malformed source
/// yields diagnostics; the explorer limits source size and nesting depth
/// itself.
pub fn compile(source: &str, optimize: bool) -> Compilation {
    let output = explorer::compile(
        source,
        &ExplorerOptions {
            optimize,
            // The same limit on every host, so the native tests see what the
            // browser does.
            max_nesting_depth: Some(explorer::WASM_MAX_NESTING_DEPTH),
            syntax_locations: true,
            ..ExplorerOptions::default()
        },
    );
    let (machine_code, listed) = output.machine_code.as_deref().map(listing).unzip();
    let listed = listed.unwrap_or_default();
    let lines = |range: Option<ExplorerLineRange>| range.map(span);
    let columns = Columns::new(source);
    let logical = output.logical_lines.as_deref().unwrap_or_default();
    let all_tokens = output.pp_tokens.as_deref().unwrap_or_default();
    let (pp_tokens, pp_ranges, pp_counts) = match output.pp_tokens.as_deref() {
        | Some(tokens) => {
            let (listing, ranges, counts) = pp_listing(tokens, &columns);
            (Some(listing), ranges, counts)
        },
        | None => (None, Vec::new(), Vec::new()),
    };
    Compilation {
        logical: output.logical_lines.as_ref().map(|lines| {
            lines
                .iter()
                .flat_map(|line| [line.text.as_str(), "\n"])
                .collect()
        }),
        logical_lines: logical
            .iter()
            .map(|line| logical_line(line, &columns))
            .collect(),
        pp_tokens,
        pp_comments: all_tokens
            .iter()
            .zip(&pp_ranges)
            .filter(|(token, _)| token.comment)
            .map(|(_, range)| *range)
            .collect(),
        pp_ranges,
        pp_counts,
        preprocessed_lines: output
            .preprocessed_lines
            .iter()
            .flatten()
            .map(|line| OutputLine {
                input:   line.file == crate::INPUT_NAME,
                line:    line.line.map(|line| line as usize),
                include: line.include.map(|line| line as usize),
            })
            .collect(),
        preprocessed: output.preprocessed,
        preprocessing: output
            .preprocessing
            .as_ref()
            .map(|trace| preprocessing(trace, &columns))
            .unwrap_or_default(),
        code_bytes: output
            .machine_code
            .iter()
            .flatten()
            .map(|function| function.bytes.len())
            .sum(),
        tokens: output.tokens,
        syntax_tree: output.syntax_tree,
        semantic_types: output.semantic_types,
        ir: output.ir,
        ir_abi: output.ir_abi,
        ir_lines: output.ir_lines.iter().map(|&line| line as usize).collect(),
        ir_abi_lines: output
            .ir_abi_lines
            .iter()
            .map(|&line| line as usize)
            .collect(),
        llvm: None,
        asm: output.asm,
        machine_code,
        diagnostics: output
            .diagnostics
            .iter()
            .map(|diagnostic| self::diagnostic(diagnostic, &columns))
            .collect(),
        functions: output
            .line_map
            .iter()
            .map(|mapping| FunctionLines {
                name:         mapping.function.clone(),
                source:       span(mapping.source),
                ir:           lines(mapping.ir),
                ir_abi:       lines(mapping.ir_abi),
                asm:          lines(mapping.asm),
                machine_code: listed
                    .iter()
                    .find(|(name, _)| *name == mapping.function)
                    .map(|(_, lines)| *lines),
            })
            .collect(),
    }
}

fn span(range: ExplorerLineRange) -> (usize, usize) {
    (range.start as usize, range.end as usize)
}

/// A logical line's physical lines and changes, in UTF-16 columns.
fn logical_line(line: &ExplorerLogicalLine, columns: &Columns) -> LogicalLine {
    let in_line = Columns::new(&line.text);
    LogicalLine {
        source:  span(line.source),
        changes: line
            .changes
            .iter()
            .map(|change| {
                let at = |column: usize| columns.utf16(change.line as usize, column);
                let length = change.spelling.chars().count().max(1);
                // A splice ends its line; mark its backslash alone.
                let length = if change.kind == "splice" { 1 } else { length };
                SourceChange {
                    kind:        change.kind.clone(),
                    column:      in_line.utf16(1, change.column as usize),
                    source:      SourceSpan {
                        line:       change.line as usize,
                        column:     at(change.source_column as usize),
                        end_line:   change.line as usize,
                        end_column: at(change.source_column as usize + length),
                    },
                    spelling:    change.spelling.clone(),
                    replacement: change.replacement.clone(),
                }
            })
            .collect(),
    }
}

/// The kinds of phase-3 entries, in the order the listing's counts give
/// them.
const PP_KINDS: [&str; 8] = [
    "identifier",
    "pp-number",
    "character-constant",
    "string-literal",
    "punctuator",
    "other",
    "whitespace",
    "newline",
];

/// The most phase-3 entries the listing shows. The page receives the
/// listing and a range for each entry, so a large source would otherwise
/// send megabytes for a view nobody scrolls through.
pub const MAX_PP_ENTRIES: usize = 20_000;

/// The most expansions the page receives, for the same reason.
pub const MAX_EXPANSIONS: usize = 2_000;

/// The most steps the page receives for one expansion; a macro that
/// expands enormously can take tens of thousands.
pub const MAX_STEPS: usize = 500;

/// Phase 3's entries as a listing, one per line: where each starts, its
/// kind, its spelling, and what is notable about it, up to
/// [`MAX_PP_ENTRIES`] of them and then a line saying how many more there
/// are. Also each listed entry's range in the source and how many entries
/// of each kind there are, with the comments among the white space counted
/// apart.
fn pp_listing(
    tokens: &[ExplorerPpToken],
    columns: &Columns,
) -> (String, Vec<SourceSpan>, Vec<(String, usize)>) {
    let mut text = String::new();
    let shown = tokens.len().min(MAX_PP_ENTRIES);
    let mut ranges = Vec::with_capacity(shown);
    for token in &tokens[..shown] {
        let place = format!("{}:{}", token.line, token.column);
        let spelling = match token.kind.as_str() {
            | "whitespace" => "' '".to_owned(),
            | "newline" => "'\\n'".to_owned(),
            | _ => token.spelling.clone(),
        };
        let mut notes = Vec::new();
        if token.line_start {
            notes.push(if token.spelling == "#" {
                "starts a directive".to_owned()
            } else {
                "line start".to_owned()
            });
        }
        if token.leading_space {
            notes.push("after space".to_owned());
        }
        if token.comment {
            notes.push("comment: one space".to_owned());
        }
        if token.header_name {
            notes.push("header name".to_owned());
        }
        if token.end_line > token.line && token.kind != "newline" {
            notes.push(format!("spliced, lines {}–{}", token.line, token.end_line));
        }
        if let Some(name) = &token.canonical {
            notes.push(format!("names {name}"));
        }
        if token.kind == "other" {
            notes.push("fits no other kind".to_owned());
        }
        let line = format!(
            "{place:<8}{:<19}{spelling:<12}{}",
            token.kind,
            notes.join(" · ")
        );
        text.push_str(line.trim_end());
        text.push('\n');
        let (line, column) = (token.line as usize, token.column as usize);
        let (end_line, end_column) = if token.kind == "newline" {
            (line, column + 1)
        } else {
            (token.end_line as usize, token.end_column as usize)
        };
        ranges.push(SourceSpan {
            line,
            column: columns.utf16(line, column),
            end_line,
            end_column: columns.utf16(end_line, end_column),
        });
    }
    if tokens.len() > shown {
        let more = tokens.len() - shown;
        _ = writeln!(text, "… and {more} more entries, not listed here");
    }
    let mut counts: Vec<(String, usize)> = PP_KINDS
        .iter()
        .map(|kind| {
            let count = tokens.iter().filter(|token| token.kind == *kind).count();
            ((*kind).to_owned(), count)
        })
        .filter(|(_, count)| *count > 0)
        .collect();
    let comments = tokens.iter().filter(|token| token.comment).count();
    if comments > 0 {
        counts.push(("comment".to_owned(), comments));
    }
    (text, ranges, counts)
}

/// What phase 4 did in the source, placed in the editor's coordinates, with
/// at most [`MAX_EXPANSIONS`] expansions, always including the one the
/// trace stopped in. Headers' own expansions, conditionals and directives
/// are left out: the view is about the program, and a header's are many.
fn preprocessing(trace: &ExplorerPreprocessing, columns: &Columns) -> Preprocessing {
    let input = |file: &str| file == crate::INPUT_NAME;
    let in_source: Vec<_> = trace
        .expansions
        .iter()
        .filter(|expansion| input(&expansion.file))
        .collect();
    let total = in_source.len();
    let mut shown = in_source;
    if shown.len() > MAX_EXPANSIONS {
        let partial = shown[MAX_EXPANSIONS..]
            .iter()
            .copied()
            .find(|expansion| expansion.partial);
        shown.truncate(MAX_EXPANSIONS);
        if let Some(partial) = partial {
            shown[MAX_EXPANSIONS - 1] = partial;
        }
    }
    Preprocessing {
        omitted_expansions: total - shown.len(),
        truncated:          trace.truncated,
        expansions:         shown
            .into_iter()
            .map(|expansion| expansion_of(expansion, columns))
            .collect(),
        conditionals:       trace
            .conditionals
            .iter()
            .filter(|conditional| input(&conditional.file))
            .map(|conditional| Conditional {
                line:      conditional.line as usize,
                presumed:  conditional.presumed.clone(),
                directive: conditional.directive.clone(),
                taken:     conditional.taken,
                evaluated: conditional.evaluated,
            })
            .collect(),
        skipped:            trace
            .skipped
            .iter()
            .filter(|skipped| input(&skipped.file))
            .map(|skipped| span(skipped.lines))
            .collect(),
        includes:           trace
            .includes
            .iter()
            .filter(|include| input(&include.file))
            .map(|include| Include {
                line:   include.line as usize,
                header: include.header.clone(),
                angled: include.angled,
                found:  include.found.is_some(),
                system: include.system,
                lines:  include.lines as usize,
            })
            .collect(),
        definitions:        trace
            .definitions
            .iter()
            .filter(|definition| input(&definition.file))
            .map(|definition| Definition {
                line:      definition.line as usize,
                name:      definition.name.clone(),
                kind:      definition.kind.clone(),
                directive: definition.directive.clone(),
            })
            .collect(),
        directives:         trace
            .directives
            .iter()
            .filter(|directive| input(&directive.file))
            .map(|directive| Directive {
                line:     directive.line as usize,
                presumed: directive.presumed.clone(),
                name:     directive.name.clone(),
                text:     directive.directive.clone(),
            })
            .collect(),
    }
}

/// One expansion in the source, with at most `MAX_STEPS` of its steps.
fn expansion_of(expansion: &ExplorerExpansion, columns: &Columns) -> Expansion {
    Expansion {
        range:         SourceSpan {
            line:       expansion.line as usize,
            column:     columns.utf16(expansion.line as usize, expansion.column as usize),
            end_line:   expansion.end_line as usize,
            end_column: columns.utf16(expansion.end_line as usize, expansion.end_column as usize),
        },
        presumed:      expansion.presumed.clone(),
        invocation:    expansion.invocation.clone(),
        directive:     expansion.directive,
        result:        expansion.result.clone(),
        partial:       expansion.partial,
        omitted_steps: expansion.steps.len().saturating_sub(MAX_STEPS),
        steps:         expansion
            .steps
            .iter()
            .take(MAX_STEPS)
            .map(|step| ExpansionStep {
                parent:     step.parent.map(|parent| parent as usize),
                kind:       step.kind.clone(),
                prescan:    step.prescan,
                macro_name: step.macro_name.clone(),
                depth:      step.depth as usize,
                arguments:  step
                    .arguments
                    .iter()
                    .map(|argument| (argument.parameter.clone(), argument.argument.clone()))
                    .collect(),
                operands:   step.operands.clone(),
                result:     step.result.clone(),
            })
            .collect(),
    }
}

/// Each function's name and first and last line in a listing.
type Listed = Vec<(String, (usize, usize))>;

/// The machine-code listing: each function's label, then one line per
/// instruction with its offset in the function, its bytes and its assembly,
/// with a blank line between functions. Also each function's lines in it.
fn listing(functions: &[ExplorerFunctionBytes]) -> (String, Listed) {
    let mut text = String::new();
    let mut lines = Vec::new();
    let mut line = 0;
    for function in functions {
        if line > 0 {
            text.push('\n');
            line += 1;
        }
        _ = writeln!(text, "<{}>:", function.name);
        line += 1;
        let first = line;
        for instruction in &function.instructions {
            let bytes: Vec<String> = instruction
                .bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            _ = writeln!(
                text,
                "{:>6x}  {:<24} {}",
                instruction.offset,
                bytes.join(" "),
                instruction
                    .asm
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            line += 1;
        }
        lines.push((function.name.clone(), (first, line)));
    }
    (text, lines)
}

/// `diagnostic` with its range in the editor's coordinates, when it is in
/// `source` rather than a header.
fn diagnostic(diagnostic: &ExplorerDiagnostic, columns: &Columns) -> CompileDiagnostic {
    let in_source = diagnostic.file.as_deref() == Some(crate::INPUT_NAME);
    let range = match (in_source, diagnostic.line, diagnostic.column) {
        | (true, Some(line), Some(column)) => {
            let line = line as usize;
            let column = columns.utf16(line, column as usize);
            let end_line = diagnostic.end_line.map_or(line, |end| end as usize);
            let end_column = diagnostic
                .end_column
                .map(|end| columns.utf16(end_line, end as usize));
            // A range that ends where it starts would draw no squiggle.
            let end_column = end_column
                .filter(|&end| end_line > line || end > column)
                .unwrap_or(column + 1);
            Some(SourceSpan {
                line,
                column,
                end_line,
                end_column,
            })
        },
        | _ => None,
    };
    CompileDiagnostic {
        severity: diagnostic.severity.clone(),
        stage: diagnostic.stage.clone(),
        message: diagnostic.message.clone(),
        range,
        rendered: diagnostic.rendered.clone(),
    }
}

/// The source's lines, to turn the compiler's 1-based character columns
/// into the editor's 1-based UTF-16 columns in time proportional to the
/// line, not the source: a listing of every token of a large source asks
/// for two columns per token.
struct Columns {
    /// For each line with a character outside ASCII, the UTF-16 units
    /// before each of its characters; an ASCII line counts the same in
    /// both.
    tables: Vec<Option<Vec<usize>>>,
}

impl Columns {
    fn new(source: &str) -> Self {
        let tables = source
            .split('\n')
            .map(|line| {
                (!line.is_ascii()).then(|| {
                    std::iter::once(0)
                        .chain(line.chars().scan(0, |units, character| {
                            *units += character.len_utf16();
                            Some(*units)
                        }))
                        .collect()
                })
            })
            .collect();
        Self { tables }
    }

    /// The 1-based UTF-16 column of 1-based character `column` on 1-based
    /// `line`. A column past the line's end keeps counting past it.
    fn utf16(&self, line: usize, column: usize) -> usize {
        let index = line.saturating_sub(1);
        let before = column.saturating_sub(1);
        match self.tables.get(index) {
            | Some(Some(table)) => {
                let characters = table.len() - 1;
                table[before.min(characters)] + before.saturating_sub(characters) + 1
            },
            | _ => before + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Stage;

    #[test]
    fn columns_count_utf16_units() {
        let source = "int x;\n/* é𝔸 */ int y\n";
        let columns = Columns::new(source);
        assert_eq!(columns.utf16(1, 5), 5);
        // `é` is one unit and `𝔸` two, so character 9 is unit 10.
        assert_eq!(columns.utf16(2, 9), 10);
        assert_eq!(columns.utf16(2, 30), 31);
        assert_eq!(columns.utf16(9, 4), 4);
        assert_eq!(Columns::new("é𝔸x").utf16(1, 3), 4);
    }

    #[test]
    fn bundled_headers_fold_and_link_to_their_include() {
        use crate::pipeline::{
            displayed,
            is_folded,
            links,
        };
        let source = "int a;\n#include <stdint.h>\nint32_t x;\n";
        let compilation = compile(source, false);
        let (full, _) = displayed(Stage::Preprocessor, &compilation, true).unwrap();
        let (folded, lines) = displayed(Stage::Preprocessor, &compilation, false).unwrap();
        assert!(full.contains("typedef"), "{full}");
        assert!(!folded.contains("typedef"), "{folded}");
        let summary = folded
            .lines()
            .position(is_folded)
            .unwrap_or_else(|| panic!("{folded}"));
        assert!(folded.contains("<stdint.h>: "), "{folded}");
        assert_eq!(lines.unwrap()[summary + 1], Some((2, 2)));
        let folded_links = links(Stage::Preprocessor, &compilation, source, false);
        assert_eq!(folded_links.outputs_for(2), [summary + 1]);
        assert!(
            folded_links
                .outputs_for(3)
                .iter()
                .all(|&line| line > summary + 1)
        );
        // With the headers shown, each of their lines links to the include.
        let shown = links(Stage::Preprocessor, &compilation, source, true);
        let typedef = full
            .lines()
            .position(|line| line.starts_with("typedef"))
            .unwrap();
        assert_eq!(shown.sources_for(typedef + 1), [2]);
    }

    #[test]
    fn predefined_macros_and_other_directives_reach_the_page() {
        let compilation = compile("#pragma once\nint line = __LINE__;\n", false);
        let trace = &compilation.preprocessing;
        assert_eq!(trace.expansions[0].invocation, "__LINE__");
        assert_eq!(trace.expansions[0].result, "2");
        assert_eq!(trace.directives[0].name, "pragma");
    }

    #[test]
    fn an_enormous_expansion_sends_its_first_steps() {
        // 11,111 replacements in one expansion, of 30,000 elements.
        let source = "#define A 0,0,0,\n#define B A A A A A A A A A A\n#define C B B B B B B B B \
                      B B\n#define D C C C C C C C C C C\n#define E D D D D D D D D D D\nint a[] \
                      = {E 0};\n";
        let compilation = compile(source, false);
        let trace = &compilation.preprocessing;
        assert!(trace.truncated);
        let expansion = &trace.expansions[0];
        assert_eq!(expansion.steps.len(), MAX_STEPS);
        assert!(expansion.omitted_steps > 0);
    }

    #[test]
    fn the_expansion_the_trace_stopped_in_is_always_listed() {
        // Each empty replacement is one event, so the trace stops as the
        // invocation on about line 49,000 begins, far past the cap.
        let source = format!("#define E\n{}", "E\n".repeat(60_000));
        let trace = &compile(&source, false).preprocessing;
        assert!(trace.truncated);
        assert_eq!(trace.expansions.len(), MAX_EXPANSIONS);
        let last = trace.expansions.last().unwrap();
        assert!(
            last.partial && last.range.line > 40_000,
            "{}",
            last.range.line
        );
        assert!(
            trace.expansions[..MAX_EXPANSIONS - 1]
                .iter()
                .all(|e| !e.partial)
        );
        assert!(trace.omitted_expansions > 40_000);
    }

    #[test]
    fn a_long_line_maps_in_linear_time() {
        // One line of 288 KB, which took 6 s in the browser when each
        // entry recounted the line.
        let source = "int x;".repeat(48_000) + "\n";
        let started = std::time::Instant::now();
        let compilation = compile(&source, false);
        let elapsed = started.elapsed();
        let last = compilation.pp_ranges.last().unwrap();
        assert_eq!(last.line, 1);
        assert!(compilation.preprocessed.is_some());
        assert!(elapsed.as_secs() < 30, "{elapsed:?}");
    }

    #[test]
    fn a_large_source_maps_in_linear_time_and_its_listing_is_capped() {
        // Phase 3 makes three entries per line: a comment, a new-line and,
        // at the end, the declaration's tokens.
        let source = "/* hi */\n".repeat(40_000) + "int x;\n";
        let started = std::time::Instant::now();
        let compilation = compile(&source, false);
        let elapsed = started.elapsed();
        let listing = compilation.pp_tokens.as_deref().unwrap();
        assert_eq!(listing.lines().count(), MAX_PP_ENTRIES + 1);
        assert!(
            listing
                .trim_end()
                .ends_with("more entries, not listed here")
        );
        assert_eq!(compilation.pp_ranges.len(), MAX_PP_ENTRIES);
        // The whole compilation, not only the mapping; quadratic mapping
        // took minutes here.
        assert!(elapsed.as_secs() < 60, "{elapsed:?}");
    }

    #[test]
    fn every_lesson_compiles_live_at_both_levels() {
        for lesson in &crate::demos::DEMOS {
            for optimize in [false, true] {
                let compilation = compile(lesson.source, optimize);
                assert!(
                    compilation.diagnostics.is_empty(),
                    "{}: {:#?}",
                    lesson.name,
                    compilation.diagnostics
                );
                assert_eq!(compilation.stop(), None);
                for stage in Stage::ALL {
                    assert_eq!(
                        compilation.output(stage).is_some(),
                        stage != Stage::Llvm,
                        "{} {}",
                        lesson.name,
                        stage.key()
                    );
                }
                assert!(compilation.code_bytes > 0);
                assert!(
                    compilation
                        .functions
                        .iter()
                        .any(|function| function.name == "main"),
                    "{}",
                    lesson.name
                );
            }
        }
    }

    #[test]
    fn a_syntax_error_is_a_placed_diagnostic_that_stops_the_front_end() {
        let source = "int main(void) {\n    int é = 1\n    return 0;\n}\n";
        let compilation = compile(source, false);
        let error = compilation
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == "error")
            .expect("a missing semicolon is an error");
        assert_eq!(error.stage, "front-end");
        let range = error.range.expect("the error is in the source");
        assert!((2..=3).contains(&range.line), "{range:?}");
        assert!(range.end_line > range.line || range.end_column > range.column);
        assert_ne!(error.rendered, "");
        assert!(compilation.tokens.is_some() && compilation.syntax_tree.is_some());
        assert!(compilation.ir.is_none() && compilation.machine_code.is_none());
        let stop = compilation.stop().expect("the error stops bcc");
        assert_eq!(stop.phase, "Front end");
        assert!(stop.explanation.contains(&error.message));
        assert!(compilation.stopped_before(Stage::Ir));
        assert!(!compilation.stopped_before(Stage::Tokens));
    }

    /// `<stdio.h>` is one of bcc's own headers on wasm32-wasip1 but none on
    /// x86-64 in the browser: the WebAssembly view's squiggles come from its
    /// own compilation, and the pipeline's stop says where the header is.
    #[test]
    fn the_c_library_headers_are_found_only_for_webassembly() {
        let source =
            "#include <stdio.h>\nint main(void) {\n    printf(\"%d\\n\", 1);\n    return x;\n}\n";
        let wasm = compile_wasm(source);
        let [error] = &*wasm.diagnostics else {
            panic!("{:#?}", wasm.diagnostics);
        };
        // Only the undeclared `x` on line 4, placed for a squiggle.
        assert_eq!(error.severity, "error");
        let range = error.range.expect("in the source");
        assert_eq!((range.line, range.column), (4, 12), "{error:#?}");
        assert!(wasm.module.is_err());
        let fixed = source.replace("return x", "return 0");
        let wasm = compile_wasm(&fixed);
        assert!(wasm.diagnostics.is_empty(), "{:#?}", wasm.diagnostics);
        assert!(wasm.module.is_ok());
        // For x86-64 the header is missing, and the stop says where to go.
        let compilation = compile(&fixed, false);
        let error = compilation
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == "error")
            .expect("no <stdio.h> on x86-64");
        assert_eq!(error.range.map(|range| range.line), Some(1));
        let stop = compilation.stop().expect("the error stops bcc");
        assert!(
            stop.explanation.contains("<stdio.h>"),
            "{}",
            stop.explanation
        );
        assert!(
            stop.explanation.contains("choose WebAssembly"),
            "{}",
            stop.explanation
        );
    }

    #[test]
    fn deep_nesting_is_a_diagnostic() {
        let source = format!(
            "int main(void) {{ return {}1{}; }}\n",
            "(".repeat(5000),
            ")".repeat(5000)
        );
        let compilation = compile(&source, false);
        assert!(
            compilation
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == "error"),
            "{:#?}",
            compilation.diagnostics
        );
    }

    /// The whole-path lesson's notes describe its `-O0` output; this keeps
    /// them true when the compiler changes.
    #[test]
    fn the_path_lessons_notes_match_what_bcc_makes_of_it() {
        let demo = crate::demos::find("path").expect("the whole-path lesson");
        assert_eq!(demo.path.len(), Stage::ALL.len());
        let compilation = compile(demo.source, false);
        let text = |stage: Option<&String>| stage.cloned().expect("the stage compiled");
        let tokens = text(compilation.tokens.as_ref());
        assert_eq!(tokens.lines().filter(|line| !line.is_empty()).count(), 38);
        // Phases 1-2: no splices, and the comment is still in the text.
        assert_eq!(compilation.logical_lines.len(), 10);
        assert!(text(compilation.logical.as_ref()).starts_with("// One small program"));
        // Phase 3: `square(side + 1)`, line 8 columns 16 to 31, is six
        // preprocessing tokens.
        // Phase 3 also lists the white space between them.
        let line = demo.source.lines().nth(7).expect("line 8");
        let call = compilation
            .pp_ranges
            .iter()
            .filter(|range| range.line == 8 && (16..=31).contains(&range.column))
            .filter(|range| !line[range.column - 1..range.end_column - 1].trim().is_empty())
            .count();
        assert_eq!(call, 6, "{:?}", compilation.pp_ranges);
        let phase4 = &compilation.preprocessing;
        assert!(phase4.directives.is_empty() && phase4.expansions.is_empty());
        let tree = text(compilation.syntax_tree.as_ref());
        for expected in [
            "callee: identifier square",
            "argument: binary +",
            "return-value: binary *",
        ] {
            assert!(tree.contains(expected), "{expected}: {tree}");
        }
        let types = text(compilation.semantic_types.as_ref());
        for expected in [
            "function square type=\"int (int)\"",
            "parameter n type=int size=4",
            "object side type=int duration=automatic size=4",
            "object area type=int duration=automatic size=4",
            "kind=lvalue type=int",
        ] {
            assert!(types.contains(expected), "{expected}: {types}");
        }
        let ir = text(compilation.ir.as_ref());
        assert!(ir.contains("iadd.i32 nsw v0, v1") && ir.contains("call @square(v2)"), "{ir}");
        assert!(!ir.contains("stack_slot"), "{ir}");
        let body = |ir: &str| ir[ir.find("function @square").expect("square")..].to_owned();
        assert_eq!(body(&text(compilation.ir_abi.as_ref())), body(&ir));
        let llvm = crate::pipeline::precomputed_llvm("path", false).expect("bundled LLVM IR");
        assert!(llvm.contains("mul nsw i32 %v0, %v0"), "{llvm}");
        let asm = text(compilation.asm.as_ref());
        for expected in ["movq\t%rdi, %rax", "imull\t%edi, %eax", "callq\tsquare"] {
            assert!(asm.contains(expected), "{expected}: {asm}");
        }
        let code = text(compilation.machine_code.as_ref());
        // Each listing line is the offset, the bytes, then the instruction.
        let bytes_of = |mnemonic: &str| {
            code.lines()
                .find(|line| line.split_whitespace().any(|word| word.starts_with(mnemonic)))
                .map(|line| {
                    line.split_whitespace()
                        .skip(1)
                        .take_while(|word| word.len() == 2 && word.chars().all(|c| c.is_ascii_hexdigit()))
                        .collect::<Vec<_>>()
                })
        };
        assert_eq!(bytes_of("ret"), Some(vec!["c3"]), "{code}");
        assert_eq!(bytes_of("call"), Some(vec!["e8", "00", "00", "00", "00"]), "{code}");
    }

    #[test]
    fn the_listing_numbers_each_function_and_its_lines() {
        let compilation = compile(crate::demos::DEMOS[0].source, false);
        let listing = compilation.machine_code.as_deref().unwrap();
        let lines: Vec<&str> = listing.lines().collect();
        for function in &compilation.functions {
            let (first, last) = function.machine_code.expect("every function is listed");
            assert_eq!(lines[first - 1], format!("<{}>:", function.name));
            assert!(
                lines[last - 1]
                    .trim_start()
                    .starts_with(|c: char| c.is_ascii_hexdigit())
            );
            assert!(lines.get(last).is_none_or(|line| line.is_empty()));
        }
    }
}
