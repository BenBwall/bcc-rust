//! The external-symbol contract: library functions whose calls the compiler
//! must treat specially because of how they transfer control, independent
//! of any builtin spelling. This is the one classifier of these names.
//! Semantic analysis projects it onto linked C functions as a retained
//! returns-twice fact, the textual IR parser onto unmarked external
//! declarations, and the interpreter dispatches the environment operations
//! it implements from it; no other phase matches these spellings.
//!
//! The returns-twice spellings are GCC's `special_function_p` set (`setjmp`,
//! `sigsetjmp`, `savectx`, `vfork`, `getcontext`, with the `_` and `__`
//! prefixes C libraries add) and the MinGW-w64 and MSVC `<setjmp.h>`
//! entries. A function declared with GNU `returns_twice` is the same
//! contract for an unlisted name; that fact stays with semantic analysis.
//! C99: §7.13.1.1 paragraph 2, p. 243; PDF p. 255 (saving an environment);
//! §7.13.2.1 paragraph 2, p. 244; PDF p. 256 (restoring it).
//! GNU extension: GCC manual, "Common Function Attributes", `returns_twice`.

/// What a call to a cataloged external symbol does to control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalContract {
    /// Saves the calling environment in the `jmp_buf` its first argument
    /// points to and returns zero; a later restore returns from it again
    /// with a nonzero value. The `setjmp` family.
    /// C99: §7.13.1.1 paragraphs 2-3, p. 243; PDF p. 255.
    SaveEnvironment,
    /// Returns twice by some other mechanism (`vfork`, `getcontext`,
    /// `savectx`): code generation must treat it like `setjmp`, but no
    /// interpreter model exists for it.
    ReturnsTwice,
    /// Restores the environment its first argument holds and makes the
    /// saving call return its second argument, or 1 for 0: the `longjmp`
    /// family, which never returns normally.
    /// C99: §7.13.2.1 paragraphs 2-4, p. 244; PDF p. 256.
    RestoreEnvironment,
    /// MinGW-w64's `mingw_getsp`: the caller's stack pointer, which its
    /// `setjmp` macro passes to `_setjmp` when the compiler is not Clang.
    StackPointer,
}

impl ExternalContract {
    /// Whether a call can return more than once, which forbids keeping the
    /// caller's frame state anywhere a second return cannot find it.
    pub(crate) const fn returns_twice(self) -> bool {
        matches!(self, Self::SaveEnvironment | Self::ReturnsTwice)
    }
}

/// The contract of the external symbol `name`, if the catalog lists it.
pub(crate) fn external_contract(name: &str) -> Option<ExternalContract> {
    use ExternalContract as E;
    Some(match name {
        | "setjmp"
        | "_setjmp"
        | "__setjmp"
        | "sigsetjmp"
        | "__sigsetjmp"
        | "_setjmpex"
        | "_setjmp3"
        | "__mingw_setjmp"
        | "__intrinsic_setjmp"
        | "__intrinsic_setjmpex" => E::SaveEnvironment,
        | "vfork" | "__vfork" | "getcontext" | "__getcontext" | "savectx" | "__savectx" =>
            E::ReturnsTwice,
        | "longjmp" | "_longjmp" | "siglongjmp" | "__longjmp_chk" | "__mingw_longjmp" =>
            E::RestoreEnvironment,
        | "mingw_getsp" => E::StackPointer,
        | _ => return None,
    })
}

/// Whether the external symbol `name` may return twice.
pub(crate) fn returns_twice(name: &str) -> bool {
    external_contract(name).is_some_and(ExternalContract::returns_twice)
}

/// Canonical C signatures for cataloged external library names. Reserved
/// builtin aliases, implicit ordinary declarations and host validation all
/// read the same descriptor, including fortified libc entry points.
/// C99: §7.1.4, pp. 166-167; PDF pp. 178-179.
pub(crate) fn external_signature(name: &str) -> Option<super::Signature> {
    let scalar = match name {
        | "copysign" => Some(super::BuiltinId::Copysign),
        | "copysignf" => Some(super::BuiltinId::Copysignf),
        | "copysignl" => Some(super::BuiltinId::Copysignl),
        | _ => None,
    };
    scalar
        .map(super::BuiltinId::descriptor)
        .or_else(|| super::ordinary(name))
        .or_else(|| {
            super::checked::BUILTINS
                .iter()
                .find(|d| d.library == Some(name))
        })
        .map(|descriptor| descriptor.signature)
        .or_else(|| {
            (name == "isprint").then_some(super::Signature {
                result:     super::SignatureType::Scalar(crate::target::Scalar::Int),
                parameters: &[super::SignatureType::Scalar(crate::target::Scalar::Int)],
                prototype:  true,
                variadic:   false,
            })
        })
}
