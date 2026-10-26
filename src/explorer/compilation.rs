//! The explorer's stages. Each one reproduces the command line's output for
//! its view, `--tokens`, `--syntax-tree` or one `--emit` kind with
//! `--backend=native`, and keeps diagnostics as values instead of printing
//! them.
//!
//! The token listing preprocesses the source in its own context, as
//! `--tokens` does. Everything else shares one context: the syntax tree
//! is printed from the parsed unit, and each `--emit` kind lowers the
//! analyzed unit afresh, retrying without optional local retention as the
//! command line does, so the IR before ABI lowering, after it, and the
//! assembly are the command line's even when one of them needs the retry.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "The explorer hands owned text and vectors across its interface, outside every arena."
)]

use std::{
    cell::Cell,
    fmt::{
        self,
        Write as _,
    },
    path::Path,
};

use super::{
    ExplorerDiagnostic,
    ExplorerFunctionBytes,
    ExplorerInstruction,
    ExplorerOptions,
    ExplorerOutput,
    line_map,
};
use crate::{
    backend::native,
    configuration::{
        CompilerConfiguration,
        DEFAULT_MAX_NESTING_DEPTH,
        ExtensionPolicy,
        LanguageMode,
    },
    diagnostics::{
        ColorChoice,
        Diagnostic,
        DiagnosticReporter,
    },
    headers::HeaderSearch,
    ir::{
        Module,
        Profile,
        verify_module,
    },
    lowering::CallDiagnostic,
    optimizer::OptimizerOptions,
    pipeline::{
        DiagnosedModule,
        InlineCopies,
        MiddleEndError,
        analyze_translation_unit,
        describe_token,
        lower_translation_unit_with_locals,
        parse_translation_unit,
        preprocess_with_diagnostics,
        with_preprocessor,
    },
    target::Target,
    translation_phases::{
        Context,
        ErrorSeverity,
        parsing::{
            InspectionOptions,
            ParsedTranslationUnit,
        },
        semantic_analysis::SemanticTranslationUnit,
    },
    util::bump::{
        ArenaString,
        Bump,
    },
};

/// The name the source is compiled under, as `--input` names it.
pub(super) const INPUT: &str = "<input>";

/// Runs every stage on the current stack.
pub(super) fn compile(source: &str, options: &ExplorerOptions) -> ExplorerOutput {
    compile_views(source, options, Views::All).0
}

/// What a compilation produces.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Views {
    /// Every explorer output.
    All,
    /// Only `--emit=llvm`'s LLVM IR, beside the diagnostics.
    #[cfg(feature = "native")]
    Llvm,
}

/// Runs the stages `views` selects on the current stack, returning the
/// outputs and, for [`Views::Llvm`], the LLVM IR.
pub(super) fn compile_views(
    source: &str,
    options: &ExplorerOptions,
    views: Views,
) -> (ExplorerOutput, Option<String>) {
    let configuration = match configure(options) {
        | Ok(configuration) => configuration,
        | Err(message) => return (failed(format_args!("{message}")), None),
    };
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let mut output = ExplorerOutput {
        tokens: (views == Views::All).then(|| tokens(source, configuration, options)),
        ..ExplorerOutput::default()
    };
    let mut context = new_context(&tu, configuration, options);
    let stage = Cell::new("front-end");
    let mut diagnostics = Vec::new();
    let mut observe = |diagnostic: &Diagnostic<'_>, text: &str, context: &Context<'_>| {
        diagnostics.push(record(diagnostic, text, context, stage.get()));
    };
    let llvm = {
        let reporter_arena = Bump::new();
        let reporter = DiagnosticReporter::new(&reporter_arena, &tu, ColorChoice::Plain)
            .observed(&mut observe);
        let mut stages = Stages {
            context: &mut context,
            reporter,
            stage: &stage,
            output: &mut output,
            views,
            llvm: None,
        };
        stages.run(source, options);
        drop(stages.reporter.finish(stages.context, &mut std::io::sink()));
        stages.llvm
    };
    let late = std::mem::take(&mut output.diagnostics);
    diagnostics.extend(late);
    output.diagnostics = diagnostics;
    (output, llvm)
}

/// An output with only an error that kept every stage from running.
pub(super) fn failed(message: fmt::Arguments<'_>) -> ExplorerOutput {
    ExplorerOutput {
        diagnostics: vec![message_diagnostic("error", "front-end", message)],
        ..ExplorerOutput::default()
    }
}

