//! Checked C library calls: the object-size-checking builtins that glibc's
//! fortified headers (`bits/string_fortified.h`, `bits/strings_fortified.h`,
//! `bits/stdio2.h`) call, each an alias of the libc `__*_chk` entry point.
//! Each signature is the libc declaration's (`bits/stdio2-decl.h` and the
//! LSB): the written C operands in order, then the destination's object
//! size; the formatting calls take a `flag` and the size before the format.
//! The caller computes every size; the callee checks it and terminates the
//! process instead of overrunning the destination. These are reserved
//! spellings only: an undeclared `__vsprintf_chk` stays an ordinary name.
//!
//! GNU extension: GCC "Object Size Checking"
//! <https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html>; the
//! `__*_chk` interfaces: Linux Standard Base Core, "Interfaces for libc".
//! C99: §7.21.2-§7.21.3, pp. 325-327; PDF pp. 337-339 (copying and
//! concatenation); §7.19.6.5-§7.19.6.13, pp. 290-294; PDF pp. 302-306
//! (formatted output); §7.15, p. 249; PDF p. 261 (`va_list` operands).
use super::{
    ArgumentRule as A,
    Availability,
    BuiltinDescriptor,
    BuiltinEffectClass,
    BuiltinId as Id,
    BuiltinOperation,
    ConversionRule as C,
    DesignatorPolicy,
    NamingPolicy,
    Signature,
    SignatureType as T,
    SyntaxForm,
    TypeRule,
};
use crate::target::Scalar as S;

const VOID: T = T::Pointer(S::Void, false);
const CONST_VOID: T = T::Pointer(S::Void, true);
const CHAR: T = T::Pointer(S::Char, false);
const CONST_CHAR: T = T::Pointer(S::Char, true);
const INT: T = T::Scalar(S::Int);

