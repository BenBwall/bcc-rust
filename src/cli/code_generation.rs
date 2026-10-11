//! Code generation: lowers the analyzed translation unit to the bcc IR,
//! optimizes it when asked, and hands it to a back end. `--emit` prints the
//! bcc IR or LLVM IR, `--interpret` runs `main` in the IR interpreter, and
//! `-o` builds an executable or object through the bundled clang.
//!
//! There is no ABI lowering yet, so the back ends receive the pre-ABI
//! module; lowering refuses the by-value aggregate calls that would need it.

#[expect(
    clippy::disallowed_types,
    reason = "clap parses the output path into a `PathBuf`; the driver borrows it as `&Path`."
)]
use std::path::PathBuf;
use std::{
    cell::RefCell,
    fmt,
};

use clap::{
    Args,
    ValueEnum,
};

use super::{
    Bump,
    Context,
    DiagnosticReporter,
    HeaderSearch,
    Path,
    RenderColor,
    Write,
    io,
    output::expect_stderr,
    parse_translation_unit,
};
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
    },
    ir::Module,
    optimizer::{
        OptimizerOptions,
        PassList,
        parse_pass_list,
    },
    pipeline::{
        analyze_translation_unit,
        lower_translation_unit,
    },
    target::Target,
    util::bump::ArenaString,
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
    expect_stderr(reporter.report_pending(context, stderr));
    if !sema.lowerable(context) {
        expect_stderr(reporter.finish(context, stderr));
        return FAILURE_STATUS;
    }
    // The module lives in an arena created after semantic analysis; the
    // syntax tree and its context outlive it.
    let ir = Bump::new();
    let print_after_all = RefCell::new(StderrText);
    let optimizer = options.optimizer(&print_after_all);
    let lowered = lower_translation_unit(context, &unit, &sema, &ir, optimizer.as_ref());
    let module = match lowered {
        | Ok(module) => module,
        | Err(errors) => {
            for error in errors.errors {
                reporter.report_late(error, error.source, context);
            }
            expect_stderr(reporter.finish(context, stderr));
            return FAILURE_STATUS;
        },
    };
    expect_stderr(reporter.finish(context, stderr));
    let target = context.configuration.target();
    match (options.emit, &options.output_path) {
        | (Some(Emit::Ir), _) => print_stdout(format_args!("{module}")),
        | (Some(Emit::Llvm), _) => print_llvm(&module, target),
        | (None, Some(path)) => build(&module, target, path, options),
        | (None, None) => interpret(&module, target),
    }
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
    /// Print the bcc IR (after the optimizer, if one runs) or the LLVM IR to
    /// stdout.
    #[arg(long, value_enum, value_name = "IR", group = "back_end",
        conflicts_with_all = ["tokens", "semantic_types", "syntax_tree", "raw_syntax"])]
    pub(super) emit:              Option<Emit>,
    /// Run `main` in the IR interpreter, print what it writes to stdout, and
    /// exit with its status (70 if the interpreter traps, 134 on `abort`).
    #[arg(long, group = "back_end",
        conflicts_with_all = ["tokens", "semantic_types", "syntax_tree", "raw_syntax"])]
    pub(super) interpret:         bool,
    /// Build an executable at PATH through the bundled clang (an object with
    /// -c).
    #[arg(short = 'o', long = "output", value_name = "PATH", group = "back_end",
        conflicts_with_all = ["tokens", "semantic_types", "syntax_tree", "raw_syntax"])]
    pub(super) output_path:       Option<PathBuf>,
    /// With -o, compile to an object file instead of linking an executable.
    #[arg(short = 'c', requires = "output_path")]
    pub(super) object:            bool,
    /// Select the middle-end optimizer: none, or bcc's pass pipeline.
    #[arg(long, value_enum, default_value = "none", requires = "back_end")]
    pub(super) opt:               MiddleEnd,
    /// Run these bcc passes once each, in order, instead of the default
    /// pipeline (implies --opt=bcc), such as fold,simplify-cfg,dce.
    #[arg(long, value_name = "LIST", value_parser = pass_list, requires = "back_end")]
    pub(super) passes:            Option<PassList>,
    /// Apply only the first N rewrites of the bcc optimizer, to bisect a
    /// miscompile (implies --opt=bcc).
    #[arg(long, value_name = "N", requires = "back_end")]
    pub(super) opt_bisect_limit:  Option<u64>,
    /// Print each function to stderr after every bcc pass (implies
    /// --opt=bcc).
    #[arg(long, requires = "back_end")]
    pub(super) print_after_all:   bool,
    /// Select clang's optimization level for -o; none uses clang's default.
    #[arg(long, value_enum, default_value = "none", requires = "back_end")]
    pub(super) llvm_opt:          LlvmOpt,
    /// Run no LLVM IR pass, only the code generator at the --llvm-opt
    /// level (`-Xclang -disable-llvm-passes`).
    #[arg(long, requires = "back_end")]
    pub(super) llvm_codegen_only: bool,
}

