//! Hands an analyzed translation unit to the middle end: lowering into an
//! `'ir` arena that the caller creates after semantic analysis, then the
//! optimizer when one is requested, then ABI lowering when the caller wants
//! the module a back end receives.
//!
//! The optimizer works on the pre-ABI module, where a structure passed by
//! value is still one `ptr` parameter and `va_arg` is one operation. ABI
//! lowering then fixes how the target passes each of them, so the LLVM back
//! end and the interpreter see the same calling-convention decisions;
//! `--emit=ir` prints the pre-ABI module and `--emit=ir-abi` the post-ABI
//! one.

use std::path::Path;

use crate::{
    abi::{
        AbiError,
        lower_abi,
    },
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    headers::HeaderSearch,
    ir::{
        Module,
        Profile,
    },
    lowering::{
        LoweringErrors,
        wide_bit_int_runtime,
    },
    optimizer::{
        OptimizerOptions,
        optimize,
        resolve_inline_copies,
        resolve_object_sizes,
    },
    pipeline::{
        analyze_translation_unit,
        parse_translation_unit,
    },
    translation_phases::{
        Context,
        SourceVectors,
        parsing::ParsedTranslationUnit,
        semantic_analysis::SemanticTranslationUnit,
    },
    util::bump::Bump,
};

/// Lowers the analyzed `syntax` for the configured target into `arena`,
/// optimizes the module with `optimizer`, resolves its inline copies for a
/// back end other than LLVM ([`InlineCopies`]), and, for
/// [`Profile::PostAbi`], lowers its calls for the target's calling
/// convention.
///
/// A unit that any phase reported an error in is refused (`sema.lowerable`)
/// with a single [`UnitHasErrors`](crate::lowering::LoweringErrorKind) error;
/// callers that have already reported those errors check
/// `sema.lowerable(context)` first. Constructs that lowering does not handle
/// yet come back as errors with their provenance, which render through the
/// shared diagnostic interface; a case ABI lowering does not handle names
/// its function.
/// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
#[cfg(test)]
pub(crate) fn lower_translation_unit_diagnosed<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
    optimizer: Option<&OptimizerOptions<'_>>,
    profile: Profile,
    copies: InlineCopies,
) -> Result<DiagnosedModule<'ir>, MiddleEndError<'ir>> {
    with_optional_retention(true, |retain_locals| {
        lower_translation_unit_with_locals(
            context,
            syntax,
            sema,
            arena,
            optimizer,
            profile,
            (copies, retain_locals),
        )
    })
}

/// Retry the diagnosed pipeline without optional O0 local retention when a
/// backend cannot emit the retained initializer tree. Other emission roots,
/// semantic diagnostics and optimization policy remain unchanged.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23.
#[cfg(any(test, not(feature = "recursive-lowering-study")))]
pub(crate) fn lower_translation_unit_with_locals<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
    optimizer: Option<&OptimizerOptions<'_>>,
    profile: Profile,
    emission: (InlineCopies, bool),
) -> Result<DiagnosedModule<'ir>, MiddleEndError<'ir>> {
    let (copies, retain_locals) = emission;
    lower_diagnosed(
        context,
        syntax,
        sema,
        arena,
        optimizer,
        profile,
        LoweringControl {
            copies,
            retain_locals,
            source_lines: false,
            #[cfg(feature = "recursive-lowering-study")]
            recursive: false,
        },
    )
}

/// Experimental CLI seam, called only inside the reserved compilation worker.
#[cfg(feature = "recursive-lowering-study")]
pub(crate) fn lower_translation_unit_recursive_with_locals<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
    optimizer: Option<&OptimizerOptions<'_>>,
    profile: Profile,
    emission: (InlineCopies, bool),
) -> Result<DiagnosedModule<'ir>, MiddleEndError<'ir>> {
    let (copies, retain_locals) = emission;
    lower_diagnosed(
        context,
        syntax,
        sema,
        arena,
        optimizer,
        profile,
        LoweringControl {
            copies,
            retain_locals,
            source_lines: false,
            recursive: true,
        },
    )
}

