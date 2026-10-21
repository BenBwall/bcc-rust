//! Code generation: lowers the analyzed translation unit to the bcc IR,
//! optimizes it when asked, lowers its calls for the target's ABI, and hands
//! it to a back end. `--emit` prints the bcc IR, LLVM IR or the native back
//! end's assembly, `--interpret` runs `main` in the IR interpreter, and `-o`
//! builds an executable or object through LLVM-C (`--backend=llvm`, the
//! default) or the native back end (`--backend=native`), linking with the
//! bundled clang.
//!
//! Every back end receives the post-ABI module. `--emit=ir` prints the
//! pre-ABI module the optimizer worked on, where a structure passed by value
//! is still one `ptr` parameter, and `--emit=ir-abi` the post-ABI one.

mod retention;

#[expect(
    clippy::disallowed_types,
    reason = "clap parses the output path into a `PathBuf`; the driver borrows it as `&Path`."
)]
use std::path::PathBuf;
use std::{
    cell::RefCell,
    fmt,
    io::{
        self,
        Write,
    },
    path::Path,
};

use clap::{
    Args,
    ValueEnum,
};

use super::diagnostic_reporter::DiagnosticReporter;
use crate::{
    backend::{
        interpreter::{
            self,
            DefaultHost,
            RuntimeValue,
            Termination,
        },
        llvm::{
            self,
            LlvmOptions,
            OptLevel,
            OutputKind,
        },
        native,
    },
    diagnostics::ColorChoice as RenderColor,
    headers::HeaderSearch,
    ir::{
        Module,
        Profile,
    },
    optimizer::{
        OptimizerOptions,
        Pass,
        PassList,
        parse_pass_list,
    },
    pipeline::{
        InlineCopies,
        MiddleEndError,
        analyze_translation_unit,
        lower_translation_unit_with_locals,
        parse_translation_unit,
        with_optional_retention,
    },
    target::{
        CodeOptions,
        Target,
    },
    translation_phases::Context,
    util::bump::{
        ArenaString,
        Bump,
    },
};

/// The status of a compilation that reported errors, or whose back end
/// failed.
pub(super) const FAILURE_STATUS: i32 = 1;
/// The status `--interpret` exits with when the interpreter stops the
/// program: undefined behaviour, an unsupported operation, or a limit. It is
/// `EX_SOFTWARE` of BSD's `sysexits.h`.
pub(super) const TRAP_STATUS: i32 = 70;
/// The status `--interpret` exits with when the program calls `abort`: 128
/// plus `SIGABRT`, as a POSIX shell reports a process that `abort` killed.
pub(super) const ABORT_STATUS: i32 = 134;

