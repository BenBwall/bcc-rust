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
        Expansion,
        ExpansionStep,
        FunctionLines,
        Include,
        LogicalLine,
        Preprocessing,
        SourceChange,
        SourceSpan,
    },
    protocol::{
        Answer,
        Command,
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
        | Command::Load(_) | Command::Step =>
            Err("The compiler Worker does not run the abstract machine.".into()),
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
    let logical = output.logical_lines.as_deref().unwrap_or_default();
    let (pp_tokens, pp_ranges, pp_counts) = match output.pp_tokens.as_deref() {
        | Some(tokens) => {
            let (listing, ranges, counts) = pp_listing(tokens, source);
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
            .map(|line| logical_line(line, source))
            .collect(),
        pp_tokens,
        pp_comments: output
            .pp_tokens
            .iter()
            .flatten()
            .zip(&pp_ranges)
            .filter(|(token, _)| token.comment)
            .map(|(_, range)| *range)
            .collect(),
        pp_ranges,
        pp_counts,
        preprocessed: output.preprocessed,
        preprocessing: output
            .preprocessing
            .as_ref()
            .map(|trace| preprocessing(trace, source))
            .unwrap_or_default(),
        code_bytes: output
            .machine_code
            .iter()
            .flatten()
            .map(|function| function.bytes.len())
            .sum(),
        tokens: output.tokens,
        syntax_tree: output.syntax_tree,
        ir: output.ir,
        ir_abi: output.ir_abi,
        llvm: None,
        asm: output.asm,
        machine_code,
        diagnostics: output
            .diagnostics
            .iter()
            .map(|diagnostic| self::diagnostic(diagnostic, source))
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
fn logical_line(line: &ExplorerLogicalLine, source: &str) -> LogicalLine {
    LogicalLine {
        source:  span(line.source),
        changes: line
            .changes
            .iter()
            .map(|change| {
                let at = |column: u32| utf16_column(source, change.line as usize, column as usize);
                let length = change.spelling.chars().count().max(1) as u32;
                // A splice ends its line; mark its backslash alone.
                let length = if change.kind == "splice" { 1 } else { length };
                SourceChange {
                    kind:        change.kind.clone(),
                    column:      utf16_column(&line.text, 1, change.column as usize),
                    source:      SourceSpan {
                        line:       change.line as usize,
                        column:     at(change.source_column),
                        end_line:   change.line as usize,
                        end_column: at(change.source_column + length),
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

/// Phase 3's entries as a listing, one per line: where each starts, its
/// kind, its spelling, and what is notable about it. Also each entry's
/// range in the source and how many entries of each kind there are, with
/// the comments among the white space counted apart.
fn pp_listing(
    tokens: &[ExplorerPpToken],
    source: &str,
) -> (String, Vec<SourceSpan>, Vec<(String, usize)>) {
    let mut text = String::new();
    let mut ranges = Vec::with_capacity(tokens.len());
    for token in tokens {
        let place = format!("{}:{}", token.line, token.column);
        let spelling = match token.kind.as_str() {
            | "whitespace" => "' '".to_owned(),
            | "newline" => "'\\n'".to_owned(),
            | _ => token.spelling.clone(),
        };
        let mut notes = Vec::new();
        if token.line_start {
            notes.push(if token.spelling == "#" {
                "line start: a directive".to_owned()
            } else {
                "line start".to_owned()
            });
        }
        if token.leading_space {
            notes.push("after space".to_owned());
        }
        if token.comment {
            notes.push("a comment, now one space".to_owned());
        }
        if token.header_name {
            notes.push("read by #include as a header name".to_owned());
        }
        if token.end_line > token.line && token.kind != "newline" {
            notes.push(format!(
                "spliced from lines {}–{}",
                token.line, token.end_line
            ));
        }
        if let Some(name) = &token.canonical {
            notes.push(format!("a universal character name for {name}"));
        }
        if token.kind == "other" {
            notes.push("no other category fits".to_owned());
        }
        let line = format!(
            "{place:<8}{:<20}{spelling:<14}{}",
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
            column: utf16_column(source, line, column),
            end_line,
            end_column: utf16_column(source, end_line, end_column),
        });
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

/// What phase 4 did in the source, placed in the editor's coordinates.
/// Headers' own expansions and conditionals are left out: the view is about
/// the program, and a header's are many.
fn preprocessing(trace: &ExplorerPreprocessing, source: &str) -> Preprocessing {
    let input = |file: &str| file == crate::INPUT_NAME;
    Preprocessing {
        expansions:   trace
            .expansions
            .iter()
            .filter(|expansion| input(&expansion.file))
            .map(|expansion| Expansion {
                range:      SourceSpan {
                    line:       expansion.line as usize,
                    column:     utf16_column(
                        source,
                        expansion.line as usize,
                        expansion.column as usize,
                    ),
                    end_line:   expansion.end_line as usize,
                    end_column: utf16_column(
                        source,
                        expansion.end_line as usize,
                        expansion.end_column as usize,
                    ),
                },
                invocation: expansion.invocation.clone(),
                result:     expansion.result.clone(),
                steps:      expansion
                    .steps
                    .iter()
                    .map(|step| ExpansionStep {
                        kind:       step.kind.clone(),
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
            })
            .collect(),
        conditionals: trace
            .conditionals
            .iter()
            .filter(|conditional| input(&conditional.file))
            .map(|conditional| Conditional {
                line:      conditional.line as usize,
                directive: conditional.directive.clone(),
                taken:     conditional.taken,
            })
            .collect(),
        skipped:      trace
            .skipped
            .iter()
            .filter(|skipped| input(&skipped.file))
            .map(|skipped| span(skipped.lines))
            .collect(),
        includes:     trace
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
        definitions:  trace
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
fn diagnostic(diagnostic: &ExplorerDiagnostic, source: &str) -> CompileDiagnostic {
    let in_source = diagnostic.file.as_deref() == Some(crate::INPUT_NAME);
    let range = match (in_source, diagnostic.line, diagnostic.column) {
        | (true, Some(line), Some(column)) => {
            let line = line as usize;
            let column = utf16_column(source, line, column as usize);
            let end_line = diagnostic.end_line.map_or(line, |end| end as usize);
            let end_column = diagnostic
                .end_column
                .map(|end| utf16_column(source, end_line, end as usize));
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

/// The 1-based UTF-16 column of 1-based character `column` on 1-based
/// `line` of `source`. A column past the line's end keeps counting past it.
fn utf16_column(source: &str, line: usize, column: usize) -> usize {
    let text = source.split('\n').nth(line.saturating_sub(1)).unwrap_or("");
    let mut characters = text.chars();
    let prefix: usize = characters
        .by_ref()
        .take(column.saturating_sub(1))
        .map(char::len_utf16)
        .sum();
    let past_end = column
        .saturating_sub(1)
        .saturating_sub(text.chars().count());
    prefix + past_end + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Stage;

    #[test]
    fn columns_count_utf16_units() {
        let source = "int x;\n/* é𝔸 */ int y\n";
        assert_eq!(utf16_column(source, 1, 5), 5);
        // `é` is one unit and `𝔸` two, so character 9 is unit 10.
        assert_eq!(utf16_column(source, 2, 9), 10);
        assert_eq!(utf16_column(source, 2, 30), 31);
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
