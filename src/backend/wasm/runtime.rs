//! The bundled C runtime of the `wasm32-wasip1` target: a small C library
//! (`runtime/*.c`, behind the headers in `headers/wasi/`) and the
//! compiler-rt routines the back end calls, written in C and compiled by
//! bcc itself whenever a WASI module is linked. Only what the program
//! reaches is kept, so a program that prints nothing carries no `printf`.
//!
//! The library covers program startup and exit (`_start`, `exit`,
//! `atexit`, `abort`), `malloc` and friends, `<string.h>`, `<ctype.h>`,
//! the conversions (`strtod` and its kin correctly rounded, in
//! `floatscan.c`) and utilities of `<stdlib.h>`, buffered `<stdio.h>`
//! streams on file descriptors 0-2 with the full `printf` family (exact
//! floating conversions) and a small `scanf`, and the rounding functions of
//! `<math.h>`. It talks to the host only through WASI preview 1 imports
//! (`fd_write`, `fd_read`, `proc_exit`, `args_get`, `environ_get`,
//! `clock_time_get`), so Node's `wasi` module, wasmtime or a browser
//! shim runs the result. Files cannot be opened.
//!
//! [`modules`] compiles every runtime source once per thread and keeps the
//! modules for the rest of the process in an arena that is never freed, so
//! the explorer, which compiles many programs in one session, and a test
//! process pay for the runtime once. The runtime's build depends only on
//! its target: it is always optimized C17 with GNU extensions, whatever
//! the program's language mode, optimization level or flags. Its sources
//! are embedded in the binary, so the cache cannot outlive a change to
//! them. In WebAssembly the kept arena is one region of linear memory,
//! whose untouched part costs address space only (`util::vm`).

use std::{
    cell::OnceCell,
    path::Path,
};

use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    diagnostics::{
        ColorChoice,
        DiagnosticReporter,
    },
    headers::{
        HeaderSearch,
        WASI_DIRECTORY,
    },
    ir::{
        Module,
        Profile,
    },
    optimizer::OptimizerOptions,
    pipeline::{
        InlineCopies,
        analyze_translation_unit,
        lower_translation_unit_with_locals,
        parse_translation_unit,
    },
    target::Target,
    translation_phases::{
        Context,
        FileSystem,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

/// The runtime's C sources, by name.
pub(crate) const SOURCES: &[(&str, &str)] = &[
    ("crt.c", include_str!("runtime/crt.c")),
    ("malloc.c", include_str!("runtime/malloc.c")),
    ("stdio.c", include_str!("runtime/stdio.c")),
    ("string.c", include_str!("runtime/string.c")),
    ("stdlib.c", include_str!("runtime/stdlib.c")),
    ("floatscan.c", include_str!("runtime/floatscan.c")),
    ("math.c", include_str!("runtime/math.c")),
    ("trig.c", include_str!("runtime/trig.c")),
    ("compiler_rt.c", include_str!("runtime/compiler_rt.c")),
];

/// The runtime's private headers.
const HEADERS: &[(&str, &str)] = &[("wasi.h", include_str!("runtime/wasi.h"))];

/// The runtime's modules for `target` (`wasm32-wasip1`), compiled on this
/// thread's first call and kept for the rest of the process. A failure is
/// kept too: it is a bug of bcc, reported with the source's diagnostics.
/// Modules hold arena references that are not `Sync`, so each thread keeps
/// its own copy.
pub(crate) fn modules(target: Target) -> Result<&'static [Module<'static>], &'static str> {
    thread_local! {
        static WASI: OnceCell<Result<&'static [Module<'static>], &'static str>> =
            const { OnceCell::new() };
    }
    if target != Target::Wasm32Wasi {
        return Err("only the wasm32-wasip1 target links the bundled runtime");
    }
    WASI.with(|cached| {
        *cached.get_or_init(|| {
            #[expect(
                clippy::disallowed_types,
                reason = "The runtime's modules live for the process: their arena is leaked once                           per thread, never freed."
            )]
            let arena: &'static Bump = Box::leak(Box::new(Bump::new()));
            compile(target, arena).map(|modules| &*modules.leak())
        })
    })
}

/// Compiles every runtime source for `target` into `arena`, optimized by
/// bcc's pipeline. A source that fails to compile is a bug of bcc, reported
/// with its diagnostics.
pub(crate) fn compile(target: Target, arena: &Bump) -> Result<ArenaVec<'_, Module<'_>>, &str> {
    let mut modules = ArenaVec::with_capacity_in(SOURCES.len(), arena);
    for &(name, source) in SOURCES {
        modules.push(compile_source(target, name, source, arena)?);
    }
    Ok(modules)
}

/// Compiles one runtime source.
fn compile_source<'ir>(
    target: Target,
    name: &str,
    source: &str,
    arena: &'ir Bump,
) -> Result<Module<'ir>, &'ir str> {
    let configuration = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
        .with_gnu_extensions(true)
        .with_target(target)
        .with_optimization((true, false))
        .with_suppress_warnings(true);
    let tu = Bump::new();
    let mut context = Context::with_configuration(&tu, configuration);
    context.file_system = FileSystem::InMemory(
        tu.alloc_slice_fill_iter(
            HEADERS
                .iter()
                .map(|&(header, text)| (Path::new(header), text)),
        ),
    );
    let source = tu.alloc_str(source);
    let search = HeaderSearch {
        angled: &[Path::new("")],
        after: &[Path::new(WASI_DIRECTORY)],
        ..HeaderSearch::default()
    };
    let path = Path::new(name);
    let unit = parse_translation_unit(&mut context, path, source, search);
    let sema = analyze_translation_unit(&mut context, &unit);
    let failed = |context: &mut Context<'_>| -> &'ir str {
        let reporter_arena = Bump::new();
        let mut reporter =
            DiagnosticReporter::new(&reporter_arena, context.tu_arena(), ColorChoice::Plain);
        let mut rendered = Rendered(ArenaString::new_in(arena));
        drop(reporter.report_pending(context, &mut rendered));
        drop(reporter.finish(context, &mut rendered));
        let mut text = ArenaString::new_in(arena);
        _ = core::fmt::write(
            &mut text,
            format_args!(
                "the bundled runtime's {name} does not compile:\n{}",
                rendered.0.as_str()
            ),
        );
        text.into_str()
    };
    if !sema.lowerable(&context) {
        return Err(failed(&mut context));
    }
    let optimizer = OptimizerOptions::default();
    let lowered = lower_translation_unit_with_locals(
        &context,
        &unit,
        &sema,
        arena,
        Some(&optimizer),
        Profile::PostAbi,
        (InlineCopies::Resolve, false),
    );
    match lowered {
        | Ok(diagnosed) => Ok(diagnosed.module),
        | Err(error) => {
            let mut text = ArenaString::new_in(arena);
            _ = core::fmt::write(&mut text, format_args!("{}{error:?}", failed(&mut context)));
            Err(text.into_str())
        },
    }
}

/// Rendered diagnostics, as text.
struct Rendered<'a>(ArenaString<'a>);

impl std::io::Write for Rendered<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for chunk in buf.utf8_chunks() {
            self.0.push_str(chunk.valid());
            if !chunk.invalid().is_empty() {
                self.0.push('\u{FFFD}');
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