/// [`lower_translation_unit_with_locals`] with each instruction's source
/// line recorded on every target, for the explorer's line-by-line links
/// between the source and its IR.
pub(crate) fn lower_translation_unit_with_lines<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
    optimizer: Option<&OptimizerOptions<'_>>,
    profile: Profile,
    emission: (InlineCopies, bool),
) -> Result<DiagnosedModule<'ir>, MiddleEndError<'ir>> {
    let (copies, retain_locals) = emission;
    lower_diagnosed(
        context,
        syntax,
        sema,
        arena,
        optimizer,
        profile,
        LoweringControl {
            copies,
            retain_locals,
            source_lines: true,
            #[cfg(feature = "recursive-lowering-study")]
            recursive: false,
        },
    )
}

#[derive(Clone, Copy)]
struct LoweringControl {
    copies:        InlineCopies,
    retain_locals: bool,
    /// Record each instruction's source line (`LoweringOptions`).
    source_lines:  bool,
    #[cfg(feature = "recursive-lowering-study")]
    recursive:     bool,
}

fn lower_diagnosed<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
    optimizer: Option<&OptimizerOptions<'_>>,
    profile: Profile,
    control: LoweringControl,
) -> Result<DiagnosedModule<'ir>, MiddleEndError<'ir>> {
    let copies = control.copies;
    let mut scratch = Bump::new();
    let target = context.configuration.target();
    let mut options =
        crate::lowering::LoweringOptions::default_for_build(copies == InlineCopies::Keep);
    options.retain_locals = control.retain_locals;
    options.source_lines = control.source_lines;
    #[cfg(feature = "recursive-lowering-study")]
    {
        options.recursive = control.recursive;
    }
    let crate::lowering::LoweredUnit {
        mut module,
        function_sources,
        call_diagnostics,
    } = crate::lowering::lower_with_options(
        context,
        syntax,
        sema,
        target,
        arena,
        &mut scratch,
        &mut options,
    )
    .map_err(MiddleEndError::Lowering)?;
    if wide_bit_int_runtime::needed(&module) {
        let mut runtime = compile_wide_bit_int_runtime(context, arena)?;
        scratch.reset();
        wide_bit_int_runtime::link(&mut module, &mut runtime, &scratch)
            .map_err(|fact| runtime_error(arena, fact))?;
    }
    if let Some(options) = optimizer {
        scratch.reset();
        _ = optimize(&mut module, options, &scratch);
    }
    if copies == InlineCopies::Resolve {
        scratch.reset();
        resolve_inline_copies(&mut module, &scratch);
    }
    scratch.reset();
    resolve_object_sizes(&mut module, context.configuration.hosted(), &scratch);
    scratch.reset();
    let retained_diagnostics =
        crate::lowering::retained_call_diagnostics(&module, call_diagnostics, arena, &scratch);
    if profile == Profile::PostAbi {
        scratch.reset();
        lower_abi(&mut module, target, &scratch).map_err(|error| MiddleEndError::Abi {
            source: function_sources
                .iter()
                .find(|(function, _)| *function == error.function)
                .map(|(_, source)| *source),
            error,
        })?;
    }
    Ok(DiagnosedModule {
        module,
        call_diagnostics,
        retained_diagnostics,
    })
}

