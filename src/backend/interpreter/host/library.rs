//! The C signatures of the library functions the host provides beyond the
//! builtin catalog (`crate::builtins::external_signature`), so that a call
//! through an incompatible declaration traps before the function runs, as
//! the catalog's functions do.
//!
//! C99: §6.5.2.2 paragraph 9, p. 72; PDF p. 84; the signatures are those of
//! §7.4, §7.19, §7.20 and §7.21; `__ctype_b_loc` and the `__isoc99_` and
//! `__*_chk` names are glibc's, `__acrt_iob_func` the Universal CRT's.

use super::{
    HostCall,
    Type,
    UbKind,
};
use crate::backend::interpreter::trap::Fault;

/// A parameter or result type, projected per target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum P {
    Void,
    /// `int` or `unsigned int`.
    Int,
    /// `long` or `unsigned long`: the target's width.
    Long,
    /// `long long`, `size_t` and the like: 64 bits on every target.
    Wide64,
    Ptr,
    /// `wchar_t`: 32 bits on GNU/Linux, 16 on Windows.
    WChar,
    /// `_Float128`.
    Quad,
}

/// A function's result, fixed parameters and whether it is variadic.
#[derive(Clone, Copy, Debug)]
pub(super) struct Signature {
    result:     P,
    parameters: &'static [P],
    variadic:   bool,
}

const fn fixed(result: P, parameters: &'static [P]) -> Signature {
    Signature {
        result,
        parameters,
        variadic: false,
    }
}

const fn variadic(result: P, parameters: &'static [P]) -> Signature {
    Signature {
        result,
        parameters,
        variadic: true,
    }
}

/// The signature of a host library function outside the builtin catalog.
#[expect(
    clippy::match_same_arms,
    reason = "Functions are grouped by family, not by signature."
)]
pub(super) fn signature(name: &str) -> Option<Signature> {
    use P::{
        Int,
        Long,
        Ptr,
        Quad,
        Void,
        WChar,
        Wide64,
    };
    Some(match name {
        | "fopen" => fixed(Ptr, &[Ptr, Ptr]),
        | "freopen" => fixed(Ptr, &[Ptr, Ptr, Ptr]),
        | "fclose" | "fflush" | "fgetc" | "getc" | "feof" | "ferror" | "remove" =>
            fixed(Int, &[Ptr]),
        | "fputc" | "putc" | "ungetc" => fixed(Int, &[Int, Ptr]),
        | "fputs" => fixed(Int, &[Ptr, Ptr]),
        | "fwrite" | "fread" => fixed(Wide64, &[Ptr, Wide64, Wide64, Ptr]),
        | "getchar" => fixed(Int, &[]),
        | "fgets" => fixed(Ptr, &[Ptr, Int, Ptr]),
        | "clearerr" | "rewind" => fixed(Void, &[Ptr]),
        | "fseek" => fixed(Int, &[Ptr, Long, Int]),
        | "ftell" => fixed(Long, &[Ptr]),
        | "tmpnam" => fixed(Ptr, &[Ptr]),
        | "fprintf" | "sscanf" | "fscanf" | "__isoc99_sscanf" | "__isoc99_fscanf" =>
            variadic(Int, &[Ptr, Ptr]),
        | "scanf" | "__isoc99_scanf" => variadic(Int, &[Ptr]),
        | "vfprintf" | "vsprintf" => fixed(Int, &[Ptr, Ptr, Ptr]),
        | "vprintf" => fixed(Int, &[Ptr, Ptr]),
        | "vsnprintf" => fixed(Int, &[Ptr, Wide64, Ptr, Ptr]),
        | "__printf_chk" => variadic(Int, &[Int, Ptr]),
        | "__fprintf_chk" => variadic(Int, &[Ptr, Int, Ptr]),
        | "__vfprintf_chk" => fixed(Int, &[Ptr, Int, Ptr, Ptr]),
        | "__acrt_iob_func" => fixed(Ptr, &[Int]),
        | "strcat" | "strstr" | "strpbrk" | "stpcpy" | "strtok" => fixed(Ptr, &[Ptr, Ptr]),
        | "strncat" => fixed(Ptr, &[Ptr, Ptr, Wide64]),
        | "strndup" => fixed(Ptr, &[Ptr, Wide64]),
        | "strnlen" => fixed(Wide64, &[Ptr, Wide64]),
        | "strspn" | "strcspn" => fixed(Wide64, &[Ptr, Ptr]),
        | "strdup" => fixed(Ptr, &[Ptr]),
        | "memrchr" => fixed(Ptr, &[Ptr, Int, Wide64]),
        | "atoi" => fixed(Int, &[Ptr]),
        | "atol" => fixed(Long, &[Ptr]),
        | "atoll" => fixed(Wide64, &[Ptr]),
        | "strtol" | "strtoul" => fixed(Long, &[Ptr, Ptr, Int]),
        | "strtoll" | "strtoull" => fixed(Wide64, &[Ptr, Ptr, Int]),
        | "isalnum" | "isalpha" | "isblank" | "iscntrl" | "isdigit" | "isgraph" | "islower"
        | "ispunct" | "isspace" | "isupper" | "isxdigit" | "tolower" | "toupper" =>
            fixed(Int, &[Int]),
        | "__ctype_b_loc" | "__ctype_tolower_loc" | "__ctype_toupper_loc" => fixed(Ptr, &[]),
        | "wcslen" => fixed(Wide64, &[Ptr]),
        | "wcscpy" | "wcscat" => fixed(Ptr, &[Ptr, Ptr]),
        | "wcsncpy" | "wmemcpy" | "wmemmove" => fixed(Ptr, &[Ptr, Ptr, Wide64]),
        | "wcscmp" => fixed(Int, &[Ptr, Ptr]),
        | "wcsncmp" | "wmemcmp" => fixed(Int, &[Ptr, Ptr, Wide64]),
        | "wcschr" | "wcsrchr" => fixed(Ptr, &[Ptr, WChar]),
        | "wmemset" | "wmemchr" => fixed(Ptr, &[Ptr, WChar, Wide64]),
        | "qsort" => fixed(Void, &[Ptr, Wide64, Wide64, Ptr]),
        | "strtof128" => fixed(Quad, &[Ptr, Ptr]),
        | "strfromf128" => fixed(Int, &[Ptr, Wide64, Ptr, Quad]),
        | "fabsf128" | "sqrtf128" | "ceilf128" | "floorf128" | "truncf128" | "roundf128"
        | "rintf128" | "nearbyintf128" => fixed(Quad, &[Quad]),
        | "copysignf128" | "fmodf128" | "fmaxf128" | "fminf128" | "fdimf128" =>
            fixed(Quad, &[Quad, Quad]),
        | "ldexpf128" | "scalbnf128" => fixed(Quad, &[Quad, Int]),
        | "frexpf128" => fixed(Quad, &[Quad, Ptr]),
        | "nanf128" => fixed(Quad, &[Ptr]),
        | "bsearch" => fixed(Ptr, &[Ptr, Ptr, Wide64, Wide64, Ptr]),
        | _ => return None,
    })
}

