//! The pedantic options and the C11 and C23 extension groups in every order,
//! with `-w` and `-Werror`, under the Clang and GCC personalities and bcc's own
//! command line: what each option makes of extensions reported by the
//! preprocessor, the parser and semantic analysis, and of the four C23
//! extensions Clang warns of by default. Each expectation is the severities in
//! order, the closing lines and the exit status that Clang 23.1.5 and GCC 14
//! gave for the same `-std=c99 -fsyntax-only` command line, recorded on
//! 2026-11-20 (`docs/design/driver-modes.md`, "Warning options"). A second
//! table pins the extensions each compiler files under a group of its own, as
//! Clang a typedef redefinition and GCC `long long` before C99, with the
//! option each diagnostic's tag names.
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

    /// The sources of the extension diagnostics whose group decides how the
    /// options reach them and which their tag names: a typedef redefinition
    /// (Clang: `-Wtypedef-redefinition`, a warning by default), an omitted
    /// variadic argument (Clang: `-Wvariadic-macro-arguments-omitted`, which
    /// `-Wc23-extensions` holds), `_Noreturn` (`-Wc11-extensions`), a label at
    /// the end of a block (`-Wc23-extensions`), `#warning` (`-Wpedantic`),
    /// variadic macros (`-Wvariadic-macros`), C99's mixed declarations, a
    /// variable length array and `long long` before C99 (GCC:
    /// `-Wdeclaration-after-statement`, `-Wvla` and `-Wlong-long`), and
    /// `sizeof(void)` and arithmetic on a function pointer (GCC:
    /// `-Wpointer-arith`), and `__VA_OPT__` outside a variadic macro (Clang:
    /// `-Wvariadic-macros`, a warning by default).
    const TAGGED_SOURCES: &[(&str, &str)] = &[
        ("typedef_redef", "typedef int T;\ntypedef int T;\n"),
        ("va_omit", "#define F(a, ...) a\nint x = F(1);\n"),
        ("c11_noreturn", "_Noreturn void f(void);\n"),
        ("c23_label_end", "void f(void) {\nl:\n}\n"),
        ("warning_dir", "#warning hi\nint x;\n"),
        ("c90_variadic", "#define F(...) __VA_ARGS__\nint x;\n"),
        ("named_va", "#define F(a...) a\nint x;\n"),
        (
            "c90_mixed",
            "void f(void) { int a; a = 1; int b; (void)a; (void)b; }\n",
        ),
        ("c90_vla", "void f(int n) { int a[n]; (void)a; }\n"),
        ("c90_longlong", "long long x;\n"),
        (
            "ptr_sizeof_void",
            "int n = sizeof(void);\nvoid (*fp)(void);\nvoid g(void) { fp = fp + 1; }\n",
        ),
        ("vaopt_misplaced", "#define F(x) __VA_OPT__(x)\nint y;\n"),
    ];

    /// (personality, `-std`, source, options, each diagnostic's severity with
    /// the option its tag names and each closing line, exit status). The
    /// personalities' expectations are Clang 23.1.5's and GCC 14's output for
    /// the same command line, recorded on 2026-11-20; bcc's own command line
    /// (`None`) follows Clang's rules in its own format, which names no
    /// `-Werror`, and keeps its own words for an omitted variadic argument.
    /// GCC's `-Wno-` forms of its own options turn their warnings off in
    /// either order; their `-Werror=` and `-Wno-error=` forms classify them
    /// in argv order.
    type TaggedCase = (
        Option<&'static str>,
        &'static str,
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
        i32,
    );

    const CLANG: Option<&str> = Some("--clang-mode");
    const GCC: Option<&str> = Some("--gcc-mode");
    const OWN: Option<&str> = None;

    const TAGGED_CASES: &[TaggedCase] = &[
        (
            CLANG,
            "c99",
            "typedef_redef",
            &[],
            &["warning [-Wtypedef-redefinition]"],
            0,
        ),
        (
            CLANG,
            "c99",
            "typedef_redef",
            &["-Werror=pedantic"],
            &["warning [-Wtypedef-redefinition]"],
            0,
        ),
        (
            CLANG,
            "c99",
            "typedef_redef",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &["error [-Werror,-Wtypedef-redefinition]"],
            1,
        ),
        (
            CLANG,
            "c99",
            "typedef_redef",
            &["-pedantic", "-Wno-typedef-redefinition"],
            &[],
            0,
        ),
        (
            CLANG,
            "c99",
            "typedef_redef",
            &["-Werror=typedef-redefinition"],
            &["error [-Werror,-Wtypedef-redefinition]"],
            1,
        ),
        (
            CLANG,
            "c99",
            "typedef_redef",
            &["-Werror", "-Wno-error=typedef-redefinition"],
            &["warning [-Wtypedef-redefinition]"],
            0,
        ),
        (CLANG, "c99", "typedef_redef", &["-w"], &[], 0),
        (CLANG, "c11", "typedef_redef", &["-pedantic-errors"], &[], 0),
        (
            OWN,
            "c99",
            "typedef_redef",
            &[],
            &["warning [-Wtypedef-redefinition]"],
            0,
        ),
        (
            OWN,
            "c99",
            "typedef_redef",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &["error [-Wtypedef-redefinition]"],
            1,
        ),
        (
            GCC,
            "c99",
            "typedef_redef",
            &["-pedantic"],
            &["warning [-Wpedantic]"],
            0,
        ),
        (
            GCC,
            "c99",
            "typedef_redef",
            &["-pedantic", "-Werror"],
            &[
                "error [-Werror=pedantic]",
                "cc1: all warnings being treated as errors",
            ],
            1,
        ),
        (GCC, "c99", "typedef_redef", &[], &[], 0),
        (
            CLANG,
            "c99",
            "va_omit",
            &["-pedantic"],
            &["warning [-Wvariadic-macro-arguments-omitted]"],
            0,
        ),
        (
            CLANG,
            "c99",
            "va_omit",
            &["-Werror=c23-extensions"],
            &["error [-Werror,-Wvariadic-macro-arguments-omitted]"],
            1,
        ),
        (
            CLANG,
            "c99",
            "va_omit",
            &["-Wc23-extensions"],
            &["warning [-Wvariadic-macro-arguments-omitted]"],
            0,
        ),
        (
            CLANG,
            "c99",
            "va_omit",
            &["-Werror=pedantic", "-Wno-c23-extensions"],
            &[],
            0,
        ),
        (
            CLANG,
            "c99",
            "va_omit",
            &["-Werror", "-Wc11-extensions", "-Wc2x-extensions"],
            &["error [-Werror,-Wvariadic-macro-arguments-omitted]"],
            1,
        ),
        (
            CLANG,
            "c99",
            "va_omit",
            &["-Werror=c23-extensions", "-Wno-gnu"],
            &[],
            0,
        ),
        (
            CLANG,
            "c99",
            "va_omit",
            &["-Wno-gnu", "-Werror=c23-extensions"],
            &["error [-Werror,-Wvariadic-macro-arguments-omitted]"],
            1,
        ),
        (
            CLANG,
            "c99",
            "va_omit",
            &["-pedantic", "-Wno-variadic-macro-arguments-omitted"],
            &[],
            0,
        ),
        (
            CLANG,
            "c99",
            "va_omit",
            &["-Wpre-c23-compat"],
            &["warning [-Wvariadic-macro-arguments-omitted]"],
            0,
        ),
        // bcc's own command line applies `-Wgnu`'s forms to the one
        // diagnostic of that group it models, and warns that it does not
        // implement the rest; the personalities pass the options their
        // compiler takes without a word.
        (
            CLANG,
            "c99",
            "va_omit",
            &["-Wno-gnu", "-Wno-long-long"],
            &[],
            0,
        ),
        (
            OWN,
            "c99",
            "va_omit",
            &["-Wno-gnu"],
            &["warning [-Wunknown-warning-option]"],
            0,
        ),
        (
            OWN,
            "c99",
            "va_omit",
            &["-Werror", "-Wc11-extensions", "-Wc2x-extensions"],
            &["error"],
            1,
        ),
        (GCC, "c99", "va_omit", &["-pedantic"], &["warning"], 0),
        (
            GCC,
            "c99",
            "va_omit",
            &["-pedantic", "-Werror"],
            &[
                "error [-Werror]",
                "cc1: all warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c99",
            "va_omit",
            &["-Werror=pedantic"],
            &["warning"],
            0,
        ),
        (
            GCC,
            "c99",
            "va_omit",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["error"],
            1,
        ),
        (
            CLANG,
            "c99",
            "c11_noreturn",
            &["-pedantic"],
            &["warning [-Wc11-extensions]"],
            0,
        ),
        (
            CLANG,
            "c99",
            "c11_noreturn",
            &["-pedantic", "-Werror"],
            &["error [-Werror,-Wc11-extensions]"],
            1,
        ),
        (
            CLANG,
            "c99",
            "c23_label_end",
            &[],
            &["warning [-Wc23-extensions]"],
            0,
        ),
        (
            CLANG,
            "c99",
            "c23_label_end",
            &["-Werror"],
            &["error [-Werror,-Wc23-extensions]"],
            1,
        ),
        (
            OWN,
            "c99",
            "c23_label_end",
            &["-Werror"],
            &["error [-Wc23-extensions]"],
            1,
        ),
        (
            CLANG,
            "c99",
            "warning_dir",
            &["-pedantic", "-Werror"],
            &["error [-Werror,-Wpedantic]", "error [-Werror,-W#warnings]"],
            1,
        ),
        (
            CLANG,
            "c99",
            "named_va",
            &["-pedantic"],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        (
            CLANG,
            "c90",
            "c90_variadic",
            &["-pedantic", "-Werror"],
            &["error [-Werror,-Wvariadic-macros]"],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-Werror", "-Wpedantic"],
            &[
                "error [-Werror=variadic-macros]",
                "cc1: all warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-Werror=pedantic"],
            &[
                "error [-Werror=variadic-macros]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["error [-Wvariadic-macros]"],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-Werror=pedantic", "-Wno-variadic-macros"],
            &[],
            0,
        ),
        (
            GCC,
            "c99",
            "named_va",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &[
                "error [-Werror=variadic-macros]",
                "cc1: all warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_mixed",
            &["-Werror=pedantic"],
            &["warning [-Wdeclaration-after-statement]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_mixed",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &[
                "error [-Werror=declaration-after-statement]",
                "cc1: all warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_mixed",
            &["-pedantic-errors"],
            &["error [-Wdeclaration-after-statement]"],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_mixed",
            &["-pedantic-errors", "-Wno-error=pedantic"],
            &["error [-Wdeclaration-after-statement]"],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_vla",
            &["-pedantic"],
            &["warning [-Wvla]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_vla",
            &["-Werror=pedantic"],
            &["warning [-Wvla]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_vla",
            &["-pedantic", "-Werror", "-Wno-error=pedantic"],
            &[
                "error [-Werror=vla]",
                "cc1: all warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c99",
            "ptr_sizeof_void",
            &["-pedantic"],
            &["warning [-Wpointer-arith]", "warning [-Wpointer-arith]"],
            0,
        ),
        (
            GCC,
            "c99",
            "ptr_sizeof_void",
            &["-Werror=pedantic", "-Wno-error=pedantic"],
            &[
                "error [-Werror=pointer-arith]",
                "error [-Werror=pointer-arith]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c99",
            "ptr_sizeof_void",
            &["-pedantic", "-Wno-pointer-arith"],
            &[],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_longlong",
            &["-Wno-long-long", "-pedantic"],
            &[],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_longlong",
            &["-Wno-long-long", "-Wlong-long", "-pedantic"],
            &["warning [-Wlong-long]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_longlong",
            &["-pedantic", "-Werror"],
            &[
                "error [-Werror=long-long]",
                "cc1: all warnings being treated as errors",
            ],
            1,
        ),
        // Every form of GCC's own options, in argv order: `-Werror=` enables
        // and classifies, `-Wno-error=` only classifies, and a `-W`, `-Wno-`
        // or `-Werror=` form ahead of `-Werror=pedantic` or
        // `-pedantic-errors` keeps them from classifying an option
        // `-Wpedantic` holds (`-Wpointer-arith`, `-Wvariadic-macros`).
        (
            GCC,
            "c90",
            "c90_vla",
            &[
                "-pedantic",
                "-Werror",
                "-Wno-error=pedantic",
                "-Wno-error=vla",
            ],
            &["warning [-Wvla]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_vla",
            &[
                "-Wno-error=vla",
                "-pedantic",
                "-Werror",
                "-Wno-error=pedantic",
            ],
            &["warning [-Wvla]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_mixed",
            &[
                "-pedantic",
                "-Werror",
                "-Wno-error=pedantic",
                "-Wno-error=declaration-after-statement",
            ],
            &["warning [-Wdeclaration-after-statement]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_mixed",
            &[
                "-Wno-error=declaration-after-statement",
                "-pedantic",
                "-Werror",
                "-Wno-error=pedantic",
            ],
            &["warning [-Wdeclaration-after-statement]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-Werror=pedantic", "-Wno-error=variadic-macros"],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-Wno-error=variadic-macros", "-Werror=pedantic"],
            &[
                "error [-Werror=variadic-macros]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-Wvariadic-macros", "-Werror=pedantic"],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-Werror=pedantic", "-Wvariadic-macros"],
            &[
                "error [-Werror=variadic-macros]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c99",
            "named_va",
            &["-Werror=pedantic", "-Wno-error=variadic-macros"],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        (
            GCC,
            "c99",
            "named_va",
            &["-Wvariadic-macros", "-Werror=pedantic"],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_vla",
            &["-Werror=pedantic", "-Werror=vla"],
            &[
                "error [-Werror=vla]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_vla",
            &["-Werror=vla", "-Werror=pedantic"],
            &[
                "error [-Werror=vla]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_mixed",
            &["-Werror=pedantic", "-Werror=declaration-after-statement"],
            &[
                "error [-Werror=declaration-after-statement]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_mixed",
            &["-Werror=declaration-after-statement", "-Werror=pedantic"],
            &[
                "error [-Werror=declaration-after-statement]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_vla",
            &["-Werror=vla"],
            &[
                "error [-Werror=vla]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c90",
            "c90_vla",
            &["-Wno-vla", "-Werror=vla"],
            &[
                "error [-Werror=vla]",
                "cc1: some warnings being treated as errors",
            ],
            1,
        ),
        (GCC, "c90", "c90_vla", &["-Werror=vla", "-Wno-vla"], &[], 0),
        (
            GCC,
            "c90",
            "c90_longlong",
            &["-pedantic-errors", "-Wno-error=long-long"],
            &["warning [-Wlong-long]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_longlong",
            &["-Wlong-long"],
            &["warning [-Wlong-long]"],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-Werror=variadic-macros"],
            &[],
            0,
        ),
        (
            GCC,
            "c90",
            "c90_variadic",
            &["-Wno-error=variadic-macros", "-pedantic-errors"],
            &["error [-Wvariadic-macros]"],
            1,
        ),
        (
            GCC,
            "c99",
            "ptr_sizeof_void",
            &["-Wpointer-arith", "-Werror"],
            &[
                "error [-Werror=pointer-arith]",
                "error [-Werror=pointer-arith]",
                "cc1: all warnings being treated as errors",
            ],
            1,
        ),
        (
            GCC,
            "c99",
            "ptr_sizeof_void",
            &["-pedantic", "-Werror", "-Wno-error=pointer-arith"],
            &["warning [-Wpointer-arith]", "warning [-Wpointer-arith]"],
            0,
        ),
        (
            GCC,
            "c99",
            "ptr_sizeof_void",
            &["-Wpointer-arith", "-Werror=pedantic"],
            &["warning [-Wpointer-arith]", "warning [-Wpointer-arith]"],
            0,
        ),
        // Clang's `-Wvariadic-macros` holds the two variadic macro
        // extensions and `__VA_OPT__` outside a variadic macro, which it
        // warns of by default.
        (
            CLANG,
            "c90",
            "c90_variadic",
            &["-pedantic", "-Werror", "-Wno-variadic-macros"],
            &[],
            0,
        ),
        (
            CLANG,
            "c90",
            "c90_variadic",
            &["-Wvariadic-macros"],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        (
            CLANG,
            "c90",
            "c90_variadic",
            &["-pedantic-errors", "-Wno-error=variadic-macros"],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        (
            CLANG,
            "c99",
            "named_va",
            &["-Werror=variadic-macros"],
            &["error [-Werror,-Wvariadic-macros]"],
            1,
        ),
        (
            CLANG,
            "c23",
            "vaopt_misplaced",
            &[],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        (
            CLANG,
            "c23",
            "vaopt_misplaced",
            &["-Wno-variadic-macros"],
            &[],
            0,
        ),
        (
            CLANG,
            "c23",
            "vaopt_misplaced",
            &["-pedantic-errors"],
            &["error [-Werror,-Wvariadic-macros]"],
            1,
        ),
        (
            CLANG,
            "c23",
            "vaopt_misplaced",
            &["-Werror", "-Wno-error=variadic-macros"],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        (
            CLANG,
            "c23",
            "vaopt_misplaced",
            &["-Werror=pedantic"],
            &["warning [-Wvariadic-macros]"],
            0,
        ),
        // bcc's own command line applies `-Wvariadic-macros` without a word.
        (
            OWN,
            "c90",
            "c90_variadic",
            &["-pedantic", "-Werror", "-Wno-variadic-macros"],
            &[],
            0,
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

    /// [`sequence`], with the option each diagnostic's tag names.
    fn tagged_sequence(stderr: &str, source: &str, own: bool) -> Vec<String> {
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
                        Some(match line.rfind(" [-W").filter(|_| line.ends_with(']')) {
                            | Some(at) => format!("{severity}{}", &line[at..]),
                            | None => severity.to_owned(),
                        }),
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
    fn extension_diagnostics_reach_and_name_the_groups_their_compilers_file_them_under() {
        let directory: PathBuf =
            std::env::temp_dir().join(format!("bcc-extension-groups-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for (name, text) in TAGGED_SOURCES {
            std::fs::write(directory.join(format!("{name}.c")), text).unwrap();
        }
        let mut failures = Vec::new();
        for &(mode, standard, name, options, expected, status) in TAGGED_CASES {
            let caret = match mode {
                | Some("--gcc-mode") => Some("-fno-diagnostics-show-caret"),
                | Some(_) => Some("-fno-caret-diagnostics"),
                | None => None,
            };
            let source = format!("{name}.c");
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(mode)
                .arg(format!("-std={standard}"))
                .args(options)
                .arg("-fsyntax-only")
                .args(caret)
                .arg(&source)
                .current_dir(&directory)
                .env("NO_COLOR", "1")
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            let found = tagged_sequence(&stderr, &source, mode.is_none());
            if found != expected || output.status.code() != Some(status) {
                failures.push(format!(
                    "{} -std={standard} {options:?} {name}: expected {expected:?} and {status}, \
                     found {found:?} and {:?}:\n{stderr}",
                    mode.unwrap_or("bcc"),
                    output.status.code()
                ));
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// bcc's own command line says which options it applies only in part,
    /// under the rules of its identity: Clang's `-Wgnu`, of which it models
    /// an omitted variadic argument alone, and GCC's `-Wvla`, which also
    /// warns of variable length arrays in C99 and later. An option it does
    /// not apply under those rules is unknown, and one whose group it models
    /// whole, as `-Wvariadic-macros`, passes without a word.
    #[test]
    fn own_command_line_names_the_options_it_applies_only_in_part() {
        let directory: PathBuf =
            std::env::temp_dir().join(format!("bcc-partial-options-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("empty.c"), "int x;\n").unwrap();
        let options = ["-Wno-gnu", "-Wno-vla", "-Wno-variadic-macros"];
        for (identity, partial, unknown) in [
            (None, "-Wno-gnu", "-Wno-vla"),
            (Some("--compiler-identity=gcc"), "-Wno-vla", "-Wno-gnu"),
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(identity)
                .args(options)
                .args(["-std=c99", "-fsyntax-only", "empty.c"])
                .current_dir(&directory)
                .env("NO_COLOR", "1")
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains(&format!(
                    "warning: warning option '{partial}' is only partly implemented \
                     [-Wunknown-warning-option]"
                )) && stderr.contains(&format!(
                    "warning: unknown warning option '{unknown}' [-Wunknown-warning-option]"
                )) && !stderr.contains("-Wno-variadic-macros")
                    && output.status.success(),
                "{identity:?}: {stderr}"
            );
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
