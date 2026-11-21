//! Whether a translation unit holds a pragma that can change a function's
//! floating-point environment. The preprocessor notes each one it reads,
//! under every identity, wherever it is written; semantic analysis then
//! computes every function's invalid operations at run time under the
//! rules of a compiler that honors one
//! ([`crate::translation_phases::semantic_analysis`],
//! `floating_environment.rs`).
//! C99: §6.10.6 paragraph 1, p. 159; PDF p. 171 (implementation-defined
//! pragmas); §7.6.1 paragraph 2, p. 189; PDF p. 201 (`FENV_ACCESS`).

/// The floating-point pragmas a translation unit holds anywhere: in a
/// function or outside one, from `#pragma`, `_Pragma` or `__pragma`, in the
/// source file or a header.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FloatingPragmas {
    /// A pragma Clang or `cl` reads for the floating-point environment:
    /// `STDC FENV_ACCESS` or `STDC FENV_ROUND` with any operand,
    /// `float_control` in any form, Microsoft's `fenv_access` (whether or
    /// not Microsoft's pragmas are on), or a `clang fp` pragma that may set
    /// `exceptions`. C99: §7.6.1 paragraph 2, p. 189; PDF p. 201. C23:
    /// §7.6.2 paragraph 2, p. 213; PDF p. 226. Clang extension: Clang
    /// Language Extensions, "Extensions to specify floating-point flags".
    pub(crate) environment: bool,
    /// `#pragma GCC optimize` or `#pragma GCC push_options`, which can turn
    /// trapping math back on for the functions after it. GNU extension:
    /// GCC manual, "Function Specific Option Pragmas".
    pub(crate) gcc_options: bool,
}

impl FloatingPragmas {
    /// Notes the pragma whose tokens after `#pragma`, white space left out,
    /// are `words`.
    pub(crate) fn note<'a>(&mut self, mut words: impl Iterator<Item = PragmaWord<'a>>) {
        let Some(first) = words.next() else {
            return;
        };
        let second = words.next().map(|word| word.spelling);
        match (first.spelling, second) {
            | ("STDC", Some("FENV_ACCESS" | "FENV_ROUND"))
            | ("float_control" | "fenv_access", _) => self.environment = true,
            // Clang replaces macros in the clauses, so one that names a
            // macro may spell `exceptions`.
            | ("clang", Some("fp")) =>
                if words.any(|word| word.macro_name || word.spelling == "exceptions") {
                    self.environment = true;
                },
            | ("GCC", Some("optimize" | "push_options")) => self.gcc_options = true,
            | _ => {},
        }
    }
}

/// One token of a pragma after `#pragma`, as [`FloatingPragmas::note`]
/// reads it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PragmaWord<'a> {
    pub(crate) spelling:   &'a str,
    /// The token is an identifier that names a macro, or one bcc does not
    /// look up (a universal character name).
    pub(crate) macro_name: bool,
}
