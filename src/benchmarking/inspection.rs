use super::{
    Bump,
    ParseBenchmarkSummary,
    Path,
    benchmark_context,
};

/// Runs translation phases 1 through 7 over `source`, copied into the
/// translation-unit arena as the CLI's `--input` is, and summarizes the
/// parse.
#[doc(hidden)]
#[must_use]
pub fn parse_source(source: &str) -> ParseBenchmarkSummary {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    summarize_parse(
        &tu,
        Path::new("<input>"),
        source,
        crate::headers::HeaderSearch::default(),
    )
}

/// Runs phases 1 through 7 with all MSVC groups enabled, for allocation tests.
#[doc(hidden)]
#[must_use]
pub fn parse_msvc_source(source: &str) -> ParseBenchmarkSummary {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let mut context = benchmark_context(&tu);
    context.configuration = context.configuration.with_msvc_extensions(true);
    let unit = crate::pipeline::parse_translation_unit(
        &mut context,
        Path::new("<input>"),
        source,
        crate::headers::HeaderSearch::default(),
    );
    ParseBenchmarkSummary {
        external_declarations: unit.external_declarations().len(),
        diagnostics:           context.pending_error_count(),
    }
}

/// Reads `path` and runs translation phases 1 through 7 over it as the CLI
/// does, with no include directories, and summarizes the parse.
///
/// # Errors
///
/// When `path` cannot be read.
#[doc(hidden)]
pub fn parse_file(path: &Path) -> std::io::Result<ParseBenchmarkSummary> {
    let tu = Bump::new();
    let source = tu.read_to_str_lossy(path)?;
    Ok(summarize_parse(
        &tu,
        path,
        source,
        crate::headers::HeaderSearch::default(),
    ))
}

/// Like [`parse_file`], with `library` as the C library's include directory
/// after the resource directory, so the resource headers chain to it.
///
/// # Errors
///
/// When `path` cannot be read.
#[doc(hidden)]
pub fn parse_file_with_library(
    path: &Path,
    library: &Path,
) -> std::io::Result<ParseBenchmarkSummary> {
    let tu = Bump::new();
    let source = tu.read_to_str_lossy(path)?;
    let after = [library];
    Ok(summarize_parse(
        &tu,
        path,
        source,
        crate::headers::HeaderSearch {
            after: &after,
            ..crate::headers::HeaderSearch::default()
        },
    ))
}

pub(super) fn summarize_parse<'tu>(
    tu: &'tu Bump,
    path: &Path,
    source: &'tu str,
    search: crate::headers::HeaderSearch<'_>,
) -> ParseBenchmarkSummary {
    let mut context = benchmark_context(tu);
    let unit = crate::pipeline::parse_translation_unit(&mut context, path, source, search);
    ParseBenchmarkSummary {
        external_declarations: unit.external_declarations().len(),
        diagnostics:           context.pending_error_count(),
    }
}
