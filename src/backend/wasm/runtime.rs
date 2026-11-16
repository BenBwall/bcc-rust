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
//! floating conversions) and a small `scanf`, the rounding functions of
//! `<math.h>`, and the functions of `<stdbit.h>`, whose header is bcc's
//! resource header. It talks to the host only through WASI preview 1 imports
//! (`fd_write`, `fd_read`, `proc_exit`, `args_get`, `environ_get`,
//! `clock_time_get`), so Node's `wasi` module, wasmtime or a browser
//! shim runs the result. Files cannot be opened.
//!
//! [`with_modules`] compiles every runtime source on a thread's first link
//! and keeps the modules in an arena until the thread exits, so the
//! explorer, which compiles many programs in one session on one thread,
//! pays for the runtime once. A compilation stack is a thread of its own,
//! so a process that compiles each program on a new one (the command line,
//! a test) frees the runtime with each compilation rather than keeping a
//! copy for every thread it ever ran. The runtime's build depends only on
//! its target: it is always optimized C17 with GNU extensions, whatever
//! the program's language mode, optimization level or flags. Its sources
//! are embedded in the binary, so the cache cannot outlive a change to
//! them. In WebAssembly the kept arena is one region of linear memory,
//! whose untouched part costs address space only (`util::vm`).

use std::{
    cell::OnceCell,
    path::Path,
    ptr::NonNull,
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
    ("stdbit.c", include_str!("runtime/stdbit.c")),
    ("compiler_rt.c", include_str!("runtime/compiler_rt.c")),
];

/// The runtime's private headers.
const HEADERS: &[(&str, &str)] = &[("wasi.h", include_str!("runtime/wasi.h"))];

/// Calls `link` with the runtime's modules for `target` (`wasm32-wasip1`),
/// compiled on this thread's first call and kept until the thread exits. A
/// failure is kept too: it is a bug of bcc, reported with the source's
/// diagnostics. Modules hold arena references that are not `Sync`, so each
/// thread keeps its own copy, and `link` borrows it only for the call, so
/// nothing that outlives the thread can point into it.
pub(crate) fn with_modules<R>(
    target: Target,
    link: impl FnOnce(Result<&[Module<'_>], &str>) -> R,
) -> R {
    thread_local! {
        static WASI: OnceCell<Kept> = const { OnceCell::new() };
    }
    if target != Target::Wasm32Wasi {
        return link(Err(
            "only the wasm32-wasip1 target links the bundled runtime"
        ));
    }
    WASI.with(|kept| link(kept.get_or_init(|| Kept::compile(target)).modules()))
}

/// A thread's compiled runtime and the arena that holds it, which it frees
/// when the thread exits.
struct Kept {
    /// Point into `arena`; [`Kept::modules`] lends them only for a borrow
    /// of `self`.
    modules: Result<&'static [Module<'static>], &'static str>,
    /// Owned: from [`Box::into_raw`], freed by [`Kept`]'s `Drop`. A raw
    /// pointer rather than a `Box`, whose moves would claim unique access
    /// while `modules` borrow it.
    arena:   NonNull<Bump>,
}

impl Kept {
    fn compile(target: Target) -> Self {
        #[expect(
            clippy::disallowed_types,
            reason = "The runtime's arena lives as long as its thread, past every compilation's \
                      arenas, so it is boxed and freed when the thread exits."
        )]
        let arena = NonNull::from(Box::leak(Box::new(Bump::new())));
        // SAFETY: the arena stays allocated, and is not mutably borrowed,
        // until `Kept` drops; `modules` lends what it holds only for a
        // borrow of `Kept`, which ends first.
        let shared: &'static Bump = unsafe { arena.as_ref() };
        let modules = compile(target, shared).map(|modules| &*modules.leak());
        Self { modules, arena }
    }

    /// The modules, for as long as `self` is borrowed (the `'static` of the
    /// field shortens to it).
    fn modules(&self) -> Result<&[Module<'_>], &str> {
        self.modules
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        #[expect(
            clippy::disallowed_types,
            reason = "Frees the arena boxed in Kept::compile."
        )]
        // SAFETY: `arena` came from `Box::leak` in `Kept::compile` and is
        // freed only here. Every module and message in it was lent for a
        // borrow of `self`, so none is still in use.
        let arena = unsafe { Box::from_raw(self.arena.as_ptr()) };
        drop(arena);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::vm::accounting;

    /// A thread's kept runtime returns its arena's memory when the thread
    /// exits, so a process that links on many threads does not keep a copy
    /// for each.
    #[test]
    fn a_kept_runtime_frees_its_arena_when_dropped() {
        let before = accounting::live();
        let kept = Kept::compile(Target::Wasm32Wasi);
        _ = kept.modules().unwrap();
        assert!(
            accounting::live().committed > before.committed,
            "the modules live in the kept arena"
        );
        drop(kept);
        assert_eq!(accounting::live(), before);
    }
}
