//! bcc's own WebAssembly back end, run in the compiler Worker:
//! `bcc_rust::explorer::compile` for `wasm32-wasip1` at `-O0`, the same
//! compiler as `bcc-rust --target=wasm32-wasip1`, built without LLVM.
//!
//! [`module`] turns its [`ExplorerWasm`] into the stepper's
//! [`WasmModule`]: the binary as it is, one line row per source-map
//! position (bcc records lines but not columns, and a prologue has no line,
//! which the stepper spells 0), and bcc's `.wat` with the line of every
//! instruction, which the WebAssembly view lists in place of the stepper's
//! own printout.

use bcc_rust::explorer::{
    self,
    ExplorerDiagnostic,
    ExplorerOptions,
    ExplorerWasm,
};

use super::{
    WasmBackEnd,
    WasmFunction,
    WasmLine,
    WasmModule,
    WasmText,
    WasmTextLine,
};
use crate::demos;

/// The target bcc compiles for: a WASI preview 1 command, which the
/// stepper's host runs (`_start`, then `proc_exit` with `main`'s status).
pub const TARGET: &str = "wasm32-wasip1";

/// bcc, compiling any source in the browser.
pub struct Bcc;

impl WasmBackEnd for Bcc {
    fn compile(&self, source: &str) -> Result<WasmModule, String> {
        self.compile_with_diagnostics(source).0
    }
}

impl Bcc {
    /// [`WasmBackEnd::compile`], and every diagnostic bcc reported, which
    /// the editor draws for the WebAssembly view.
    pub fn compile_with_diagnostics(
        &self,
        source: &str,
    ) -> (Result<WasmModule, String>, Vec<ExplorerDiagnostic>) {
        let options = ExplorerOptions {
            target: TARGET.into(),
            max_nesting_depth: Some(explorer::WASM_MAX_NESTING_DEPTH),
            ..ExplorerOptions::default()
        };
        let output = explorer::compile(source, &options);
        let name = demos::matching(source).map_or("program.c", |demo| demo.name);
        let module = match output.wasm {
            | Some(wasm) => Ok(module(wasm, name, &options.std)),
            | None => Err(refusal(&output.diagnostics)),
        };
        (module, output.diagnostics)
    }
}

/// The stepper's module for bcc's `wasm`, compiled from `name` in language
/// mode `std`.
pub fn module(wasm: ExplorerWasm, name: &str, std: &str) -> WasmModule {
    let lines = wasm
        .positions
        .iter()
        .map(|position| WasmLine {
            offset: position.offset,
            line:   position.line.unwrap_or(0),
            column: 0,
        })
        .collect();
    let text = WasmText {
        instructions: wasm
            .instructions
            .iter()
            .map(|instruction| WasmTextLine {
                offset: instruction.offset,
                line:   instruction.text_line,
            })
            .collect(),
        functions:    wasm
            .functions
            .into_iter()
            .map(|function| WasmFunction {
                index: function.index,
                name:  function.name,
                start: function.start,
                end:   function.end,
            })
            .collect(),
        text:         wasm.text,
    };
    WasmModule {
        bytes:    wasm.binary,
        lines:    Some(lines),
        producer: "bcc".into(),
        command:  format!(
            "$ bcc-rust {name} --target={TARGET} -std={std} -O0 -o {}",
            name.replace(".c", ".wasm")
        ),
        text:     Some(text),
    }
}

/// Why bcc has no module: its errors as it prints them.
fn refusal(diagnostics: &[ExplorerDiagnostic]) -> String {
    let errors: Vec<&str> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "error")
        .map(|diagnostic| diagnostic.rendered.trim_end())
        .collect();
    if errors.is_empty() {
        "bcc produced no WebAssembly module for this program.".into()
    } else {
        errors.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use bcc_rust::explorer::{
        ExplorerWasmFunction,
        ExplorerWasmInstruction,
        ExplorerWasmPosition,
    };

    use super::*;

    #[test]
    fn positions_become_line_rows_and_the_text_keeps_its_instruction_lines() {
        let position = |offset, block, line| ExplorerWasmPosition {
            offset,
            function: 1,
            block,
            inst: None,
            line,
        };
        let wasm = ExplorerWasm {
            binary:       b"\0asm\x01\0\0\0".to_vec(),
            text:         "(module\n  (func $main\n    i32.const 1\n    end))\n".into(),
            functions:    vec![ExplorerWasmFunction {
                index:  1,
                name:   "main".into(),
                start:  40,
                end:    44,
                source: None,
            }],
            positions:    vec![position(40, None, None), position(42, Some(0), Some(3))],
            instructions: vec![
                ExplorerWasmInstruction {
                    offset:    40,
                    text_line: 3,
                },
                ExplorerWasmInstruction {
                    offset:    43,
                    text_line: 4,
                },
            ],
        };
        let module = module(wasm, "objects.c", "c17");
        assert_eq!(module.bytes, b"\0asm\x01\0\0\0");
        // A prologue has no line, which the stepper spells 0; bcc records
        // no columns.
        assert_eq!(
            module.lines,
            Some(vec![
                WasmLine {
                    offset: 40,
                    line:   0,
                    column: 0,
                },
                WasmLine {
                    offset: 42,
                    line:   3,
                    column: 0,
                },
            ])
        );
        assert_eq!(module.producer, "bcc");
        assert_eq!(
            module.command,
            "$ bcc-rust objects.c --target=wasm32-wasip1 -std=c17 -O0 -o objects.wasm"
        );
        let text = module.text.expect("bcc's text");
        assert!(text.text.starts_with("(module"));
        assert_eq!(
            text.instructions[1],
            WasmTextLine {
                offset: 43,
                line:   4,
            }
        );
        assert_eq!(
            text.functions,
            vec![WasmFunction {
                index: 1,
                name:  "main".into(),
                start: 40,
                end:   44,
            }]
        );
    }

    #[test]
    fn a_program_bcc_refuses_for_webassembly_is_its_diagnostic() {
        let source = "#include <setjmp.h>\njmp_buf here;\nint main(void) {\n    if (setjmp(here)) \
                      return 1;\n    longjmp(here, 1);\n}\n";
        let error = Bcc.compile(source).expect_err("no setjmp on WebAssembly");
        assert!(error.contains("error"), "{error}");
        assert!(error.contains("setjmp"), "{error}");
        let error = Bcc.compile("int main(void) { return x; }").unwrap_err();
        assert!(error.contains('x'), "{error}");
    }
}
