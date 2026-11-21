//! The pragmas that can change a function's floating-point environment
//! cross the batch boundary by token ordinal, as `#pragma pack` does
//! (`pragma_pack.rs`). The preprocessor records each one it reads, under
//! every identity; semantic analysis decides which compiler honors which,
//! and computes an invalid operation at run time wherever one might make
//! the environment strict ([`crate::translation_phases::semantic_analysis`],
//! `floating_environment.rs`).
//! C99: §6.10.6 paragraph 1, p. 159; PDF p. 171 (implementation-defined
//! pragmas); §7.6.1 paragraph 2, p. 189; PDF p. 201 (`FENV_ACCESS`).

/// One pragma's effect on the floating-point environment of the code after
/// it. `float_control(except, on, push)` gives two: [`Self::Push`], then
/// the exception behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FloatingPragma {
    /// `#pragma STDC FENV_ACCESS` (`ON`; `OFF` and `DEFAULT` turn it off,
    /// Clang's default), or Microsoft's `#pragma fenv_access(on|off)`.
    /// C99: §7.6.1 paragraph 2, p. 189; PDF p. 201.
    FenvAccess(bool),
    /// Clang's exception behavior: `#pragma clang fp exceptions(...)` or
    /// `#pragma float_control(except, on|off)`. Clang extension: Clang
    /// Language Extensions, "Extensions to specify floating-point flags".
    Exceptions(ExceptionBehavior),
    /// `#pragma STDC FENV_ROUND`: whether it may set a constant direction
    /// other than to nearest, whose operations Clang computes at run time
    /// at `-O0`. `FE_TONEAREST` and `FE_DYNAMIC` do not; any other operand
    /// may, including a macro, which Clang replaces. C23: §7.6.2 paragraph
    /// 2, p. 213; PDF p. 226.
    DirectedRounding(bool),
    /// `#pragma float_control(push)`, alone or after another operand:
    /// saves the whole state, which [`Self::Pop`] restores.
    Push,
    /// `#pragma float_control(pop)`.
    Pop,
    /// One of these pragmas whose operands bcc does not read: they name a
    /// macro, which Clang replaces, or are not a form Clang reads.
    Unread,
    /// `#pragma GCC optimize`: GCC applies its options to the functions
    /// declared after it, which can turn trapping math back on. GNU
    /// extension: GCC manual, "Function Specific Option Pragmas".
    GccOptimize,
    /// `#pragma GCC push_options`.
    GccPushOptions,
    /// `#pragma GCC pop_options`.
    GccPopOptions,
    /// `#pragma GCC reset_options`: back to the command line's options.
    GccResetOptions,
}

/// Clang's floating-point exception behavior. `MayTrap` is computed at run
/// time at `-O0` and folded at `-O2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExceptionBehavior {
    Ignore,
    MayTrap,
    Strict,
}

/// One token of a pragma after `#pragma`, as [`read_floating_pragma`]
/// reads it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PragmaWord<'a> {
    pub(crate) spelling:   &'a str,
    /// The token is an identifier that names a macro, or one bcc does not
    /// look up (a universal character name).
    pub(crate) macro_name: bool,
}