/// The command line's configuration for `--target`, `--std`, `-O2` or
/// `-O0`, `--max-nesting-depth` and `--source-date-epoch`.
fn configure(options: &ExplorerOptions) -> Result<CompilerConfiguration, String> {
    let mode = LanguageMode::parse(&options.std)
        .ok_or_else(|| format!("unknown language standard `{}`", options.std))?;
    let target = Target::parse(&options.target)
        .ok_or_else(|| format!("unsupported target `{}`", options.target))?;
    let default_depth = if cfg!(feature = "native") {
        DEFAULT_MAX_NESTING_DEPTH
    } else {
        super::WASM_MAX_NESTING_DEPTH
    };
    let depth = options.max_nesting_depth.unwrap_or(default_depth).max(1);
    Ok(
        CompilerConfiguration::new(mode.standard, ExtensionPolicy::Allow)
            .with_gnu_extensions(mode.gnu)
            .with_target(target)
            .with_optimization((options.optimize, false))
            .with_source_date_epoch(Some(options.source_date_epoch.unwrap_or(0)))
            .with_max_nesting_depth(depth)
            .with_repeated_specifier_warnings(true),
    )
}

/// A context holding the explorer's in-memory headers.
fn new_context<'tu>(
    tu: &'tu Bump,
    configuration: CompilerConfiguration,
    options: &ExplorerOptions,
) -> Context<'tu> {
    let mut context = Context::with_configuration(tu, configuration);
    context.virtual_files = tu.alloc_slice_fill_iter(options.files.iter().map(|file| {
        (
            Path::new(tu.alloc_str(&file.name)),
            &*tu.alloc_str(&file.contents),
        )
    }));
    context
}

/// Where headers are searched: beside the source, where the in-memory
/// headers are, and then the embedded resource headers.
fn header_search<'a>(tu: &'a Bump, options: &ExplorerOptions) -> HeaderSearch<'a> {
    let beside: &[&Path] = if options.files.is_empty() {
        &[]
    } else {
        tu.alloc_slice_fill_iter([Path::new("")])
    };
    HeaderSearch {
        angled: beside,
        ..HeaderSearch::default()
    }
}

/// The `--tokens` listing, without the diagnostics it interleaves.
fn tokens(source: &str, configuration: CompilerConfiguration, options: &ExplorerOptions) -> String {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let mut context = new_context(&tu, configuration, options);
    let items = with_preprocessor(
        &mut context,
        Path::new(INPUT),
        source,
        header_search(&tu, options),
        |preprocessor, context, _pp| preprocess_with_diagnostics(preprocessor, context),
    );
    let mut text = String::new();
    let mut scratch = Bump::new();
    for token in items.into_iter().flatten() {
        scratch.reset();
        _ = writeln!(text, "{}", describe_token(token, &context, &scratch));
    }
    text
}

/// The stages after preprocessing, sharing one context and reporter.
pub(super) struct Stages<'a, 'r, 'tu> {
    pub(super) context: &'a mut Context<'tu>,
    reporter:           DiagnosticReporter<'r, 'tu>,
    stage:              &'a Cell<&'static str>,
    output:             &'a mut ExplorerOutput,
    views:              Views,
    llvm:               Option<String>,
}

