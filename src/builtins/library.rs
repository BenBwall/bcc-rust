//! One signature set for reserved aliases and hosted ordinary implicit names.
//! GNU extension: <https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html>.
//! C99: §7.1.4, pp. 166-167; PDF pp. 178-179;
//! §5.1.2.1p1, p. 11; PDF p. 23 (freestanding ordinary names).
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

pub(super) const BUILTINS: &[BuiltinDescriptor] = &[
    descriptor(
        Id::LibraryAbort,
        "__builtin_abort",
        "abort",
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[],
            prototype:  true,
            variadic:   false,
        },
        &[],
    ),
    descriptor(
        Id::LibraryAbs,
        "__builtin_abs",
        "abs",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    descriptor(
        Id::LibraryCalloc,
        "__builtin_calloc",
        "calloc",
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Size, T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Size)),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryExit,
        "__builtin_exit",
        "exit",
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    descriptor(
        Id::LibraryFabs,
        "__builtin_fabs",
        "fabs",
        Signature {
            result:     T::Scalar(S::Double),
            parameters: &[T::Scalar(S::Double)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Double)))],
    ),
    descriptor(
        Id::LibraryFabsf,
        "__builtin_fabsf",
        "fabsf",
        Signature {
            result:     T::Scalar(S::Float),
            parameters: &[T::Scalar(S::Float)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Float)))],
    ),
    descriptor(
        Id::LibraryFabsl,
        "__builtin_fabsl",
        "fabsl",
        Signature {
            result:     T::Scalar(S::LongDouble),
            parameters: &[T::Scalar(S::LongDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::LongDouble)))],
    ),
    descriptor(
        Id::LibraryFloor,
        "__builtin_floor",
        "floor",
        Signature {
            result:     T::Scalar(S::Double),
            parameters: &[T::Scalar(S::Double)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Double)))],
    ),
    descriptor(
        Id::LibraryFree,
        "__builtin_free",
        "free",
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[T::Pointer(S::Void, false)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Pointer(S::Void, false)))],
    ),
    descriptor(
        Id::LibraryMalloc,
        "__builtin_malloc",
        "malloc",
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Size))],
    ),
    descriptor(
        Id::LibraryMemchr,
        "__builtin_memchr",
        "memchr",
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Pointer(S::Void, true), T::Scalar(S::Int), T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, true))),
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryMemcmp,
        "__builtin_memcmp",
        "memcmp",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[
                T::Pointer(S::Void, true),
                T::Pointer(S::Void, true),
                T::Size,
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, true))),
            A::value(C::Parameter(T::Pointer(S::Void, true))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryMemcpy,
        "__builtin_memcpy",
        "memcpy",
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[
                T::Pointer(S::Void, false),
                T::Pointer(S::Void, true),
                T::Size,
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, false))),
            A::value(C::Parameter(T::Pointer(S::Void, true))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryMemmove,
        "__builtin_memmove",
        "memmove",
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[
                T::Pointer(S::Void, false),
                T::Pointer(S::Void, true),
                T::Size,
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, false))),
            A::value(C::Parameter(T::Pointer(S::Void, true))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryMemset,
        "__builtin_memset",
        "memset",
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Pointer(S::Void, false), T::Scalar(S::Int), T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, false))),
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryPrintf,
        "__builtin_printf",
        "printf",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Pointer(S::Char, true)],
            prototype:  true,
            variadic:   true,
        },
        &[A::value(C::Parameter(T::Pointer(S::Char, true)))],
    ),
    descriptor(
        Id::LibraryPutchar,
        "__builtin_putchar",
        "putchar",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    descriptor(
        Id::LibraryPuts,
        "__builtin_puts",
        "puts",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Pointer(S::Char, true)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Pointer(S::Char, true)))],
    ),
    descriptor(
        Id::LibrarySnprintf,
        "__builtin_snprintf",
        "snprintf",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[
                T::Pointer(S::Char, false),
                T::Size,
                T::Pointer(S::Char, true),
            ],
            prototype:  true,
            variadic:   true,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, false))),
            A::value(C::Parameter(T::Size)),
            A::value(C::Parameter(T::Pointer(S::Char, true))),
        ],
    ),
    descriptor(
        Id::LibrarySprintf,
        "__builtin_sprintf",
        "sprintf",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Pointer(S::Char, false), T::Pointer(S::Char, true)],
            prototype:  true,
            variadic:   true,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, false))),
            A::value(C::Parameter(T::Pointer(S::Char, true))),
        ],
    ),
    descriptor(
        Id::LibraryStrchr,
        "__builtin_strchr",
        "strchr",
        Signature {
            result:     T::Pointer(S::Char, false),
            parameters: &[T::Pointer(S::Char, true), T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, true))),
            A::value(C::Parameter(T::Scalar(S::Int))),
        ],
    ),
    descriptor(
        Id::LibraryStrcmp,
        "__builtin_strcmp",
        "strcmp",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Pointer(S::Char, true), T::Pointer(S::Char, true)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, true))),
            A::value(C::Parameter(T::Pointer(S::Char, true))),
        ],
    ),
    descriptor(
        Id::LibraryStrcpy,
        "__builtin_strcpy",
        "strcpy",
        Signature {
            result:     T::Pointer(S::Char, false),
            parameters: &[T::Pointer(S::Char, false), T::Pointer(S::Char, true)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, false))),
            A::value(C::Parameter(T::Pointer(S::Char, true))),
        ],
    ),
    descriptor(
        Id::LibraryStrlen,
        "__builtin_strlen",
        "strlen",
        Signature {
            result:     T::Size,
            parameters: &[T::Pointer(S::Char, true)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Pointer(S::Char, true)))],
    ),
    descriptor(
        Id::LibraryStrncmp,
        "__builtin_strncmp",
        "strncmp",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[
                T::Pointer(S::Char, true),
                T::Pointer(S::Char, true),
                T::Size,
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, true))),
            A::value(C::Parameter(T::Pointer(S::Char, true))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryStrncpy,
        "__builtin_strncpy",
        "strncpy",
        Signature {
            result:     T::Pointer(S::Char, false),
            parameters: &[
                T::Pointer(S::Char, false),
                T::Pointer(S::Char, true),
                T::Size,
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, false))),
            A::value(C::Parameter(T::Pointer(S::Char, true))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryStrrchr,
        "__builtin_strrchr",
        "strrchr",
        Signature {
            result:     T::Pointer(S::Char, false),
            parameters: &[T::Pointer(S::Char, true), T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, true))),
            A::value(C::Parameter(T::Scalar(S::Int))),
        ],
    ),
];

pub(crate) fn ordinary(name: &str) -> Option<&'static BuiltinDescriptor> {
    BUILTINS
        .binary_search_by_key(&name, |d| d.library.expect("library shard"))
        .ok()
        .map(|index| &BUILTINS[index])
        .or_else(|| super::math::ordinary(name))
}

const fn descriptor(
    id: Id,
    spelling: &'static str,
    library: &'static str,
    signature: Signature,
    arguments: &'static [A],
) -> BuiltinDescriptor {
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
            signature.parameters.len(),
            if signature.variadic {
                usize::MAX
            } else {
                signature.parameters.len()
            },
        ),
        effects: BuiltinEffectClass::Library,
        availability: Availability {
            feature: None,
            naming:  NamingPolicy::Reserved,
        },
    }
}
