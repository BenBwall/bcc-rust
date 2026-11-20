//! The pedantic options and the C11 and C23 extension groups in every order,
//! with `-w` and `-Werror`, under the Clang and GCC personalities and bcc's own
//! command line: what each option makes of extensions reported by the
//! preprocessor, the parser and semantic analysis, and of the four C23
//! extensions Clang warns of by default. Each expectation is the severities in
//! order, the closing lines and the exit status that Clang 23.1.5 and GCC 14
//! gave for the same `-std=c99 -fsyntax-only` command line, recorded on
//! 2026-11-20 (`docs/design/driver-modes.md`, "Warning options").
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary directories, command inputs and captured output."
)]

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        process::Command,
    };

    /// The sources, each with one extension diagnostic: a zero-length array
    /// (`-Wpedantic` alone), `_Static_assert` (C11), a binary constant (a
    /// C23 extension the preprocessor reports, off by default), a label at
    /// the end of a block and an unnamed parameter (C23 extensions Clang
    /// warns of by default), arithmetic on `void *` (semantic analysis;
    /// GCC's `-Wpointer-arith`) and `#elifdef` (the preprocessor's C23
    /// directive, which Clang warns of by default).
    const SOURCES: &[(&str, &str)] = &[
        ("parse_zero", "int a[0];\n"),
        ("parse_sa", "_Static_assert(1, \"m\");\n"),
        ("lex_binary", "int b = 0b101;\n"),
        ("c23_label_end", "void f(void) {\nl:\n}\n"),
        ("c23_unnamed", "void f(int) {}\n"),
        ("sema_ptrarith", "void *p;\nvoid f(void) { p = p + 1; }\n"),
        ("pp_elifdef", "#ifdef A\n#elifdef B\n#endif\nint x;\n"),
    ];

    /// (source, options, Clang's severities and closing lines, Clang's
    /// status, GCC's, GCC's status). Clang prints no counts without its
    /// caret, which the cases turn off.
    type Case = (
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
        i32,
        &'static [&'static str],
        i32,
    );

    const CASES: &[Case] = &[
        ("parse_zero", &[], &[], 0, &[], 0),
        (
            "parse_zero",
            &["-pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("parse_zero", &["-Werror"], &[], 0, &[], 0),
        ("parse_zero", &["-pedantic-errors", "-w"], &[], 0, &[], 0),
        ("parse_zero", &["-w", "-pedantic-errors"], &[], 0, &[], 0),
        (
            "parse_zero",
            &["-pedantic-errors", "-Wno-c23-extensions"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "parse_zero",
            &["-pedantic-errors", "-Wno-c11-extensions"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "parse_zero",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_zero",
            &["-Wno-error=pedantic", "-pedantic-errors"],
            &["warning"],
            0,
            &["error"],
            1,
        ),
        (
            "parse_zero",
            &["-pedantic-errors", "-Wno-pedantic"],
            &[],
            0,
            &[],
            0,
        ),
        ("parse_zero", &["-Werror=c23-extensions"], &[], 0, &[], 1),
        ("parse_zero", &["-Werror=c11-extensions"], &[], 0, &[], 1),
        ("parse_zero", &["-Wc23-extensions"], &[], 0, &[], 1),
        (
            "parse_zero",
            &["-Werror=pedantic"],
            &["error"],
            1,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "parse_zero",
            &["-Werror=pedantic", "-Wpedantic"],
            &["error"],
            1,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "parse_zero",
            &["-Werror=pedantic", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("parse_zero", &["-Werror=pedantic", "-w"], &[], 0, &[], 0),
        (
            "parse_zero",
            &["-Wno-pedantic", "-pedantic"],
            &[],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_zero",
            &["-Wno-pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_zero",
            &["-Wpedantic", "-Wno-c11-extensions"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_zero",
            &["-Wno-c11-extensions", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_zero",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_zero",
            &["-Werror", "-Wno-error=c23-extensions"],
            &[],
            0,
            &[],
            1,
        ),
        ("parse_sa", &[], &[], 0, &[], 0),
        ("parse_sa", &["-pedantic"], &["warning"], 0, &["warning"], 0),
        ("parse_sa", &["-Werror"], &[], 0, &[], 0),
        ("parse_sa", &["-pedantic-errors", "-w"], &[], 0, &[], 0),
        ("parse_sa", &["-w", "-pedantic-errors"], &[], 0, &[], 0),
        (
            "parse_sa",
            &["-pedantic-errors", "-Wno-c23-extensions"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "parse_sa",
            &["-pedantic-errors", "-Wno-c11-extensions"],
            &[],
            0,
            &["error"],
            1,
        ),
        (
            "parse_sa",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_sa",
            &["-Wno-error=pedantic", "-pedantic-errors"],
            &["warning"],
            0,
            &["error"],
            1,
        ),
        (
            "parse_sa",
            &["-pedantic-errors", "-Wno-pedantic"],
            &[],
            0,
            &[],
            0,
        ),
        ("parse_sa", &["-Werror=c23-extensions"], &[], 0, &[], 1),
        (
            "parse_sa",
            &["-Werror=c11-extensions"],
            &["error"],
            1,
            &[],
            1,
        ),
        ("parse_sa", &["-Wc23-extensions"], &[], 0, &[], 1),
        (
            "parse_sa",
            &["-Werror=pedantic"],
            &["error"],
            1,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "parse_sa",
            &["-Werror=pedantic", "-Wpedantic"],
            &["error"],
            1,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "parse_sa",
            &["-Werror=pedantic", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("parse_sa", &["-Werror=pedantic", "-w"], &[], 0, &[], 0),
        (
            "parse_sa",
            &["-Wno-pedantic", "-pedantic"],
            &[],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_sa",
            &["-Wno-pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_sa",
            &["-Wpedantic", "-Wno-c11-extensions"],
            &[],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_sa",
            &["-Wno-c11-extensions", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_sa",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "parse_sa",
            &["-Werror", "-Wno-error=c23-extensions"],
            &[],
            0,
            &[],
            1,
        ),
        ("lex_binary", &[], &[], 0, &[], 0),
        (
            "lex_binary",
            &["-pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("lex_binary", &["-Werror"], &[], 0, &[], 0),
        ("lex_binary", &["-pedantic-errors", "-w"], &[], 0, &[], 0),
        ("lex_binary", &["-w", "-pedantic-errors"], &[], 0, &[], 0),
        (
            "lex_binary",
            &["-pedantic-errors", "-Wno-c23-extensions"],
            &[],
            0,
            &["error"],
            1,
        ),
        (
            "lex_binary",
            &["-pedantic-errors", "-Wno-c11-extensions"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "lex_binary",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["error"],
            1,
        ),
        (
            "lex_binary",
            &["-Wno-error=pedantic", "-pedantic-errors"],
            &["warning"],
            0,
            &["error"],
            1,
        ),
        (
            "lex_binary",
            &["-pedantic-errors", "-Wno-pedantic"],
            &[],
            0,
            &[],
            0,
        ),
        (
            "lex_binary",
            &["-Werror=c23-extensions"],
            &["error"],
            1,
            &[],
            1,
        ),
        ("lex_binary", &["-Werror=c11-extensions"], &[], 0, &[], 1),
        ("lex_binary", &["-Wc23-extensions"], &["warning"], 0, &[], 1),
        (
            "lex_binary",
            &["-Werror=pedantic"],
            &["error"],
            1,
            &["warning"],
            0,
        ),
        (
            "lex_binary",
            &["-Werror=pedantic", "-Wpedantic"],
            &["error"],
            1,
            &["warning"],
            0,
        ),
        (
            "lex_binary",
            &["-Werror=pedantic", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("lex_binary", &["-Werror=pedantic", "-w"], &[], 0, &[], 0),
        (
            "lex_binary",
            &["-Wno-pedantic", "-pedantic"],
            &[],
            0,
            &["warning"],
            0,
        ),
        (
            "lex_binary",
            &["-Wno-pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "lex_binary",
            &["-Wpedantic", "-Wno-c11-extensions"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "lex_binary",
            &["-Wno-c11-extensions", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "lex_binary",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["error", "cc1: all warnings being treated as errors"],
            1,
        ),
        (
            "lex_binary",
            &["-Werror", "-Wno-error=c23-extensions"],
            &[],
            0,
            &[],
            1,
        ),
        ("c23_label_end", &[], &["warning"], 0, &[], 0),
        (
            "c23_label_end",
            &["-pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("c23_label_end", &["-Werror"], &["error"], 1, &[], 0),
        ("c23_label_end", &["-pedantic-errors", "-w"], &[], 0, &[], 0),
        ("c23_label_end", &["-w", "-pedantic-errors"], &[], 0, &[], 0),
        (
            "c23_label_end",
            &["-pedantic-errors", "-Wno-c23-extensions"],
            &[],
            0,
            &["error"],
            1,
        ),
        (
            "c23_label_end",
            &["-pedantic-errors", "-Wno-c11-extensions"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "c23_label_end",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["error"],
            1,
            &["warning"],
            0,
        ),
        (
            "c23_label_end",
            &["-Wno-error=pedantic", "-pedantic-errors"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "c23_label_end",
            &["-pedantic-errors", "-Wno-pedantic"],
            &["error"],
            1,
            &[],
            0,
        ),
        (
            "c23_label_end",
            &["-Werror=c23-extensions"],
            &["error"],
            1,
            &[],
            1,
        ),
        (
            "c23_label_end",
            &["-Werror=c11-extensions"],
            &["warning"],
            0,
            &[],
            1,
        ),
        (
            "c23_label_end",
            &["-Wc23-extensions"],
            &["warning"],
            0,
            &[],
            1,
        ),
        (
            "c23_label_end",
            &["-Werror=pedantic"],
            &["warning"],
            0,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "c23_label_end",
            &["-Werror=pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "c23_label_end",
            &["-Werror=pedantic", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("c23_label_end", &["-Werror=pedantic", "-w"], &[], 0, &[], 0),
        (
            "c23_label_end",
            &["-Wno-pedantic", "-pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "c23_label_end",
            &["-Wno-pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "c23_label_end",
            &["-Wpedantic", "-Wno-c11-extensions"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "c23_label_end",
            &["-Wno-c11-extensions", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "c23_label_end",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &["error"],
            1,
            &["warning"],
            0,
        ),
        (
            "c23_label_end",
            &["-Werror", "-Wno-error=c23-extensions"],
            &["warning"],
            0,
            &[],
            1,
        ),
        ("c23_unnamed", &[], &["warning"], 0, &[], 0),
        (
            "c23_unnamed",
            &["-pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("c23_unnamed", &["-Werror"], &["error"], 1, &[], 0),
        ("c23_unnamed", &["-pedantic-errors", "-w"], &[], 0, &[], 0),
        ("c23_unnamed", &["-w", "-pedantic-errors"], &[], 0, &[], 0),
        (
            "c23_unnamed",
            &["-pedantic-errors", "-Wno-c23-extensions"],
            &[],
            0,
            &["error"],
            1,
        ),
        (
            "c23_unnamed",
            &["-pedantic-errors", "-Wno-c11-extensions"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "c23_unnamed",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["error"],
            1,
            &["warning"],
            0,
        ),
        (
            "c23_unnamed",
            &["-Wno-error=pedantic", "-pedantic-errors"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "c23_unnamed",
            &["-pedantic-errors", "-Wno-pedantic"],
            &["error"],
            1,
            &[],
            0,
        ),
        (
            "c23_unnamed",
            &["-Werror=c23-extensions"],
            &["error"],
            1,
            &[],
            1,
        ),
        (
            "c23_unnamed",
            &["-Werror=c11-extensions"],
            &["warning"],
            0,
            &[],
            1,
        ),
        (
            "c23_unnamed",
            &["-Wc23-extensions"],
            &["warning"],
            0,
            &[],
            1,
        ),
        (
            "c23_unnamed",
            &["-Werror=pedantic"],
            &["warning"],
            0,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "c23_unnamed",
            &["-Werror=pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "c23_unnamed",
            &["-Werror=pedantic", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("c23_unnamed", &["-Werror=pedantic", "-w"], &[], 0, &[], 0),
        (
            "c23_unnamed",
            &["-Wno-pedantic", "-pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "c23_unnamed",
            &["-Wno-pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "c23_unnamed",
            &["-Wpedantic", "-Wno-c11-extensions"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "c23_unnamed",
            &["-Wno-c11-extensions", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "c23_unnamed",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &["error"],
            1,
            &["warning"],
            0,
        ),
        (
            "c23_unnamed",
            &["-Werror", "-Wno-error=c23-extensions"],
            &["warning"],
            0,
            &[],
            1,
        ),
        ("sema_ptrarith", &[], &[], 0, &[], 0),
        (
            "sema_ptrarith",
            &["-pedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        ("sema_ptrarith", &["-Werror"], &[], 0, &[], 0),
        ("sema_ptrarith", &["-pedantic-errors", "-w"], &[], 0, &[], 0),
        ("sema_ptrarith", &["-w", "-pedantic-errors"], &[], 0, &[], 0),
        (
            "sema_ptrarith",
            &["-pedantic-errors", "-Wno-c23-extensions"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "sema_ptrarith",
            &["-pedantic-errors", "-Wno-c11-extensions"],
            &["error"],
            1,
            &["error"],
            1,
        ),
        (
            "sema_ptrarith",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["error"],
            1,
        ),
        (
            "sema_ptrarith",
            &["-Wno-error=pedantic", "-pedantic-errors"],
            &["warning"],
            0,
            &["error"],
            1,
        ),
        (
            "sema_ptrarith",
            &["-pedantic-errors", "-Wno-pedantic"],
            &[],
            0,
            &[],
            0,
        ),
        ("sema_ptrarith", &["-Werror=c23-extensions"], &[], 0, &[], 1),
        ("sema_ptrarith", &["-Werror=c11-extensions"], &[], 0, &[], 1),
        ("sema_ptrarith", &["-Wc23-extensions"], &[], 0, &[], 1),
        (
            "sema_ptrarith",
            &["-Werror=pedantic"],
            &["error"],
            1,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "sema_ptrarith",
            &["-Werror=pedantic", "-Wpedantic"],
            &["error"],
            1,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        (
            "sema_ptrarith",
            &["-Werror=pedantic", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["error", "cc1: some warnings being treated as errors"],
            1,
        ),
        ("sema_ptrarith", &["-Werror=pedantic", "-w"], &[], 0, &[], 0),
        (
            "sema_ptrarith",
            &["-Wno-pedantic", "-pedantic"],
            &[],
            0,
            &["warning"],
            0,
        ),
        (
            "sema_ptrarith",
            &["-Wno-pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "sema_ptrarith",
            &["-Wpedantic", "-Wno-c11-extensions"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "sema_ptrarith",
            &["-Wno-c11-extensions", "-Wpedantic"],
            &["warning"],
            0,
            &["warning"],
            0,
        ),
        (
            "sema_ptrarith",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &["error", "cc1: all warnings being treated as errors"],
            1,
        ),
        (
            "sema_ptrarith",
            &["-Werror", "-Wno-error=c23-extensions"],
            &[],
            0,
            &[],
            1,
        ),
        ("pp_elifdef", &[], &["warning"], 0, &[], 0),
        ("pp_elifdef", &["-pedantic"], &["warning"], 0, &[], 0),
        ("pp_elifdef", &["-Werror"], &["error"], 1, &[], 0),
        ("pp_elifdef", &["-pedantic-errors", "-w"], &[], 0, &[], 0),
        ("pp_elifdef", &["-w", "-pedantic-errors"], &[], 0, &[], 0),
        (
            "pp_elifdef",
            &["-pedantic-errors", "-Wno-c23-extensions"],
            &[],
            0,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-pedantic-errors", "-Wno-c11-extensions"],
            &["error"],
            1,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["error"],
            1,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-Wno-error=pedantic", "-pedantic-errors"],
            &["error"],
            1,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-pedantic-errors", "-Wno-pedantic"],
            &["error"],
            1,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-Werror=c23-extensions"],
            &["error"],
            1,
            &[],
            1,
        ),
        (
            "pp_elifdef",
            &["-Werror=c11-extensions"],
            &["warning"],
            0,
            &[],
            1,
        ),
        ("pp_elifdef", &["-Wc23-extensions"], &["warning"], 0, &[], 1),
        ("pp_elifdef", &["-Werror=pedantic"], &["warning"], 0, &[], 0),
        (
            "pp_elifdef",
            &["-Werror=pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-Werror=pedantic", "-Wno-error=pedantic"],
            &["warning"],
            0,
            &[],
            0,
        ),
        ("pp_elifdef", &["-Werror=pedantic", "-w"], &[], 0, &[], 0),
        (
            "pp_elifdef",
            &["-Wno-pedantic", "-pedantic"],
            &["warning"],
            0,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-Wno-pedantic", "-Wpedantic"],
            &["warning"],
            0,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-Wpedantic", "-Wno-c11-extensions"],
            &["warning"],
            0,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-Wno-c11-extensions", "-Wpedantic"],
            &["warning"],
            0,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &["error"],
            1,
            &[],
            0,
        ),
        (
            "pp_elifdef",
            &["-Werror", "-Wno-error=c23-extensions"],
            &["warning"],
            0,
            &[],
            1,
        ),
    ];

    /// Each diagnostic's severity and each closing line, in order, from GCC's
    /// and Clang's `source:line:column: severity:` lines or bcc's own
    /// `severity: message` lines.
    fn sequence(stderr: &str, source: &str, own: bool) -> Vec<String> {
        stderr
            .lines()
            .filter_map(|line| {
                let severity = if own {
                    line.split_once(": ").map(|(severity, _)| severity)
                } else if line.starts_with(source) {
                    line.split(": ").nth(1)
                } else {
                    None
                };
                match severity {
                    | Some(severity @ ("warning" | "error" | "fatal error")) =>
                        Some(severity.to_owned()),
                    | _ if !own
                        && (line.ends_with("being treated as errors")
                            || line.ends_with("generated.")) =>
                        Some(line.to_owned()),
                    | _ => None,
                }
            })
            .collect()
    }

    #[test]
    fn pedantic_options_classify_extensions_as_clang_and_gcc_do() {
        let directory: PathBuf =
            std::env::temp_dir().join(format!("bcc-pedantic-options-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for (name, text) in SOURCES {
            std::fs::write(directory.join(format!("{name}.c")), text).unwrap();
        }
        let mut failures = Vec::new();
        for &(name, options, clang, clang_status, gcc, gcc_status) in CASES {
            // bcc's own command line follows Clang's rules, though not its
            // strict modes' refusal of a binary constant, and has no C23
            // directive warning outside the Clang identity.
            let own = !matches!(name, "lex_binary" | "pp_elifdef");
            for (mode, expected, status) in [
                (Some("--clang-mode"), clang, clang_status),
                (Some("--gcc-mode"), gcc, gcc_status),
                (None, clang, clang_status),
            ] {
                if mode.is_none() && !own {
                    continue;
                }
                let caret = match mode {
                    | Some("--gcc-mode") => Some("-fno-diagnostics-show-caret"),
                    | Some(_) => Some("-fno-caret-diagnostics"),
                    | None => None,
                };
                let source = format!("{name}.c");
                let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                    .args(mode)
                    .arg("-std=c99")
                    .args(options)
                    .arg("-fsyntax-only")
                    .args(caret)
                    .arg(&source)
                    .current_dir(&directory)
                    .env("NO_COLOR", "1")
                    .output()
                    .unwrap();
                let stderr = String::from_utf8_lossy(&output.stderr);
                let found = sequence(&stderr, &source, mode.is_none());
                let expected: Vec<String> = expected
                    .iter()
                    .filter(|line| mode.is_some() || !line.contains(' '))
                    .map(|line| (*line).to_owned())
                    .collect();
                if found != expected || output.status.code() != Some(status) {
                    failures.push(format!(
                        "{} {options:?} {name}: expected {expected:?} and {status}, found \
                         {found:?} and {:?}:\n{stderr}",
                        mode.unwrap_or("bcc"),
                        output.status.code()
                    ));
                }
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