/// The runtime of the `_BitInt` types of more than 128 bits
/// (`lowering/wide_bit_int/runtime.c`), compiled for the unit's target into
/// `arena`: optimized C17 with GNU extensions, whatever the unit's language
/// mode, as the bundled WebAssembly runtime is built. It does not compile
/// only through a bug of bcc, reported as a missing fact.
fn compile_wide_bit_int_runtime<'ir>(
    context: &Context<'_>,
    arena: &'ir Bump,
) -> Result<Module<'ir>, MiddleEndError<'ir>> {
    let configuration = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
        .with_gnu_extensions(true)
        .with_target(context.configuration.target())
        .with_optimization((true, false))
        .with_suppress_warnings(true);
    let tu = Bump::new();
    let mut runtime_context = Context::with_configuration(&tu, configuration);
    let source = tu.alloc_str(wide_bit_int_runtime::SOURCE);
    let unit = parse_translation_unit(
        &mut runtime_context,
        Path::new("bitint-runtime.c"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut runtime_context, &unit);
    if !sema.lowerable(&runtime_context) {
        return Err(runtime_error(arena, "the _BitInt runtime does not compile"));
    }
    let optimizer = OptimizerOptions::default();
    lower_diagnosed(
        &runtime_context,
        &unit,
        &sema,
        arena,
        Some(&optimizer),
        Profile::PreAbi,
        LoweringControl {
            copies: InlineCopies::Resolve,
            retain_locals: false,
            source_lines: false,
            #[cfg(feature = "recursive-lowering-study")]
            recursive: false,
        },
    )
    .map(|diagnosed| diagnosed.module)
}

/// A failure of the `_BitInt` runtime, `fact`, as a lowering error.
fn runtime_error<'ir>(arena: &'ir Bump, fact: &'static str) -> MiddleEndError<'ir> {
    MiddleEndError::Lowering(LoweringErrors {
        errors: arena.alloc_slice_copy(&[crate::lowering::LoweringError {
            kind:   crate::lowering::LoweringErrorKind::MissingFact(fact),
            source: None,
        }]),
    })
}

/// Optional retention is speculative across the complete caller-owned
/// capability pipeline. A refusal retries once with only required roots.
/// The caller must finish preparation before committing output or running
/// user code. Required and `used` roots remain in the second attempt.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23; best-effort retention follows Clang.
pub(crate) fn with_optional_retention<T, E>(
    retain_locals: bool,
    mut attempt: impl FnMut(bool) -> Result<T, E>,
) -> Result<T, E> {
    match attempt(retain_locals) {
        | Err(_) if retain_locals => attempt(false),
        | result => result,
    }
}

/// Internal module-only adapter. Callers that report diagnostics use the
/// diagnosed entry point so warnings are retained alongside successful code.
#[cfg(test)]
pub(crate) fn lower_translation_unit<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
    optimizer: Option<&OptimizerOptions<'_>>,
    profile: Profile,
) -> Result<Module<'ir>, MiddleEndError<'ir>> {
    let DiagnosedModule {
        module,
        retained_diagnostics: diagnostics,
        ..
    } = lower_translation_unit_diagnosed(
        context,
        syntax,
        sema,
        arena,
        optimizer,
        profile,
        InlineCopies::Keep,
    )?;
    let mut errors = crate::util::bump::ArenaVec::new_in(arena);
    errors.extend(diagnostics.iter().filter(|d| !d.warning).map(|d| {
        crate::lowering::LoweringError {
            kind:   crate::lowering::LoweringErrorKind::CallDiagnostic(d.message),
            source: Some(d.source),
        }
    }));
    let errors = errors.leak();
    if errors.is_empty() {
        Ok(module)
    } else {
        Err(MiddleEndError::Lowering(LoweringErrors { errors }))
    }
}

/// What the middle end does with the private copies of inline definitions
/// ([`Function::inline_copy_of`](crate::ir::Function)).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InlineCopies {
    /// Keep them for an optimizing LLVM pipeline, which gets each as its
    /// external function's `available_externally` body, as Clang emits an
    /// inline definition above O0: plain inline-only bodies are lowered too.
    Keep,
    /// Keep only the `always_inline` copies, as Clang emits no other
    /// `available_externally` body at O0, where LLVM never reads one.
    KeepAlwaysInline,
    /// Make LLVM's inlining choices for the interpreter and the native back
    /// end, which then call the external definition wherever a call is not
    /// expanded ([`resolve_inline_copies`]).
    Resolve,
}

/// A module and its source diagnostic table; numeric call IDs index the full
/// table, while the retained slice reflects the bcc optimizer's emitted calls.
pub(crate) struct DiagnosedModule<'ir> {
    pub(crate) module:               Module<'ir>,
    pub(crate) call_diagnostics:     &'ir [crate::lowering::CallDiagnostic],
    pub(crate) retained_diagnostics: &'ir [crate::lowering::CallDiagnostic],
}

/// Why the middle end produced no module.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MiddleEndError<'ir> {
    /// Lowering refused the unit, with one error per construct.
    Lowering(LoweringErrors<'ir>),
    /// ABI lowering met a case it does not handle.
    Abi {
        error:  AbiError<'ir>,
        source: Option<SourceVectors>,
    },
}