/// Compiles the input for the back end `options` selects and returns the
/// process's exit status: the program's status for `--interpret`, otherwise
/// zero on success and [`FAILURE_STATUS`] after errors.
pub(super) fn generate_code<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    search: HeaderSearch<'_>,
    options: &CodeGeneration,
) -> i32 {
    let unit = parse_translation_unit(context, source_filename, input_string, search);
    let sema = analyze_translation_unit(context, &unit);
    let reporter_arena = Bump::new();
    let mut reporter = DiagnosticReporter::new(
        &reporter_arena,
        context.tu_arena(),
        RenderColor::for_stderr(),
    );
    let stderr = &mut io::stderr();
    drop(reporter.report_pending(context, stderr));
    if !sema.lowerable(context) {
        drop(reporter.finish(context, stderr));
        return FAILURE_STATUS;
    }
    // The module lives in an arena created after semantic analysis; the
    // syntax tree and its context outlive it.
    let ir = Bump::new();
    let print_after_all = RefCell::new(StderrText);
    let optimizer = options.optimizer(&print_after_all);
    let profile = if options.emit == Some(Emit::Ir) {
        Profile::PreAbi
    } else {
        Profile::PostAbi
    };
    let copies = options.inline_copies();
    let target = context.configuration.target();
    let code = context.configuration.code_options();
    let prepared = with_optional_retention(true, |retain_locals| {
        let diagnosed = lower_translation_unit_with_locals(
            context,
            &unit,
            &sema,
            &ir,
            optimizer.as_ref(),
            profile,
            (copies, retain_locals),
        )
        .map_err(retention::Failure::MiddleEnd)?;
        let prepared = retention::prepare(&diagnosed, &ir, target, code, options)?;
        Ok((diagnosed, prepared))
    });
    let (diagnosed, prepared) = match prepared {
        | Ok(value) => value,
        | Err(retention::Failure::MiddleEnd(MiddleEndError::Lowering(errors))) => {
            for error in errors.errors {
                reporter.report_late(error, error.source, context);
            }
            drop(reporter.finish(context, stderr));
            return FAILURE_STATUS;
        },
        | Err(retention::Failure::MiddleEnd(MiddleEndError::Abi { error, source })) => {
            reporter.report_late(&error, source, context);
            drop(reporter.finish(context, stderr));
            return FAILURE_STATUS;
        },
        | Err(retention::Failure::Calls(diagnostics)) => {
            for diagnostic in diagnostics {
                diagnostic.report(context);
                drop(reporter.report_pending(context, stderr));
            }
            drop(reporter.finish(context, stderr));
            return FAILURE_STATUS;
        },
        | Err(retention::Failure::Message { text, status }) => {
            drop(reporter.finish(context, stderr));
            report_stderr(format_args!("{text}"));
            return status;
        },
    };
    for diagnostic in prepared.diagnostics {
        diagnostic.report(context);
        drop(reporter.report_pending(context, stderr));
    }
    drop(reporter.finish(context, stderr));
    match prepared.output {
        | retention::Output::Text(text) => print_stdout(format_args!("{text}")),
        | retention::Output::Built => 0,
        | retention::Output::Interpret =>
            interpret(&diagnosed.module, target, options.interpret_step_limit),
    }
}

/// Prints the default pipeline in `--passes` syntax and returns the exit
/// status.
pub(super) fn print_default_passes() -> i32 {
    let mut stdout = io::stdout().lock();
    let mut separator = "";
    for pass in Pass::ALL {
        if let Err(error) = write!(stdout, "{separator}{pass}") {
            return stdout_error(&error);
        }
        separator = ",";
    }
    if let Err(error) = writeln!(stdout).and_then(|()| stdout.flush()) {
        return stdout_error(&error);
    }
    0
}