/// What `--emit` prints.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(super) enum Emit {
    /// The bcc IR.
    Ir,
    /// LLVM IR.
    Llvm,
}

/// The middle-end optimizer `--opt` selects.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(super) enum MiddleEnd {
    /// No optimization: the module as lowered.
    None,
    /// bcc's own pass pipeline.
    Bcc,
}

/// Clang's optimization level, from `--llvm-opt`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(super) enum LlvmOpt {
    /// No `-O` flag: clang's default.
    None,
    #[value(name = "O0")]
    O0,
    #[value(name = "O1")]
    O1,
    #[value(name = "O2")]
    O2,
    #[value(name = "O3")]
    O3,
}

impl CodeGeneration {
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
        let enabled = self.opt == MiddleEnd::Bcc
            || self.passes.is_some()
            || self.opt_bisect_limit.is_some()
            || self.print_after_all;
        #[cfg_attr(
            not(test),
            expect(
                clippy::field_reassign_with_default,
                reason = "Test builds give the options a private field, which rules out \
                          functional update syntax."
            )
        )]
        enabled.then(|| {
            let mut options = OptimizerOptions::default();
            options.passes = self.passes;
            options.print_after_all = self.print_after_all.then_some(print_after_all);
            options.bisect_limit = self.opt_bisect_limit;
            options
        })
    }

    /// How clang compiles the LLVM IR for `target`.
    fn llvm_options(&self, target: Target) -> LlvmOptions {
        LlvmOptions {
            opt: match self.llvm_opt {
                | LlvmOpt::None => None,
                | LlvmOpt::O0 => Some(OptLevel::O0),
                | LlvmOpt::O1 => Some(OptLevel::O1),
                | LlvmOpt::O2 => Some(OptLevel::O2),
                | LlvmOpt::O3 => Some(OptLevel::O3),
            },
            disable_llvm_passes: self.llvm_codegen_only,
            target,
            kind: if self.object {
                OutputKind::Object
            } else {
                OutputKind::Executable
            },
        }
    }
}

/// Runs `main` and returns its status, after forwarding what the program
/// wrote to stdout. A trap is reported like a diagnostic.
fn interpret(module: &Module<'_>, target: Target) -> i32 {
    let arena = Bump::new();
    let mut host = DefaultHost::for_target(&arena, target);
    let result = interpreter::run(module, "main", &[], &mut host);
    let mut stdout = io::stdout().lock();
    if let Err(error) = stdout
        .write_all(host.output())
        .and_then(|()| stdout.flush())
    {
        panic!("failed printing to stdout: {error}");
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
            | Termination::Returned(_) => 0,
            | Termination::Exited(status) => status,
            | Termination::Aborted => ABORT_STATUS,
        },
        | Err(trap) => {
            eprintln!(
                "error: the interpreter stopped the program: {}",
                trap.display_in(module)
            );
            TRAP_STATUS
        },
    }
}

/// Prints the module as LLVM IR for `target` to stdout.
fn print_llvm(module: &Module<'_>, target: Target) -> i32 {
    let arena = Bump::new();
    let mut text = ArenaString::new_in(&arena);
    match llvm::emit_module(module, target, &mut text) {
        | Ok(()) => print_stdout(format_args!("{text}")),
        | Err(error) => {
            eprintln!("error: {error}");
            FAILURE_STATUS
        },
    }
}

/// Prints `text` to stdout and returns success.
fn print_stdout(text: fmt::Arguments<'_>) -> i32 {
    let mut stdout = io::stdout().lock();
    if let Err(error) = stdout.write_fmt(text).and_then(|()| stdout.flush()) {
        panic!("failed printing to stdout: {error}");
    }
    0
}

/// Writes the module's LLVM IR to a temporary file and has the bundled
/// clang build `output` from it.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tooling around the compiler: it names one temporary file per clang run, outside the \
              arena-allocated compile path."
)]
fn build(module: &Module<'_>, target: Target, output: &Path, options: &CodeGeneration) -> i32 {
    let arena = Bump::new();
    let mut text = ArenaString::new_in(&arena);
    if let Err(error) = llvm::emit_module(module, target, &mut text) {
        eprintln!("error: {error}");
        return FAILURE_STATUS;
    }
    let ll: PathBuf = std::env::temp_dir().join(format!("bcc-{}.ll", std::process::id()));
    let result = std::fs::write(&ll, text.as_bytes())
        .and_then(|()| llvm::compile_ll(&ll, output, options.llvm_options(target)));
    drop(std::fs::remove_file(&ll));
    match result {
        | Ok(()) => 0,
        | Err(error) => {
            eprintln!("error: cannot build `{}`: {error}", output.display());
            FAILURE_STATUS
        },
    }
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

/// Writes `--print-after-all` text to stderr.
struct StderrText;

impl fmt::Write for StderrText {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        io::stderr().write_all(s.as_bytes()).map_err(|_| fmt::Error)
    }
}
