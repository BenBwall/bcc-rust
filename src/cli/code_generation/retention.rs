//! Prepare the complete output before accepting optional local-static roots.
//! No program instructions or stdout writes occur during an attempt. A failed
//! attempt can therefore be retried with the required emission closure.
//! C99: §5.1.1.3p1, p. 11; PDF p. 23. Retention policy follows Clang.

use super::{
    ArenaString,
    BackEnd,
    Bump,
    CodeGeneration,
    CodeOptions,
    Context,
    DefaultHost,
    Emit,
    FAILURE_STATUS,
    MiddleEndError,
    OutputKind,
    TRAP_STATUS,
    Target,
    fmt,
    interpreter,
    io,
    llvm,
    native,
};
use crate::{
    backend::wasm,
    lowering::CallDiagnostic,
    pipeline::DiagnosedModule,
    util::bump::ArenaVec,
};

/// Validate and generate the selected output, delaying diagnostics and text
/// until the caller accepts this attempt. Interpreter loading has no user
/// effects; actual execution happens only after the retry boundary.
pub(super) fn prepare<'ir>(
    diagnosed: &DiagnosedModule<'ir>,
    context: &Context<'_>,
    arena: &'ir Bump,
    target: Target,
    code: CodeOptions,
    options: &CodeGeneration,
) -> Result<Prepared<'ir>, Failure<'ir>> {
    let module = &diagnosed.module;
    if (target.is_wasm()
        || options.backend == BackEnd::Wasm
        || matches!(options.emit, Some(Emit::Wasm | Emit::Wat)))
        && let Some(prepared) = wasm(diagnosed, context, arena, target, options)?
    {
        return Ok(prepared);
    }
    let llvm_output =
        options.emit.is_none() && options.output_path.is_some() && options.backend == BackEnd::Llvm;
    let reports_calls =
        !llvm_output && !matches!(options.emit, Some(Emit::Ir | Emit::IrAbi | Emit::Llvm));
    let mut diagnostics = if reports_calls {
        diagnosed.retained_diagnostics
    } else {
        &[]
    };
    if diagnostics.iter().any(|d| d.is_error(context)) {
        return Err(Failure::Calls(diagnostics));
    }
    if matches!(options.emit, Some(Emit::Ir | Emit::IrAbi | Emit::Asm))
        || (options.emit.is_none() && options.output_path.is_none())
        || (options.emit.is_none() && options.backend == BackEnd::Native)
    {
        let scratch = Bump::new();
        let errors = crate::ir::verify_module(module, module.profile(), &scratch);
        if !errors.is_empty() {
            let mut text = ArenaString::new_in(arena);
            for error in &errors {
                _ = fmt::write(
                    &mut text,
                    format_args!("error: internal error: invalid bcc IR: {error}\n"),
                );
            }
            return Err(Failure::Message {
                text:   text.into_str(),
                status: FAILURE_STATUS,
            });
        }
    }
    let mut atomic_runtime = false;
    let output = match (options.emit, &options.output_path) {
        | (Some(Emit::Ir | Emit::IrAbi), _) => {
            let mut text = ArenaString::new_in(arena);
            _ = fmt::write(&mut text, format_args!("{module}"));
            Output::Text(text.into_str())
        },
        | (Some(Emit::Wasm | Emit::Wat), _) =>
            unreachable!("the WebAssembly back end handles these first"),
        | (Some(Emit::Llvm), _) => {
            let mut text = ArenaString::new_in(arena);
            llvm::emit_module_with(module, target, code, &mut text)
                .map_err(|e| message(arena, format_args!("error: {e}"), FAILURE_STATUS))?;
            Output::Text(text.into_str())
        },
        | (Some(Emit::Asm), _) => {
            let mut text = ArenaString::new_in(arena);
            if options.backend == BackEnd::Native {
                native::emit_assembly_with(module, target, code, &mut text)
                    .map_err(|e| message(arena, format_args!("error: {e}"), FAILURE_STATUS))?;
            } else {
                llvm::emit_assembly_text(module, options.llvm_options(target, code), &mut text)
                    .map_err(|e| message(arena, format_args!("error: {e}"), FAILURE_STATUS))?;
            }
            Output::Text(text.into_str())
        },
        | (None, Some(path)) => {
            let failure = |e: io::Error| {
                message(
                    arena,
                    format_args!("error: cannot build `{}`: {e}", path.display()),
                    FAILURE_STATUS,
                )
            };
            if options.backend == BackEnd::Native {
                if options.output_kind() == OutputKind::SharedLibrary {
                    return Err(message(
                        arena,
                        format_args!("error: {}", native::NativeError::SharedLibraryCode),
                        FAILURE_STATUS,
                    ));
                }
                let writer =
                    native::object_writer(module, target, options.requested_object_writer())
                        .map_err(|e| message(arena, format_args!("error: {e}"), FAILURE_STATUS))?;
                let mut text = ArenaString::new_in(arena);
                native::emit_assembly_with(module, target, code, &mut text)
                    .map_err(|e| message(arena, format_args!("error: {e}"), FAILURE_STATUS))?;
                // Preserve the driver's read-only-file policy before a tool
                // that may unlink an unwritable output gets to open it.
                if path.is_file() {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(path)
                        .map(drop)
                        .map_err(failure)?;
                }
                native::build(&text, path, target, options.output_kind(), writer)
                    .map_err(failure)?;
            } else if diagnosed.call_diagnostics.is_empty() {
                llvm::compile_module(module, path, options.llvm_options(target, code))
                    .map_err(failure)?;
            } else {
                let built = llvm::compile_module_checked(
                    module,
                    path,
                    options.llvm_options(target, code),
                    diagnosed.call_diagnostics.len(),
                    |counts| {
                        let mut retained = ArenaVec::new_in(arena);
                        for (&diagnostic, &count) in diagnosed.call_diagnostics.iter().zip(counts) {
                            for _ in 0..count {
                                retained.push(diagnostic);
                            }
                        }
                        diagnostics = retained.leak();
                        !diagnostics.iter().any(|d| d.is_error(context))
                    },
                )
                .map_err(failure)?;
                if !built {
                    return Err(Failure::Calls(diagnostics));
                }
            }
            atomic_runtime = options.backend == BackEnd::Llvm
                && options.output_kind() == OutputKind::Object
                && llvm::needs_atomic_runtime(module);
            Output::Built
        },
        | (None, None) => {
            let host = DefaultHost::for_target(arena, target);
            interpreter::check_module(module, &host, interpreter::Limits::default()).map_err(
                |e| {
                    message(
                        arena,
                        format_args!(
                            "error: the interpreter stopped the program: {}",
                            e.display_in(module)
                        ),
                        TRAP_STATUS,
                    )
                },
            )?;
            Output::Interpret
        },
    };
    Ok(Prepared {
        output,
        diagnostics,
        atomic_runtime,
    })
}