/// The code generation options. A back end runs only when `--emit`,
/// `--interpret` or `-o` asks for one; the other options configure it.
#[derive(Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Clap owns independent command-line switches."
)]
#[expect(
    clippy::disallowed_types,
    reason = "clap's derived parser owns the output path."
)]
pub(super) struct CodeGeneration {
    /// Print the bcc IR (after the optimizer, if one runs; before ABI
    /// lowering for `ir`, after it for `ir-abi`) or the LLVM IR to stdout.
    #[arg(long, value_enum, value_name = "IR", group = "back_end",
        conflicts_with_all = ["tokens", "semantic_types", "syntax_tree", "raw_syntax"])]
    pub(super) emit:                 Option<Emit>,
    /// Run `main` in the IR interpreter, print what it writes to stdout, and
    /// exit with its status (70 if the interpreter traps, 134 on `abort`).
    #[arg(long, group = "back_end",
        conflicts_with_all = ["tokens", "semantic_types", "syntax_tree", "raw_syntax"])]
    pub(super) interpret:            bool,
    /// Maximum interpreter instructions (default 100000000); 0 is unlimited.
    #[arg(long, value_name = "N", requires = "interpret")]
    pub(super) interpret_step_limit: Option<u64>,
    /// Build an executable at PATH through the bundled clang (an object with
    /// -c).
    #[arg(short = 'o', long = "output", value_name = "PATH", group = "back_end",
        conflicts_with_all = ["tokens", "semantic_types", "syntax_tree", "raw_syntax"])]
    pub(super) output_path:          Option<PathBuf>,
    /// With -o, compile to an object file instead of linking an executable.
    #[arg(short = 'c', requires = "output_path")]
    pub(super) object:               bool,
    /// With -o, link a shared library (also accepts GCC -shared); its code
    /// needs -fpic or -fPIC.
    #[arg(long, requires = "output_path", conflicts_with_all = ["object", "pie", "no_pie"])]
    pub(super) shared:               bool,
    /// With -o, link a position-independent executable, the default on
    /// Linux (also accepts GCC -pie).
    #[arg(long, requires = "output_path", overrides_with = "no_pie")]
    pub(super) pie:                  bool,
    /// With -o, link an executable at a fixed address (also accepts GCC
    /// -no-pie), as code compiled with -fno-pic needs.
    #[arg(long, requires = "output_path", overrides_with = "pie")]
    pub(super) no_pie:               bool,
    /// GCC optimization level (also accepts -O0, -O1, -O2, -O3, -Os, -Oz, -Og).
    #[arg(long, value_enum, overrides_with = "optimization")]
    pub(super) optimization:         Option<LlvmOpt>,
    /// Select the middle-end optimizer: none, or bcc's pass pipeline.
    #[arg(long, value_enum, overrides_with = "opt")]
    pub(super) opt:                  Option<MiddleEnd>,
    /// Run these bcc passes once each, in order, instead of the default
    /// pipeline (implies --opt=bcc), such as fold,simplify-cfg,dce.
    #[arg(long, value_name = "LIST", value_parser = pass_list, requires = "back_end")]
    pub(super) passes:               Option<PassList>,
    /// Apply only the first N rewrites of the bcc optimizer, to bisect a
    /// miscompile (implies --opt=bcc).
    #[arg(long, value_name = "N", requires = "back_end")]
    pub(super) opt_bisect_limit:     Option<u64>,
    /// Print each function to stderr after every bcc pass (implies
    /// --opt=bcc).
    #[arg(long, requires = "back_end")]
    pub(super) print_after_all:      bool,
    /// Print the bcc optimizer's per-pass runs, rewrites and times to
    /// stderr when it finishes (implies --opt=bcc).
    #[arg(long, requires = "back_end")]
    pub(super) time_passes:          bool,
    /// Select the LLVM IR pipeline and code generation level; none means O0.
    #[arg(long, value_enum, overrides_with = "llvm_opt")]
    pub(super) llvm_opt:             Option<LlvmOpt>,
    /// Run no LLVM IR pass, only the code generator at the --llvm-opt
    /// level.
    #[arg(long, requires = "back_end")]
    pub(super) llvm_codegen_only:    bool,
    /// Select the code generator for -o and --emit=asm: LLVM through its C
    /// API, or bcc's own x86-64 back end, which refuses what it does not
    /// support yet.
    #[arg(long, value_enum, default_value = "llvm", requires = "back_end")]
    pub(super) backend:              BackEnd,
    /// Encode the native back end's ELF or COFF objects in process, the
    /// default for Windows targets, where a listing the encoder refuses is
    /// assembled unless this flag is given; a module with file-scope assembly
    /// is refused.
    #[arg(long, requires = "back_end", overrides_with = "no_native_object")]
    pub(super) native_object:        bool,
    /// Assemble the native back end's listing with the bundled clang, the
    /// default for Linux targets and for a module with file-scope assembly.
    #[arg(long, requires = "back_end", overrides_with = "native_object")]
    pub(super) no_native_object:     bool,
}

/// What `--emit` prints.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(super) enum Emit {
    /// The bcc IR before ABI lowering, as the optimizer leaves it.
    Ir,
    /// The bcc IR after ABI lowering, as the back ends receive it.
    IrAbi,
    /// LLVM IR.
    Llvm,
    /// The native back end's assembly (with --backend=native).
    Asm,
}

/// The code generator `--backend` selects.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(super) enum BackEnd {
    /// LLVM through its C API.
    Llvm,
    /// bcc's own x86-64 code generator.
    Native,
}

/// The middle-end optimizer `--opt` selects.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(super) enum MiddleEnd {
    /// No optimization: the module as lowered.
    None,
    /// bcc's own pass pipeline.
    Bcc,
}

/// LLVM's optimization level, from `--llvm-opt`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(super) enum LlvmOpt {
    /// The default O0 pipeline and code generation.
    None,
    #[value(name = "O0")]
    O0,
    #[value(name = "O1")]
    O1,
    #[value(name = "O2")]
    O2,
    #[value(name = "O3")]
    O3,
    #[value(name = "Os")]
    Os,
    #[value(name = "Oz")]
    Oz,
    #[value(name = "Og")]
    Og,
}

impl LlvmOpt {
    fn enabled(self) -> bool {
        !matches!(self, Self::None | Self::O0)
    }
}