/// Rejects a call whose operand types do not match `signature`, after
/// default argument promotions, before the function performs any effects.
pub(super) fn check(
    call: HostCall<'_>,
    signature: Signature,
    long_width: u32,
    windows: bool,
) -> Result<(), Fault> {
    #[expect(
        clippy::match_same_arms,
        reason = "Each pattern projects separately; equal widths are a coincidence."
    )]
    let project = |pattern: P| -> Option<Type> {
        Some(match pattern {
            | P::Void => return None,
            | P::Int => Type::I32,
            | P::Long if long_width == 32 => Type::I32,
            | P::Long | P::Wide64 => Type::I64,
            | P::Ptr => Type::Ptr,
            | P::WChar if windows => Type::I16,
            | P::WChar => Type::I32,
            | P::Quad => Type::F128,
        })
    };
    let fixed = signature.parameters.len();
    if call.result != project(signature.result)
        || call.arg_types.len() < fixed
        || (!signature.variadic && call.arg_types.len() != fixed)
        || call
            .arg_types
            .iter()
            .zip(signature.parameters)
            .any(|(&actual, &expected)| Some(actual) != project(expected))
    {
        return Err(UbKind::SignatureMismatch.into());
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn every_added_host_function_has_a_signature() {
    for &name in super::stdio::FUNCTIONS
        .iter()
        .chain(super::strings::FUNCTIONS)
        .chain(super::quad::FUNCTIONS)
        .chain(&crate::backend::interpreter::callbacks::FUNCTIONS)
        .chain(&[
            "sscanf",
            "fscanf",
            "scanf",
            "__isoc99_sscanf",
            "__isoc99_fscanf",
            "__isoc99_scanf",
        ])
    {
        assert!(signature(name).is_some(), "{name}");
    }
}