/// What the pragma whose tokens after `#pragma`, white space left out, are
/// `words` does to the floating-point environment, in order. `complete` is
/// false when the pragma has more tokens than `words` holds. `ms_pragmas`
/// says whether Microsoft's pragmas are read (`-fms-extensions`, or `cl`).
/// Clang replaces macros in the operands of `clang fp`, `float_control`,
/// `fenv_access` and `STDC FENV_ROUND`, though not in the switch of
/// `STDC FENV_ACCESS`.
pub(crate) fn read_floating_pragma(
    words: &[PragmaWord<'_>],
    complete: bool,
    ms_pragmas: bool,
) -> [Option<FloatingPragma>; 2] {
    use FloatingPragma as P;
    let spelled = |index: usize| words.get(index).map(|word| word.spelling);
    let one = |pragma| [Some(pragma), None];
    let ours = match (spelled(0), spelled(1)) {
        | (Some("STDC"), Some("FENV_ACCESS")) => {
            return match spelled(2) {
                | Some("ON") => one(P::FenvAccess(true)),
                | Some("OFF" | "DEFAULT") => one(P::FenvAccess(false)),
                | _ => [None; 2],
            };
        },
        | (Some("STDC"), Some("FENV_ROUND")) => {
            return match words.get(2) {
                | Some(word)
                    if !word.macro_name
                        && matches!(word.spelling, "FE_TONEAREST" | "FE_DYNAMIC") =>
                    one(P::DirectedRounding(false)),
                | Some(_) => one(P::DirectedRounding(true)),
                | None => [None; 2],
            };
        },
        | (Some("GCC"), Some("optimize")) => return one(P::GccOptimize),
        | (Some("GCC"), Some("push_options")) => return one(P::GccPushOptions),
        | (Some("GCC"), Some("pop_options")) => return one(P::GccPopOptions),
        | (Some("GCC"), Some("reset_options")) => return one(P::GccResetOptions),
        | (Some("clang"), Some("fp")) => clang_fp(&words[2..]),
        | (Some("float_control"), _) => float_control(&words[1..]),
        | (Some("fenv_access"), _) if ms_pragmas => fenv_access(&words[1..]),
        | _ => return [None; 2],
    };
    if complete {
        ours.unwrap_or([Some(P::Unread), None])
    } else {
        [Some(P::Unread), None]
    }
}

/// The clauses of `#pragma clang fp`, each `name(argument)`: the last
/// `exceptions` clause sets the exception behavior, and the others do not
/// touch the environment. `None` for a form bcc does not read.
fn clang_fp(clauses: &[PragmaWord<'_>]) -> Option<[Option<FloatingPragma>; 2]> {
    if clauses.is_empty() || clauses.iter().any(|word| word.macro_name) {
        return None;
    }
    let mut behavior = None;
    for clause in clauses.chunks(4) {
        let &[name, open, argument, close] = clause else {
            return None;
        };
        if open.spelling != "(" || close.spelling != ")" {
            return None;
        }
        if name.spelling == "exceptions" {
            behavior = Some(match argument.spelling {
                | "ignore" => ExceptionBehavior::Ignore,
                | "maytrap" => ExceptionBehavior::MayTrap,
                | "strict" => ExceptionBehavior::Strict,
                | _ => return None,
            });
        }
    }
    Some([behavior.map(FloatingPragma::Exceptions), None])
}

/// `#pragma float_control(push)`, `(pop)`, or `(precise|except, on|off)`
/// with an optional `push` after it. `None` for a form bcc does not read.
fn float_control(operands: &[PragmaWord<'_>]) -> Option<[Option<FloatingPragma>; 2]> {
    use FloatingPragma as P;
    if operands.iter().any(|word| word.macro_name) {
        return None;
    }
    let spellings: [Option<&str>; 8] =
        std::array::from_fn(|index| operands.get(index).map(|word| word.spelling));
    let (kind, on, push) = match spellings {
        | [Some("("), Some("push"), Some(")"), None, ..] => return Some([Some(P::Push), None]),
        | [Some("("), Some("pop"), Some(")"), None, ..] => return Some([Some(P::Pop), None]),
        | [
            Some("("),
            Some(kind),
            Some(","),
            Some(on),
            Some(")"),
            None,
            ..,
        ] => (kind, on, false),
        | [
            Some("("),
            Some(kind),
            Some(","),
            Some(on),
            Some(","),
            Some("push"),
            Some(")"),
            None,
        ] => (kind, on, true),
        | _ => return None,
    };
    let on = match on {
        | "on" => true,
        | "off" => false,
        | _ => return None,
    };
    let setting = match kind {
        | "precise" => None,
        | "except" => Some(P::Exceptions(if on {
            ExceptionBehavior::Strict
        } else {
            ExceptionBehavior::Ignore
        })),
        | _ => return None,
    };
    Some(if push {
        [Some(P::Push), setting]
    } else {
        [setting, None]
    })
}

/// Microsoft's `#pragma fenv_access(on|off)`. `None` for a form bcc does
/// not read. MSVC extension: Microsoft Learn, "fenv_access pragma".
fn fenv_access(operands: &[PragmaWord<'_>]) -> Option<[Option<FloatingPragma>; 2]> {
    match operands {
        | [open, switch, close]
            if open.spelling == "(" && close.spelling == ")" && !switch.macro_name =>
            match switch.spelling {
                | "on" => Some([Some(FloatingPragma::FenvAccess(true)), None]),
                | "off" => Some([Some(FloatingPragma::FenvAccess(false)), None]),
                | _ => None,
            },
        | _ => None,
    }
}