impl CodeGeneration {
    /// GNU implementation macros describe the effective optimizer selection.
    pub(super) fn optimization_macros(&self) -> (bool, bool) {
        let llvm = self.llvm_opt.or(self.optimization).unwrap_or(LlvmOpt::None);
        (
            self.bcc_enabled() || llvm.enabled(),
            matches!(llvm, LlvmOpt::Os | LlvmOpt::Oz),
        )
    }

    /// Why the selected options do not fit the selected back end, if they
    /// do not.
    pub(super) fn back_end_conflict(&self) -> Option<&'static str> {
        let native = self.backend == BackEnd::Native;
        match self.emit {
            | Some(Emit::Asm) if !native =>
                return Some(
                    "--emit=asm prints the native back end's assembly; add --backend=native",
                ),
            | Some(Emit::Llvm) if native =>
                return Some("--emit=llvm prints LLVM IR, which --backend=native does not use"),
            | _ => {},
        }
        if !native {
            if self.native_object {
                return Some("--native-object requires --backend=native");
            }
            if self.no_native_object {
                return Some("--no-native-object requires --backend=native");
            }
            return None;
        }
        if self.emit.is_none() && self.output_path.is_none() {
            return Some("--backend=native needs -o or --emit=asm; --interpret runs the IR");
        }
        if self.llvm_opt.is_some_and(|level| level != LlvmOpt::None) || self.llvm_codegen_only {
            return Some(
                "--llvm-opt and --llvm-codegen-only configure the LLVM back end, not \
                 --backend=native",
            );
        }
        None
    }

    /// Whether a back end was requested.
    pub(super) fn requested(&self) -> bool {
        self.emit.is_some() || self.interpret || self.output_path.is_some()
    }

    /// The optimizer options, or `None` when no bcc pass runs. Any of the
    /// optimizer's own options turns it on.
    fn optimizer<'a>(
        &self,
        print_after_all: &'a RefCell<dyn fmt::Write + 'a>,
    ) -> Option<OptimizerOptions<'a>> {
        #[cfg_attr(
            not(test),
            expect(
                clippy::field_reassign_with_default,
                reason = "Test builds give the options a private field, which rules out \
                          functional update syntax."
            )
        )]
        self.bcc_enabled().then(|| {
            let mut options = OptimizerOptions::default();
            options.passes = self.passes;
            options.print_after_all = self.print_after_all.then_some(print_after_all);
            options.time_passes = self.time_passes.then_some(print_after_all);
            options.bisect_limit = self.opt_bisect_limit;
            options
        })
    }

    fn bcc_enabled(&self) -> bool {
        self.opt
            .unwrap_or(if self.optimization.is_some_and(LlvmOpt::enabled) {
                MiddleEnd::Bcc
            } else {
                MiddleEnd::None
            })
            == MiddleEnd::Bcc
            || self.passes.is_some()
            || self.opt_bisect_limit.is_some()
            || self.print_after_all
            || self.time_passes
    }

    /// The native object writer `--native-object` or `--no-native-object`
    /// requests, whichever came last, or `None` for the target's default.
    fn requested_object_writer(&self) -> Option<native::RequestedWriter> {
        if self.native_object {
            Some(native::RequestedWriter::Direct)
        } else if self.no_native_object {
            Some(native::RequestedWriter::Assembler)
        } else {
            None
        }
    }

    /// What `-o` builds.
    fn output_kind(&self) -> OutputKind {
        if self.object {
            OutputKind::Object
        } else if self.shared {
            OutputKind::SharedLibrary
        } else if self.no_pie {
            OutputKind::PositionDependentExecutable
        } else {
            OutputKind::Executable
        }
    }

    /// What the middle end does with inline copies: the interpreter and the
    /// native back end make LLVM's inlining choices themselves, and LLVM
    /// gets the copies as `available_externally` bodies, the plain ones only
    /// when its IR pipeline optimizes, as Clang emits them.
    fn inline_copies(&self) -> InlineCopies {
        if self.backend == BackEnd::Native || (self.emit.is_none() && self.output_path.is_none()) {
            InlineCopies::Resolve
        } else if !self.llvm_codegen_only
            && matches!(
                self.llvm_opt.or(self.optimization),
                Some(LlvmOpt::O1 | LlvmOpt::O2 | LlvmOpt::O3)
            )
        {
            InlineCopies::Keep
        } else {
            InlineCopies::KeepAlwaysInline
        }
    }

    /// LLVM IR passes and code generation for `target`.
    fn llvm_options(&self, target: Target, code: CodeOptions) -> LlvmOptions {
        LlvmOptions {
            opt: match self.llvm_opt.or(self.optimization).unwrap_or(LlvmOpt::None) {
                | LlvmOpt::None => None,
                | LlvmOpt::O0 => Some(OptLevel::O0),
                | LlvmOpt::O1 | LlvmOpt::Og => Some(OptLevel::O1),
                | LlvmOpt::O2 => Some(OptLevel::O2),
                | LlvmOpt::O3 => Some(OptLevel::O3),
                | LlvmOpt::Os => Some(OptLevel::Os),
                | LlvmOpt::Oz => Some(OptLevel::Oz),
            },
            disable_llvm_passes: self.llvm_codegen_only,
            target,
            kind: self.output_kind(),
            code,
        }
    }
}

