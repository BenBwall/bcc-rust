//! A WebAssembly module over the explorer for `run.mjs`, which instantiates
//! it in Node without `wasm-bindgen`: the host writes the source into memory
//! from `smoke_alloc` and reads `smoke_compile`'s report, a little-endian
//! length followed by that many bytes of text. See `run.mjs` for the build.

use std::fmt::Write as _;

use bcc_rust::explorer::{
    ExplorerOptions,
    ExplorerOutput,
    compile,
};

/// Compiles the `length` bytes at `source` for `wasm32-wasip1` and returns
/// a report of its diagnostics, the WebAssembly text, the number of source
/// map positions with a line, and the binary module in hexadecimal.
///
/// # Safety
///
/// `source` must hold `length` initialized bytes from [`smoke_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn smoke_compile_wasm(source: *mut u8, length: usize, optimize: u32) -> *const u8 {
    // SAFETY: The host wrote `length` bytes into the allocation.
    let bytes = unsafe { Vec::from_raw_parts(source, length, length) };
    let source = String::from_utf8_lossy(&bytes);
    let output = compile(&source, &ExplorerOptions {
        target: "wasm32-wasip1".into(),
        optimize: optimize != 0,
        ..ExplorerOptions::default()
    });
    let mut report = String::new();
    for diagnostic in &output.diagnostics {
        _ = write!(report, "== diagnostic {}\n{}", diagnostic.severity, diagnostic.rendered);
    }
    if let Some(wasm) = &output.wasm {
        let lines = wasm.positions.iter().filter(|position| position.line.is_some()).count();
        _ = write!(report, "== wat\n{}== positions {lines}\n== binary\n", wasm.text);
        for byte in &wasm.binary {
            _ = write!(report, "{byte:02x}");
        }
        report.push('\n');
    }
    framed(&report)
}

/// Room for `length` bytes of source, which the host fills before calling
/// [`smoke_compile`].
#[unsafe(no_mangle)]
pub extern "C" fn smoke_alloc(length: usize) -> *mut u8 {
    Vec::<u8>::with_capacity(length).leak().as_mut_ptr()
}

/// Compiles the `length` bytes at `source` (from [`smoke_alloc`]) and returns
/// the report, or with `optimize` set, as `-O2` does. A nonzero `depth` is the
/// nesting limit. A zero `trace` leaves out the phase-4 trace.
///
/// # Safety
///
/// `source` must hold `length` initialized bytes from [`smoke_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn smoke_compile(
    source: *mut u8,
    length: usize,
    optimize: u32,
    depth: u32,
    trace: u32,
) -> *const u8 {
    // SAFETY: The host wrote `length` bytes into the allocation.
    let bytes = unsafe { Vec::from_raw_parts(source, length, length) };
    let source = String::from_utf8_lossy(&bytes);
    let output = compile(&source, &ExplorerOptions {
        optimize: optimize != 0,
        max_nesting_depth: (depth != 0).then_some(depth),
        trace: trace != 0,
        ..ExplorerOptions::default()
    });
    let mut report = String::new();
    for diagnostic in &output.diagnostics {
        _ = write!(report, "== diagnostic {}\n{}", diagnostic.severity, diagnostic.rendered);
    }
    if let Some(lines) = &output.logical_lines {
        _ = writeln!(report, "== logical-lines {}", lines.len());
        for line in lines.iter().filter(|line| !line.changes.is_empty()) {
            for change in &line.changes {
                _ = writeln!(report, "{}-{}: {}", line.source.start, line.source.end, change.kind);
            }
        }
    }
    if let Some(tokens) = &output.pp_tokens {
        _ = writeln!(report, "== pp-tokens {}", tokens.len());
    }
    if let Some(trace) = &output.preprocessing {
        if trace.truncated {
            _ = writeln!(report, "== trace truncated");
        }
        _ = writeln!(report, "== expansions");
        for expansion in trace.expansions.iter().filter(|expansion| expansion.file == "<input>") {
            let partial = if expansion.partial { "partial: " } else { "" };
            _ = writeln!(report, "{partial}{} -> {}", expansion.invocation, expansion.result);
        }
    }
    for (name, text) in [
        ("preprocessed", &output.preprocessed),
        ("tokens", &output.tokens),
        ("syntax-tree", &output.syntax_tree),
        ("ir", &output.ir),
        ("ir-abi", &output.ir_abi),
        ("asm", &output.asm),
    ] {
        if let Some(text) = text {
            _ = write!(report, "== {name}\n{text}");
        }
    }
    for function in output.machine_code.iter().flatten() {
        _ = writeln!(report, "== machine-code {}", function.name);
        for instruction in &function.instructions {
            let bytes: Vec<_> = instruction.bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            _ = writeln!(report, "{:4x}: {:<24} {}", instruction.offset, bytes.join(" "), instruction.asm);
        }
    }
    framed(&report)
}

/// Compiles as [`smoke_compile`] does, at `-O0` with the default nesting
/// limit, for `wasm32-wasip1` with `wasm` set, and returns every output but
/// the phase-4 trace, as JSON: what tracing must leave unchanged.
///
/// # Safety
///
/// As for [`smoke_compile`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn smoke_compile_json(source: *mut u8, length: usize, trace: u32, wasm: u32) -> *const u8 {
    // SAFETY: The host wrote `length` bytes into the allocation.
    let bytes = unsafe { Vec::from_raw_parts(source, length, length) };
    let source = String::from_utf8_lossy(&bytes);
    let defaults = ExplorerOptions::default();
    let output = compile(&source, &ExplorerOptions {
        target: if wasm != 0 { "wasm32-wasip1".into() } else { defaults.target.clone() },
        trace: trace != 0,
        ..defaults
    });
    let json = serde_json::to_string(&ExplorerOutput {
        preprocessing: None,
        ..output
    })
    .unwrap_or_default();
    framed(&json)
}

/// `text` after its little-endian length, leaked for the host to read.
fn framed(text: &str) -> *const u8 {
    let mut framed = u32::try_from(text.len()).unwrap_or(u32::MAX).to_le_bytes().to_vec();
    framed.extend_from_slice(text.as_bytes());
    framed.leak().as_ptr()
}