pub(super) const BUILTINS: &[BuiltinDescriptor] = &[
    descriptor(
        Id::CheckedMemcpy,
        "__builtin___memcpy_chk",
        "__memcpy_chk",
        VOID,
        &[VOID, CONST_VOID, T::Size, T::Size],
        &[
            value(VOID),
            value(CONST_VOID),
            value(T::Size),
            value(T::Size),
        ],
        false,
    ),
    descriptor(
        Id::CheckedMemmove,
        "__builtin___memmove_chk",
        "__memmove_chk",
        VOID,
        &[VOID, CONST_VOID, T::Size, T::Size],
        &[
            value(VOID),
            value(CONST_VOID),
            value(T::Size),
            value(T::Size),
        ],
        false,
    ),
    descriptor(
        Id::CheckedMempcpy,
        "__builtin___mempcpy_chk",
        "__mempcpy_chk",
        VOID,
        &[VOID, CONST_VOID, T::Size, T::Size],
        &[
            value(VOID),
            value(CONST_VOID),
            value(T::Size),
            value(T::Size),
        ],
        false,
    ),
    descriptor(
        Id::CheckedMemset,
        "__builtin___memset_chk",
        "__memset_chk",
        VOID,
        &[VOID, INT, T::Size, T::Size],
        &[value(VOID), value(INT), value(T::Size), value(T::Size)],
        false,
    ),
    descriptor(
        Id::CheckedStrcpy,
        "__builtin___strcpy_chk",
        "__strcpy_chk",
        CHAR,
        &[CHAR, CONST_CHAR, T::Size],
        &[value(CHAR), value(CONST_CHAR), value(T::Size)],
        false,
    ),
    descriptor(
        Id::CheckedStpcpy,
        "__builtin___stpcpy_chk",
        "__stpcpy_chk",
        CHAR,
        &[CHAR, CONST_CHAR, T::Size],
        &[value(CHAR), value(CONST_CHAR), value(T::Size)],
        false,
    ),
    descriptor(
        Id::CheckedStrncpy,
        "__builtin___strncpy_chk",
        "__strncpy_chk",
        CHAR,
        &[CHAR, CONST_CHAR, T::Size, T::Size],
        &[
            value(CHAR),
            value(CONST_CHAR),
            value(T::Size),
            value(T::Size),
        ],
        false,
    ),
    descriptor(
        Id::CheckedStpncpy,
        "__builtin___stpncpy_chk",
        "__stpncpy_chk",
        CHAR,
        &[CHAR, CONST_CHAR, T::Size, T::Size],
        &[
            value(CHAR),
            value(CONST_CHAR),
            value(T::Size),
            value(T::Size),
        ],
        false,
    ),
    descriptor(
        Id::CheckedStrcat,
        "__builtin___strcat_chk",
        "__strcat_chk",
        CHAR,
        &[CHAR, CONST_CHAR, T::Size],
        &[value(CHAR), value(CONST_CHAR), value(T::Size)],
        false,
    ),
    descriptor(
        Id::CheckedStrncat,
        "__builtin___strncat_chk",
        "__strncat_chk",
        CHAR,
        &[CHAR, CONST_CHAR, T::Size, T::Size],
        &[
            value(CHAR),
            value(CONST_CHAR),
            value(T::Size),
            value(T::Size),
        ],
        false,
    ),
    descriptor(
        Id::CheckedSprintf,
        "__builtin___sprintf_chk",
        "__sprintf_chk",
        INT,
        &[CHAR, INT, T::Size, CONST_CHAR],
        &[value(CHAR), value(INT), value(T::Size), value(CONST_CHAR)],
        true,
    ),
    descriptor(
        Id::CheckedSnprintf,
        "__builtin___snprintf_chk",
        "__snprintf_chk",
        INT,
        &[CHAR, T::Size, INT, T::Size, CONST_CHAR],
        &[
            value(CHAR),
            value(T::Size),
            value(INT),
            value(T::Size),
            value(CONST_CHAR),
        ],
        true,
    ),
    descriptor(
        Id::CheckedVsprintf,
        "__builtin___vsprintf_chk",
        "__vsprintf_chk",
        INT,
        &[CHAR, INT, T::Size, CONST_CHAR, T::VaList],
        &[
            value(CHAR),
            value(INT),
            value(T::Size),
            value(CONST_CHAR),
            value(T::VaList),
        ],
        false,
    ),
    descriptor(
        Id::CheckedVsnprintf,
        "__builtin___vsnprintf_chk",
        "__vsnprintf_chk",
        INT,
        &[CHAR, T::Size, INT, T::Size, CONST_CHAR, T::VaList],
        &[
            value(CHAR),
            value(T::Size),
            value(INT),
            value(T::Size),
            value(CONST_CHAR),
            value(T::VaList),
        ],
        false,
    ),
    descriptor(
        Id::CheckedVprintf,
        "__builtin___vprintf_chk",
        "__vprintf_chk",
        INT,
        &[INT, CONST_CHAR, T::VaList],
        &[value(INT), value(CONST_CHAR), value(T::VaList)],
        false,
    ),
    descriptor(
        Id::CheckedVfprintf,
        "__builtin___vfprintf_chk",
        "__vfprintf_chk",
        INT,
        &[T::File, INT, CONST_CHAR, T::VaList],
        &[
            value(T::File),
            value(INT),
            value(CONST_CHAR),
            value(T::VaList),
        ],
        false,
    ),
];

/// A library alias whose every written operand converts to its parameter,
/// as a prototyped call's operands do (C99 §6.5.2.2p7, p. 72; PDF p. 84).
const fn descriptor(
    id: Id,
    spelling: &'static str,
    library: &'static str,
    result: T,
    parameters: &'static [T],
    arguments: &'static [A],
    variadic: bool,
) -> BuiltinDescriptor {
    let signature = Signature {
        result,
        parameters,
        prototype: true,
        variadic,
    };
    BuiltinDescriptor {
        id,
        spelling,
        library: Some(library),
        signature,
        arguments,
        operation: BuiltinOperation::Library,
        type_rule: TypeRule::Fixed,
        syntax: SyntaxForm::OrdinaryCall,
        designator: DesignatorPolicy::AddressableAlias,
        arity: (
            parameters.len(),
            if variadic {
                usize::MAX
            } else {
                parameters.len()
            },
        ),
        effects: BuiltinEffectClass::Library,
        availability: Availability {
            feature: None,
            naming:  NamingPolicy::Reserved,
        },
    }
}

/// A written operand converted to its parameter type and evaluated once.
const fn value(parameter: T) -> A {
    A::value(C::Parameter(parameter))
}