/// Runs `main` and returns its status, after forwarding what the program
/// wrote to stdout. A trap is reported like a diagnostic.
fn interpret(module: &Module<'_>, target: Target, steps: Option<u64>) -> i32 {
    let arena = Bump::new();
    let mut host = DefaultHost::for_target(&arena, target);
    let mut limits = interpreter::Limits::default();
    if let Some(steps) = steps {
        limits.steps = steps;
    }
    let result = interpreter::run_main_with_limits(module, &mut host, limits);
    let mut stdout = io::stdout().lock();
    if let Err(error) = stdout
        .write_all(host.output())
        .and_then(|()| stdout.flush())
        && error.kind() != io::ErrorKind::BrokenPipe
    {
        return stdout_error(&error);
    }
    match result {
        // C99: §5.1.2.2.3 paragraph 1, p. 13; PDF p. 25: returning from
        // the initial call to `main` is calling `exit` with its value.
        | Ok(outcome) => match outcome.termination {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "`main` returns an `int`, held in the low 32 bits."
            )]
            | Termination::Returned(Some(RuntimeValue::Int(bits))) => bits as u32 as i32,
            | Termination::Returned(Some(RuntimeValue::Poison)) => {
                report_stderr(format_args!(
                    "error: the interpreter stopped the program: undefined behaviour: {} (return \
                     from main)",
                    interpreter::UbKind::PoisonInHostCall
                ));
                TRAP_STATUS
            },
            | Termination::Returned(_) => 0,
            | Termination::Exited(status) => status,
            | Termination::Aborted => ABORT_STATUS,
        },
        | Err(trap) => {
            report_stderr(format_args!(
                "error: the interpreter stopped the program: {}",
                trap.display_in(module)
            ));
            TRAP_STATUS
        },
    }
}

/// Prints `text` to stdout and returns success.
fn print_stdout(text: fmt::Arguments<'_>) -> i32 {
    let mut stdout = io::stdout().lock();
    if let Err(error) = stdout.write_fmt(text).and_then(|()| stdout.flush()) {
        return stdout_error(&error);
    }
    0
}

/// Closed consumers end forwarding quietly; other output errors are CLI
/// failures rather than compiler panics.
fn stdout_error(error: &io::Error) -> i32 {
    if error.kind() == io::ErrorKind::BrokenPipe {
        return 0;
    }
    report_stderr(format_args!("error: failed writing to stdout: {error}"));
    FAILURE_STATUS
}

/// Parses `--passes`.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Clap argument errors own their startup message."
)]
fn pass_list(value: &str) -> Result<PassList, String> {
    parse_pass_list(value).map_err(|error| error.to_string())
}

/// Writes diagnostics best-effort, preserving the status of the compilation
/// or interpreted program even when the diagnostic handle is unwritable.
///
/// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23 (required diagnostics);
/// handling a failed host write is a CLI policy.
pub(super) fn report_stderr(text: fmt::Arguments<'_>) {
    drop(writeln!(io::stderr().lock(), "{text}"));
}

/// Writes `--print-after-all` text to stderr.
struct StderrText;

impl fmt::Write for StderrText {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        io::stderr().write_all(s.as_bytes()).map_err(|_| fmt::Error)
    }
}
