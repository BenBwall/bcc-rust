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
    let output = match (options.emit, &options.output_path) {
        | (Some(Emit::Ir | Emit::IrAbi), _) => {
            let mut text = ArenaString::new_in(arena);
            _ = fmt::write(&mut text, format_args!("{module}"));
            Output::Text(text.into_str())
        },
        | (Some(Emit::Llvm), _) => {
            let mut text = ArenaString::new_in(arena);
            llvm::emit_module_with(module, target, code, &mut text)
                .map_err(|e| message(arena, format_args!("error: {e}"), FAILURE_STATUS))?;
            Output::Text(text.into_str())
        },
        | (Some(Emit::Asm), _) => {
            let mut text = ArenaString::new_in(arena);
            native::emit_assembly_with(module, target, code, &mut text)
                .map_err(|e| message(arena, format_args!("error: {e}"), FAILURE_STATUS))?;
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
    })
}

pub(super) enum Failure<'ir> {
    MiddleEnd(MiddleEndError<'ir>),
    Calls(&'ir [CallDiagnostic]),
    Message { text: &'ir str, status: i32 },
}

pub(super) enum Output<'ir> {
    Text(&'ir str),
    Built,
    Interpret,
}

pub(super) struct Prepared<'ir> {
    pub(super) output:      Output<'ir>,
    pub(super) diagnostics: &'ir [CallDiagnostic],
}

fn message<'ir>(arena: &'ir Bump, args: fmt::Arguments<'_>, status: i32) -> Failure<'ir> {
    let mut text = ArenaString::new_in(arena);
    _ = fmt::write(&mut text, args);
    Failure::Message {
        text: text.into_str(),
        status,
    }
}
