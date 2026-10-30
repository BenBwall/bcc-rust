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
use crate::{
    configuration::Feature,
    target::Scalar as S,
};

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
    gcc_descriptor(
        Id::LibraryAlignedAlloc,
        "__builtin_aligned_alloc",
        "aligned_alloc",
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
        Id::LibraryBcmp,
        "__builtin_bcmp",
        "bcmp",
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
        Id::LibraryBcopy,
        "__builtin_bcopy",
        "bcopy",
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[
                T::Pointer(S::Void, true),
                T::Pointer(S::Void, false),
                T::Size,
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, true))),
            A::value(C::Parameter(T::Pointer(S::Void, false))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryBzero,
        "__builtin_bzero",
        "bzero",
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[T::Pointer(S::Void, false), T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, false))),
            A::value(C::Parameter(T::Size)),
        ],
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
        Id::LibraryIndex,
        "__builtin_index",
        "index",
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
    gcc_descriptor(
        Id::LibraryIsalnum,
        "__builtin_isalnum",
        "isalnum",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIsalpha,
        "__builtin_isalpha",
        "isalpha",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIsblank,
        "__builtin_isblank",
        "isblank",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIscntrl,
        "__builtin_iscntrl",
        "iscntrl",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIsdigit,
        "__builtin_isdigit",
        "isdigit",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIsgraph,
        "__builtin_isgraph",
        "isgraph",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIslower,
        "__builtin_islower",
        "islower",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIsprint,
        "__builtin_isprint",
        "isprint",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIspunct,
        "__builtin_ispunct",
        "ispunct",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIsspace,
        "__builtin_isspace",
        "isspace",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIsupper,
        "__builtin_isupper",
        "isupper",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryIsxdigit,
        "__builtin_isxdigit",
        "isxdigit",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    descriptor(
        Id::LibraryLabs,
        "__builtin_labs",
        "labs",
        Signature {
            result:     T::Scalar(S::Long),
            parameters: &[T::Scalar(S::Long)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Long)))],
    ),
    descriptor(
        Id::LibraryLlabs,
        "__builtin_llabs",
        "llabs",
        Signature {
            result:     T::Scalar(S::LongLong),
            parameters: &[T::Scalar(S::LongLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::LongLong)))],
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
        Id::LibraryMempcpy,
        "__builtin_mempcpy",
        "mempcpy",
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
        Id::LibraryRealloc,
        "__builtin_realloc",
        "realloc",
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Pointer(S::Void, false), T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, false))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryRindex,
        "__builtin_rindex",
        "rindex",
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
        Id::LibraryStpcpy,
        "__builtin_stpcpy",
        "stpcpy",
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
        Id::LibraryStpncpy,
        "__builtin_stpncpy",
        "stpncpy",
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
        Id::LibraryStrcasecmp,
        "__builtin_strcasecmp",
        "strcasecmp",
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
        Id::LibraryStrcat,
        "__builtin_strcat",
        "strcat",
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
        Id::LibraryStrcspn,
        "__builtin_strcspn",
        "strcspn",
        Signature {
            result:     T::Size,
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
        Id::LibraryStrdup,
        "__builtin_strdup",
        "strdup",
        Signature {
            result:     T::Pointer(S::Char, false),
            parameters: &[T::Pointer(S::Char, true)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Pointer(S::Char, true)))],
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
        Id::LibraryStrncasecmp,
        "__builtin_strncasecmp",
        "strncasecmp",
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
        Id::LibraryStrncat,
        "__builtin_strncat",
        "strncat",
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
        Id::LibraryStrndup,
        "__builtin_strndup",
        "strndup",
        Signature {
            result:     T::Pointer(S::Char, false),
            parameters: &[T::Pointer(S::Char, true), T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, true))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    gcc_descriptor(
        Id::LibraryStrnlen,
        "__builtin_strnlen",
        "strnlen",
        Signature {
            result:     T::Size,
            parameters: &[T::Pointer(S::Char, true), T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, true))),
            A::value(C::Parameter(T::Size)),
        ],
    ),
    descriptor(
        Id::LibraryStrpbrk,
        "__builtin_strpbrk",
        "strpbrk",
        Signature {
            result:     T::Pointer(S::Char, false),
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
    descriptor(
        Id::LibraryStrspn,
        "__builtin_strspn",
        "strspn",
        Signature {
            result:     T::Size,
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
        Id::LibraryStrstr,
        "__builtin_strstr",
        "strstr",
        Signature {
            result:     T::Pointer(S::Char, false),
            parameters: &[T::Pointer(S::Char, true), T::Pointer(S::Char, true)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, true))),
            A::value(C::Parameter(T::Pointer(S::Char, true))),
        ],
    ),
    gcc_descriptor(
        Id::LibraryTolower,
        "__builtin_tolower",
        "tolower",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
    gcc_descriptor(
        Id::LibraryToupper,
        "__builtin_toupper",
        "toupper",
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
    ),
];

pub(crate) fn ordinary(name: &str) -> Option<&'static BuiltinDescriptor> {
    BUILTINS
        .binary_search_by_key(&name, |d| d.library.expect("library shard"))
        .ok()
        .map(|index| &BUILTINS[index])
        .or_else(|| super::math::ordinary(name))
}

/// Whether `name` is one of the POSIX and BSD functions here rather than an
/// ISO C one. GCC declares these implicitly only in its GNU modes
/// (`DEF_EXT_LIB_BUILTIN` in its `builtins.def`); a strict ISO mode leaves
/// the ordinary name to the program, though the `__builtin_` alias stays.
pub(crate) fn extension_name(name: &str) -> bool {
    matches!(
        name,
        "bcmp"
            | "bcopy"
            | "bzero"
            | "index"
            | "mempcpy"
            | "rindex"
            | "stpcpy"
            | "stpncpy"
            | "strcasecmp"
            | "strdup"
            | "strncasecmp"
            | "strndup"
            | "strnlen"
    )
}

/// A library alias only GCC has: Clang 23 rejects `__builtin_aligned_alloc`,
/// `__builtin_strnlen` and the `<ctype.h>` aliases as unknown builtins,
/// though it knows the ordinary names as library functions.
const fn gcc_descriptor(
    id: Id,
    spelling: &'static str,
    library: &'static str,
    signature: Signature,
    arguments: &'static [A],
) -> BuiltinDescriptor {
    let mut descriptor = descriptor(id, spelling, library, signature, arguments);
    descriptor.availability.feature = Some(Feature::GccOnlyBuiltins);
    descriptor
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