/// Why a middle-end or back-end stage produced nothing.
pub(super) enum Failure<'ir> {
    MiddleEnd(MiddleEndError<'ir>),
    Calls(&'ir [CallDiagnostic]),
    Message(String),
}

impl<'tu> Stages<'_, '_, 'tu> {
    fn run(&mut self, source: &'tu str, options: &ExplorerOptions) {
        let unit = parse_translation_unit(
            self.context,
            Path::new(INPUT),
            source,
            header_search(self.context.tu_arena(), options),
        );
        let inspection = Bump::new();
        self.output.syntax_tree = (self.views == Views::All).then(|| {
            unit.inspect(
                &inspection,
                self.context,
                InspectionOptions {
                    show_locations: false,
                },
            )
            .to_string()
        });
        let sema = analyze_translation_unit(self.context, &unit);
        self.flush();
        if !sema.lowerable(self.context) {
            return;
        }
        let optimizer = options.optimize.then(OptimizerOptions::default);
        let optimizer = optimizer.as_ref();
        let arena = Bump::new();
        #[cfg(feature = "native")]
        if self.views == Views::Llvm {
            self.llvm = self.llvm_ir(&unit, &sema, &arena, optimizer, options.optimize);
            return;
        }
        let Some(ir) = self.print_module(&unit, &sema, &arena, optimizer, Profile::PreAbi) else {
            return;
        };
        let Some(ir_abi) = self.print_module(&unit, &sema, &arena, optimizer, Profile::PostAbi)
        else {
            self.output.ir = Some(ir);
            return;
        };
        let asm = self.assembly(&unit, &sema, &arena, optimizer);
        if let Some(asm) = &asm {
            self.output.machine_code = self.machine_code(asm);
        }
        self.output.line_map = line_map::map(
            &unit,
            self.context,
            INPUT,
            [Some(ir.as_str()), Some(ir_abi.as_str()), asm.as_deref()],
        );
        self.output.ir = Some(ir);
        self.output.ir_abi = Some(ir_abi);
        self.output.asm = asm;
    }

    /// `--emit=ir` or `--emit=ir-abi` with `--backend=native`.
    fn print_module(
        &mut self,
        unit: &ParsedTranslationUnit<'tu>,
        sema: &SemanticTranslationUnit<'tu>,
        arena: &Bump,
        optimizer: Option<&OptimizerOptions<'_>>,
        profile: Profile,
    ) -> Option<String> {
        self.stage.set("middle-end");
        let context = &*self.context;
        let printed = crate::pipeline::with_optional_retention(true, |retain_locals| {
            let diagnosed = lower(
                context,
                unit,
                sema,
                arena,
                optimizer,
                profile,
                retain_locals,
            )?;
            verify(&diagnosed.module)?;
            Ok(diagnosed.module.to_string())
        });
        self.accept(printed)
    }

    /// `--emit=asm --backend=native`.
    fn assembly(
        &mut self,
        unit: &ParsedTranslationUnit<'tu>,
        sema: &SemanticTranslationUnit<'tu>,
        arena: &Bump,
        optimizer: Option<&OptimizerOptions<'_>>,
    ) -> Option<String> {
        let context = &*self.context;
        let target = context.configuration.target();
        let code = context.configuration.code_options();
        let printed = crate::pipeline::with_optional_retention(true, |retain_locals| {
            let diagnosed = lower(
                context,
                unit,
                sema,
                arena,
                optimizer,
                Profile::PostAbi,
                retain_locals,
            )?;
            let calls = diagnosed.retained_diagnostics;
            if calls.iter().any(|call| call.is_error(context)) {
                return Err(Failure::Calls(calls));
            }
            verify(&diagnosed.module)?;
            let mut text = ArenaString::new_in(arena);
            native::emit_assembly_with(&diagnosed.module, target, code, &mut text)
                .map_err(|error| Failure::Message(error.to_string()))?;
            Ok((text.to_string(), calls))
        });
        self.stage.set("back-end");
        let (text, calls) = self.accept(printed)?;
        self.stage.set("middle-end");
        for call in calls {
            call.report(self.context);
            self.flush();
        }
        Some(text)
    }

    /// The machine code of `asm`, which only an ELF target's encoder lays
    /// out.
    fn machine_code(&mut self, asm: &str) -> Option<Vec<ExplorerFunctionBytes>> {
        if !matches!(
            self.context.configuration.target(),
            Target::LinuxGnu | Target::LinuxMusl
        ) {
            self.note("machine code is shown for ELF (Linux) targets only");
            return None;
        }
        let arena = Bump::new();
        match native::machine_code(asm, &arena) {
            | Ok(functions) => Some(
                functions
                    .iter()
                    .map(|function| ExplorerFunctionBytes {
                        name:         function.name.to_owned(),
                        bytes:        function.bytes.to_vec(),
                        instructions: function
                            .instructions
                            .iter()
                            .map(|instruction| ExplorerInstruction {
                                offset: u32::try_from(instruction.offset).unwrap_or(u32::MAX),
                                bytes:  instruction.bytes.to_vec(),
                                asm:    instruction.text.to_owned(),
                            })
                            .collect(),
                    })
                    .collect(),
            ),
            | Err(reason) => {
                self.note(format_args!(
                    "machine code is unavailable: the object encoder refused the assembly \
                     ({reason})"
                ));
                None
            },
        }
    }

    /// Reports a failed stage's diagnostics, as the command line does, or
    /// returns its output.
    pub(super) fn accept<T>(&mut self, result: Result<T, Failure<'_>>) -> Option<T> {
        let failure = match result {
            | Ok(value) => return Some(value),
            | Err(failure) => failure,
        };
        match failure {
            | Failure::MiddleEnd(MiddleEndError::Lowering(errors)) => {
                self.stage.set("middle-end");
                for error in errors.errors {
                    self.reporter.report_late(error, error.source, self.context);
                }
            },
            | Failure::MiddleEnd(MiddleEndError::Abi { error, source }) => {
                self.stage.set("middle-end");
                self.reporter.report_late(&error, source, self.context);
            },
            | Failure::Calls(calls) => {
                self.stage.set("middle-end");
                for call in calls {
                    call.report(self.context);
                    self.flush();
                }
            },
            | Failure::Message(message) => {
                self.flush();
                self.output.diagnostics.push(message_diagnostic(
                    "error",
                    self.stage.get(),
                    format_args!("{message}"),
                ));
            },
        }
        self.flush();
        None
    }

    /// Reports every pending diagnostic.
    fn flush(&mut self) {
        drop(
            self.reporter
                .report_pending(self.context, &mut std::io::sink()),
        );
    }

    fn note(&mut self, message: impl fmt::Display) {
        self.output.diagnostics.push(message_diagnostic(
            "note",
            "back-end",
            format_args!("{message}"),
        ));
    }
}

/// Lowers the unit with the native back end's inline-copy choices.
fn lower<'tu, 'ir>(
    context: &Context<'tu>,
    unit: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
    optimizer: Option<&OptimizerOptions<'_>>,
    profile: Profile,
    retain_locals: bool,
) -> Result<DiagnosedModule<'ir>, Failure<'ir>> {
    lower_translation_unit_with_locals(
        context,
        unit,
        sema,
        arena,
        optimizer,
        profile,
        (InlineCopies::Resolve, retain_locals),
    )
    .map_err(Failure::MiddleEnd)
}

/// Refuses a module the verifier rejects, as the command line does before
/// printing it.
fn verify(module: &Module<'_>) -> Result<(), Failure<'static>> {
    let scratch = Bump::new();
    let errors = verify_module(module, module.profile(), &scratch);
    if errors.is_empty() {
        return Ok(());
    }
    let mut text = String::new();
    for error in &errors {
        _ = write!(text, "internal error: invalid bcc IR: {error}; ");
    }
    Err(Failure::Message(text.trim_end_matches("; ").to_owned()))
}

/// A diagnostic the reporter rendered, with the place its primary range
/// starts and ends.
fn record(
    diagnostic: &Diagnostic<'_>,
    rendered: &str,
    context: &Context<'_>,
    stage: &str,
) -> ExplorerDiagnostic {
    let segments = diagnostic.primary_segments(context);
    let first = segments.first();
    let last = segments
        .iter()
        .rev()
        .find(|segment| Some(segment.source_file_index) == first.map(|f| f.source_file_index));
    let end = last.and_then(|last| {
        let text = context.source_text(last.source_file_index)?;
        let starts = context.source_line_starts(last.source_file_index)?;
        let end = last.end().min(text.len());
        let line = starts.partition_point(|&start| start <= end).max(1);
        let start = starts[line - 1];
        let column = text
            .get(start..end)
            .map_or(0, |prefix| prefix.chars().count());
        Some((
            u32::try_from(line).unwrap_or(u32::MAX),
            u32::try_from(column + 1).unwrap_or(u32::MAX),
        ))
    });
    ExplorerDiagnostic {
        severity:   match diagnostic.severity {
            | ErrorSeverity::Error => "error",
            | ErrorSeverity::Warning => "warning",
        }
        .to_owned(),
        stage:      stage.to_owned(),
        message:    diagnostic.message.to_owned(),
        file:       first.map(|first| {
            context
                .get_source_file(first.source_file_index)
                .display()
                .to_string()
        }),
        line:       first.map(|first| first.line),
        column:     first.map(|first| first.column),
        end_line:   end.map(|(line, _)| line),
        end_column: end.map(|(_, column)| column),
        rendered:   rendered.to_owned(),
    }
}

/// A diagnostic with no place, rendered as the command line prints one.
fn message_diagnostic(
    severity: &str,
    stage: &str,
    message: fmt::Arguments<'_>,
) -> ExplorerDiagnostic {
    let message = message.to_string();
    ExplorerDiagnostic {
        severity: severity.to_owned(),
        stage: stage.to_owned(),
        rendered: format!("{severity}: {message}\n"),
        message,
        file: None,
        line: None,
        column: None,
        end_line: None,
        end_column: None,
    }
}
