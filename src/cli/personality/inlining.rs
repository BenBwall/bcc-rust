//! How Clang's driver decides from its inlining options whether a
//! compilation inlines, which bcc's `-fno-inline` stands for, so that
//! `__NO_INLINE__` is defined as Clang defines it. GCC's `-finline` and
//! `-fno-inline` are bcc's own, the last one deciding; Clang's driver reads
//! the `-f[no-]inline-functions` group as well.

/// The last of Clang's `-finline` and `-fno-inline`, and the last of its
/// `-finline-functions`, `-finline-hint-functions` and
/// `-fno-inline-functions`, each recorded as whether it is the `no-` one.
/// clang-cl's `/Ob0`, `/Ob1` and `/Ob2` (or `/Ob3`) are `-fno-inline`,
/// `-finline-hint-functions` and `-finline-functions` where they stand
/// (`TranslateOptArg` in Clang's `MSVC.cpp`).
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct ClangInlining {
    no_inline:           Option<bool>,
    no_inline_functions: Option<bool>,
}

impl ClangInlining {
    /// Records `option` if it is one of Clang's inlining options, and
    /// returns whether it is.
    pub(super) fn read(&mut self, option: &str) -> bool {
        match option {
            | "-finline" | "-fno-inline" => self.no_inline = Some(option == "-fno-inline"),
            | "-finline-functions" | "-finline-hint-functions" | "-fno-inline-functions" =>
                self.no_inline_functions = Some(option == "-fno-inline-functions"),
            | _ => return false,
        }
        true
    }

    /// bcc's option for what the options read ask, if any: `-fno-inline`
    /// when Clang inlines only `always_inline` functions. Clang's driver
    /// passes on the last of `-finline` and `-fno-inline` if there is one,
    /// the `-fno-inline` alone, and otherwise the last of the
    /// `-f[no-]inline-functions` group (`Clang::ConstructJob`); its front
    /// end inlines only `always_inline` functions after `-fno-inline` or
    /// `-fno-inline-functions` (`CompilerInvocation::ParseCodeGenArgs`), and
    /// then defines `__NO_INLINE__` (`InitializePredefinedMacros`), as it
    /// does with no optimization.
    pub(super) fn option(self) -> Option<&'static str> {
        self.no_inline
            .or(self.no_inline_functions)
            .unwrap_or(false)
            .then_some("-fno-inline")
    }
}