pub(super) enum Failure<'ir> {
    MiddleEnd(MiddleEndError<'ir>),
    Calls(&'ir [CallDiagnostic]),
    Message { text: &'ir str, status: i32 },
}

pub(super) enum Output<'ir> {
    Text(&'ir str),
    /// A binary module for stdout or the `--emit` path.
    Binary(&'ir [u8]),
    Built,
    Interpret,
}

pub(super) struct Prepared<'ir> {
    pub(super) output:         Output<'ir>,
    pub(super) diagnostics:    &'ir [CallDiagnostic],
    /// Whether the object written needs the atomic runtime when linked.
    pub(super) atomic_runtime: bool,
}

/// The WebAssembly back end's outputs: `--emit=wasm`, `--emit=wat` (and
/// `--emit=asm`, `-S`), and `-o`, which writes the binary module. `None`
/// for the IR views, which the common path prints.
fn wasm<'ir>(
    diagnosed: &DiagnosedModule<'ir>,
    context: &Context<'_>,
    arena: &'ir Bump,
    target: Target,
    options: &CodeGeneration,
) -> Result<Option<Prepared<'ir>>, Failure<'ir>> {
    let module = &diagnosed.module;
    let refuse = |text: &str| {
        Err(message(
            arena,
            format_args!("error: {text}"),
            FAILURE_STATUS,
        ))
    };
    match (options.emit, &options.output_path) {
        | (Some(Emit::Ir | Emit::IrAbi), _) => return Ok(None),
        | (Some(Emit::Llvm), _) =>
            return refuse("--emit=llvm needs the LLVM back end, which has no wasm32 target here"),
        | (None, None) =>
            return refuse(
                "--interpret runs modules of the 64-bit targets; compile wasm32 code with -o or                  --emit=wasm and run it in a WebAssembly engine",
            ),
        | _ => {},
    }
    if !target.is_wasm() {
        return refuse(
            "the WebAssembly back end needs --target=wasm32-wasip1 or wasm32-unknown-unknown",
        );
    }
    let diagnostics = diagnosed.retained_diagnostics;
    if diagnostics.iter().any(|d| d.is_error(context)) {
        return Err(Failure::Calls(diagnostics));
    }
    let scratch = Bump::new();
    let errors = crate::ir::verify_module(module, module.profile(), &scratch);
    if !errors.is_empty() {
        let mut text = ArenaString::new_in(arena);
        for error in &errors {
            _ = fmt::write(
                &mut text,
                format_args!("error: internal error: invalid bcc IR: {error}\n"),
            );
        }
        return Err(Failure::Message {
            text:   text.into_str(),
            status: FAILURE_STATUS,
        });
    }
    let runtime = if target == Target::Wasm32Wasi {
        wasm::runtime::compile(target, arena).map_err(|text| {
            message(
                arena,
                format_args!("error: internal error: {text}"),
                FAILURE_STATUS,
            )
        })?
    } else {
        ArenaVec::new_in(arena)
    };
    let mut units = ArenaVec::with_capacity_in(1 + runtime.len(), arena);
    units.push(module);
    units.extend(runtime.iter());
    let compiled = wasm::compile(&units, 1, target, arena)
        .map_err(|e| message(arena, format_args!("error: {e}"), FAILURE_STATUS))?;
    let output = match (options.emit, &options.output_path) {
        | (Some(Emit::Wasm), _) => Output::Binary(arena.alloc_slice_copy(&compiled.bytes)),
        | (Some(_), _) => {
            let mut text = ArenaString::new_in(arena);
            _ = compiled.write_text(&mut text);
            Output::Text(text.into_str())
        },
        | (None, Some(path)) => {
            std::fs::write(path, &compiled.bytes).map_err(|e| {
                message(
                    arena,
                    format_args!("error: cannot write `{}`: {e}", path.display()),
                    FAILURE_STATUS,
                )
            })?;
            Output::Built
        },
        | (None, None) => unreachable!("refused above"),
    };
    Ok(Some(Prepared {
        output,
        diagnostics,
        atomic_runtime: false,
    }))
}

fn message<'ir>(arena: &'ir Bump, args: fmt::Arguments<'_>, status: i32) -> Failure<'ir> {
    let mut text = ArenaString::new_in(arena);
    _ = fmt::write(&mut text, args);
    Failure::Message {
        text: text.into_str(),
        status,
    }
}
