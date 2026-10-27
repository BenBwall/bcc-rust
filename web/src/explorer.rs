//! The compiler Worker's side: run bcc's real compiler
//! (`bcc_rust::explorer::compile`, its pure-Rust build without LLVM) over the
//! page's source and present its output as a [`Compilation`].
//!
//! The text stages carry over unchanged, with the syntax tree's locations on
//! so it links to the source line by line. Each function's machine code
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
    ExplorerOptions,
};

use crate::{
    pipeline::{
        Compilation,
        CompileDiagnostic,
        FunctionLines,
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
    Compilation {
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
