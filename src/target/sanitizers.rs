//! `UndefinedBehaviorSanitizer`'s vocabulary: the checks of GCC's and Clang's
//! `-fsanitize=undefined`, by the names their options give them, and what the
//! options ask of each check: whether it runs, and whether a failure runs the
//! `trap` instruction or calls the run-time library, which reports it and then
//! returns or ends the program.
//!
//! [`SanitizerOption::parse`] reads one option, [`Named::of`] one name in its
//! list, as the compiler that bcc answers as knows it, and
//! [`UndefinedSanitizer::apply`] updates what the options asked so far.
//! Lowering writes each check (`lowering/sanitizer.rs`), and the link adds the
//! library the compiler would link (`UndefinedSanitizer::link_flags`).
//!
//! The two compilers' `undefined` groups differ. Measured with GCC 13.3 and
//! Clang 23.1: both hold `shift`, `integer-divide-by-zero`, `unreachable`,
//! `vla-bound`, `null`, `return`, `signed-integer-overflow`, `bool`, `enum`,
//! the array bounds check, `alignment`, `object-size`, `nonnull-attribute`,
//! `returns-nonnull-attribute`, `vptr`, `pointer-overflow` and `builtin`;
//! Clang's also holds `float-cast-overflow` and `function`. Neither holds
//! `float-divide-by-zero`. `return`, `enum` and `vptr` check nothing in C: a
//! function may end without a value its caller does not use, an enumeration's
//! range is its whole underlying type, and C has no classes.
//! GCC manual, "Instrumentation Options"; Clang,
//! "`UndefinedBehaviorSanitizer`".

use bitflags::bitflags;

bitflags! {
    /// `UndefinedBehaviorSanitizer` checks, one bit for each name that turns
    /// on a single check.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub(crate) struct UbChecks: u32 {
        /// Signed `+`, `-`, `*`, negation, `++` and `--` that overflow, and
        /// the most negative value divided by -1. C99 §6.5p5.
        const SIGNED_INTEGER_OVERFLOW = 1 << 0;
        /// A left shift of a negative value, or one that shifts a set bit
        /// into or past the sign bit. C99 §6.5.7p4.
        const SHIFT_BASE = 1 << 1;
        /// A shift by a negative count or by the width or more. C99 §6.5.7p3.
        const SHIFT_EXPONENT = 1 << 2;
        /// An integer `/` or `%` by zero. C99 §6.5.5p5.
        const INTEGER_DIVIDE_BY_ZERO = 1 << 3;
        /// An access, or a member access, through a null pointer.
        /// C99 §6.5.3.2p4.
        const NULL = 1 << 4;
        /// An access through a pointer misaligned for its type.
        /// C99 §6.3.2.3p7.
        const ALIGNMENT = 1 << 5;
        /// An access past the end of the object it points into.
        const OBJECT_SIZE = 1 << 6;
        /// An index outside an array of known size. C99 §6.5.6p8.
        const ARRAY_BOUNDS = 1 << 7;
        /// `__builtin_unreachable` reached.
        const UNREACHABLE = 1 << 8;
        /// The end of a value-returning function reached: C++ only.
        const RETURN = 1 << 9;
        /// A variable length array bound that is not positive. C99 §6.7.5.2p5.
        const VLA_BOUND = 1 << 10;
        /// A floating value converted to an integer type that cannot hold
        /// its integer part. C99 §6.3.1.4p1.
        const FLOAT_CAST_OVERFLOW = 1 << 11;
        /// A `_Bool` object read with a value other than 0 or 1.
        /// C99 §6.2.6.1p5.
        const BOOL = 1 << 12;
        /// An enumeration object read with a value outside its range: C++
        /// only, since a C enumeration's range is its underlying type's.
        const ENUM = 1 << 13;
        /// Pointer arithmetic that wraps around the address space or leaves
        /// a null pointer. C99 §6.5.6p8.
        const POINTER_OVERFLOW = 1 << 14;
        /// Zero passed to `__builtin_clz` or `__builtin_ctz`.
        const BUILTIN = 1 << 15;
        /// A null pointer passed for a `nonnull` parameter.
        const NONNULL_ATTRIBUTE = 1 << 16;
        /// A null pointer returned from a `returns_nonnull` function.
        const RETURNS_NONNULL_ATTRIBUTE = 1 << 17;
        /// C++ dynamic types: nothing in C.
        const VPTR = 1 << 18;
        /// Clang's call through a function pointer of the wrong type.
        const FUNCTION = 1 << 19;
        /// A floating division by zero.
        const FLOAT_DIVIDE_BY_ZERO = 1 << 20;
        /// GCC's bounds check of trailing arrays too.
        const BOUNDS_STRICT = 1 << 21;
        /// Clang's `local-bounds`, which traps on an access outside a local
        /// object.
        const LOCAL_BOUNDS = 1 << 22;
    }
}

impl UbChecks {
    /// Clang's `-fsanitize=undefined`: GCC's, with the floating conversion
    /// check and the function type check.
    const CLANG_UNDEFINED: Self = Self::GCC_UNDEFINED
        .union(Self::FLOAT_CAST_OVERFLOW)
        .union(Self::FUNCTION);
    /// GCC's `-fsanitize=undefined`.
    const GCC_UNDEFINED: Self = Self::SHIFT_BASE
        .union(Self::SHIFT_EXPONENT)
        .union(Self::INTEGER_DIVIDE_BY_ZERO)
        .union(Self::UNREACHABLE)
        .union(Self::VLA_BOUND)
        .union(Self::NULL)
        .union(Self::RETURN)
        .union(Self::SIGNED_INTEGER_OVERFLOW)
        .union(Self::BOOL)
        .union(Self::ENUM)
        .union(Self::ARRAY_BOUNDS)
        .union(Self::ALIGNMENT)
        .union(Self::NONNULL_ATTRIBUTE)
        .union(Self::RETURNS_NONNULL_ATTRIBUTE)
        .union(Self::OBJECT_SIZE)
        .union(Self::VPTR)
        .union(Self::POINTER_OVERFLOW)
        .union(Self::BUILTIN);
    /// The checks bcc writes. `RETURN`, `ENUM` and `VPTR` are here because
    /// in C neither compiler checks anything for them either.
    pub(crate) const IMPLEMENTED: Self = Self::SIGNED_INTEGER_OVERFLOW
        .union(Self::SHIFT_BASE)
        .union(Self::SHIFT_EXPONENT)
        .union(Self::INTEGER_DIVIDE_BY_ZERO)
        .union(Self::NULL)
        .union(Self::ALIGNMENT)
        .union(Self::OBJECT_SIZE)
        .union(Self::ARRAY_BOUNDS)
        .union(Self::UNREACHABLE)
        .union(Self::RETURN)
        .union(Self::VLA_BOUND)
        .union(Self::FLOAT_CAST_OVERFLOW)
        .union(Self::BOOL)
        .union(Self::ENUM)
        .union(Self::POINTER_OVERFLOW)
        .union(Self::BUILTIN)
        .union(Self::NONNULL_ATTRIBUTE)
        .union(Self::RETURNS_NONNULL_ATTRIBUTE)
        .union(Self::VPTR);
    /// The checks that always end the program: both compilers refuse to
    /// recover from them.
    const UNRECOVERABLE: Self = Self::UNREACHABLE.union(Self::RETURN);
}

/// What `-fsanitize=undefined` and its kin ask of the checks: which run, which
/// of those trap, and which of the others return from the run-time library's
/// report. `gcc` says whose library reports and which conventions the static
/// data follow (`lowering/sanitizer.rs`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct UndefinedSanitizer {
    pub(crate) checks:  UbChecks,
    pub(crate) trap:    UbChecks,
    pub(crate) recover: UbChecks,
    pub(crate) gcc:     bool,
}

impl UndefinedSanitizer {
    pub(crate) const NONE: Self = Self {
        checks:  UbChecks::empty(),
        trap:    UbChecks::empty(),
        recover: UbChecks::empty(),
        gcc:     false,
    };

    /// The state before any option: no check runs, none traps, and every
    /// check but those that end the program recovers, as in both compilers.
    pub(crate) const fn initial(gcc: bool) -> Self {
        Self {
            checks: UbChecks::empty(),
            trap: UbChecks::empty(),
            recover: UbChecks::all().difference(UbChecks::UNRECOVERABLE),
            gcc,
        }
    }

    /// Applies the sanitizer option `option` to the state before it.
    /// `address` and `thread` are the other sanitizers' switches. Names bcc
    /// does not implement change nothing here: the personality refused the
    /// options that would need them.
    pub(crate) fn apply(
        &mut self,
        option: SanitizerOption<'_>,
        address: &mut bool,
        thread: &mut bool,
    ) {
        let clang = !self.gcc;
        let SanitizerOption {
            kind,
            enable,
            names,
        } = option;
        let Some(names) = names else {
            match kind {
                | ListKind::Recover => self.recover.set(UbChecks::all(), enable),
                | ListKind::Trap | ListKind::Sanitize => self.trap.set(UbChecks::all(), enable),
            }
            return;
        };
        for name in names.split(',') {
            let checks = match Named::of(name, clang) {
                | Some(Named::Checks(checks)) => checks,
                | Some(Named::All) => UbChecks::all(),
                | Some(Named::Address) if kind == ListKind::Sanitize => {
                    *address = enable;
                    continue;
                },
                | Some(Named::Thread) if kind == ListKind::Sanitize => {
                    *thread = enable;
                    continue;
                },
                | _ => continue,
            };
            let set = match kind {
                | ListKind::Sanitize => &mut self.checks,
                | ListKind::Recover => &mut self.recover,
                | ListKind::Trap => &mut self.trap,
            };
            set.set(checks, enable);
            if kind == ListKind::Sanitize && !enable && name == "all" {
                (*address, *thread) = (false, false);
            }
        }
    }

    /// The state the options leave: only the checks bcc writes run, and those
    /// that end the program never recover.
    pub(crate) fn finish(self) -> Self {
        let checks = self.checks & UbChecks::IMPLEMENTED;
        Self {
            checks,
            trap: self.trap & checks,
            recover: self.recover & checks.difference(UbChecks::UNRECOVERABLE),
            gcc: self.gcc,
        }
    }

    /// Whether some check runs that reports through the run-time library.
    pub(crate) const fn needs_runtime(self) -> bool {
        !self.checks.difference(self.trap).is_empty()
    }
}

/// One sanitizer option of GCC's and Clang's: `-f[no-]sanitize=`,
/// `-f[no-]sanitize-recover=` or `-f[no-]sanitize-trap=` and its
/// comma-separated `names`; `-f[no-]sanitize-recover` or `-f[no-]sanitize-trap`
/// without them, which apply to every check; and
/// `-f[no-]sanitize-undefined-trap-on-error`, which traps on every check
/// (`kind` [`ListKind::Sanitize`] without names).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SanitizerOption<'a> {
    pub(crate) kind:   ListKind,
    pub(crate) enable: bool,
    pub(crate) names:  Option<&'a str>,
}

/// Which property of the checks a list option sets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ListKind {
    Sanitize,
    Recover,
    Trap,
}

impl<'a> SanitizerOption<'a> {
    /// The sanitizer option `text` is, if it is one.
    pub(crate) fn parse(text: &'a str) -> Option<Self> {
        let (enable, rest) = match text.strip_prefix("-fno-") {
            | Some(rest) => (false, rest),
            | None => (true, text.strip_prefix("-f")?),
        };
        let (kind, names) = match rest {
            | "sanitize-recover" => (ListKind::Recover, None),
            | "sanitize-trap" => (ListKind::Trap, None),
            | "sanitize-undefined-trap-on-error" => (ListKind::Sanitize, None),
            | _ => match rest.split_once('=')? {
                | ("sanitize", names) => (ListKind::Sanitize, Some(names)),
                | ("sanitize-recover", names) => (ListKind::Recover, Some(names)),
                | ("sanitize-trap", names) => (ListKind::Trap, Some(names)),
                | _ => return None,
            },
        };
        Some(Self {
            kind,
            enable,
            names,
        })
    }

    /// The option's name before its list, as the compilers quote it:
    /// `-fsanitize=`, `-fno-sanitize-recover=` and so on.
    pub(crate) fn spelling(self) -> &'static str {
        match (self.kind, self.enable) {
            | (ListKind::Sanitize, true) => "-fsanitize=",
            | (ListKind::Sanitize, false) => "-fno-sanitize=",
            | (ListKind::Recover, true) => "-fsanitize-recover=",
            | (ListKind::Recover, false) => "-fno-sanitize-recover=",
            | (ListKind::Trap, true) => "-fsanitize-trap=",
            | (ListKind::Trap, false) => "-fno-sanitize-trap=",
        }
    }
}

/// What a name in a sanitizer list stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Named {
    /// `UndefinedBehaviorSanitizer` checks.
    Checks(UbChecks),
    /// `address`: `AddressSanitizer`.
    Address,
    /// `thread`: `ThreadSanitizer`.
    Thread,
    /// `all`, which only the lists that turn checks off, and the recovery
    /// and trap lists, take.
    All,
    /// A sanitizer of the compiler's that bcc does not have.
    Other,
}

impl Named {
    /// What `name` stands for in GCC's lists, or Clang's (`clang`), or `None`
    /// when that compiler does not know it.
    pub(crate) fn of(name: &str, clang: bool) -> Option<Self> {
        use UbChecks as C;
        let checks = match name {
            | "address" => return Some(Self::Address),
            | "thread" => return Some(Self::Thread),
            | "all" => return Some(Self::All),
            | "undefined" =>
                if clang {
                    C::CLANG_UNDEFINED
                } else {
                    C::GCC_UNDEFINED
                },
            | "shift" => C::SHIFT_BASE | C::SHIFT_EXPONENT,
            | "shift-base" => C::SHIFT_BASE,
            | "shift-exponent" => C::SHIFT_EXPONENT,
            | "integer-divide-by-zero" => C::INTEGER_DIVIDE_BY_ZERO,
            | "unreachable" => C::UNREACHABLE,
            | "vla-bound" => C::VLA_BOUND,
            | "null" => C::NULL,
            | "return" => C::RETURN,
            | "signed-integer-overflow" => C::SIGNED_INTEGER_OVERFLOW,
            | "alignment" => C::ALIGNMENT,
            | "object-size" => C::OBJECT_SIZE,
            | "float-divide-by-zero" => C::FLOAT_DIVIDE_BY_ZERO,
            | "float-cast-overflow" => C::FLOAT_CAST_OVERFLOW,
            | "nonnull-attribute" => C::NONNULL_ATTRIBUTE,
            | "returns-nonnull-attribute" => C::RETURNS_NONNULL_ATTRIBUTE,
            | "bool" => C::BOOL,
            | "enum" => C::ENUM,
            | "vptr" => C::VPTR,
            | "pointer-overflow" => C::POINTER_OVERFLOW,
            | "builtin" => C::BUILTIN,
            // Clang's `bounds` adds `local-bounds` to the array bounds check.
            | "bounds" if clang => C::ARRAY_BOUNDS | C::LOCAL_BOUNDS,
            | "bounds" => C::ARRAY_BOUNDS,
            | "array-bounds" if clang => C::ARRAY_BOUNDS,
            | "local-bounds" if clang => C::LOCAL_BOUNDS,
            | "function" if clang => C::FUNCTION,
            | "bounds-strict" if !clang => C::BOUNDS_STRICT,
            | "kernel-address" | "hwaddress" | "kernel-hwaddress" | "leak"
            | "shadow-call-stack" => return Some(Self::Other),
            | "pointer-compare" | "pointer-subtract" if !clang => return Some(Self::Other),
            | "memory"
            | "kernel-memory"
            | "memtag"
            | "memtag-stack"
            | "memtag-heap"
            | "memtag-globals"
            | "numerical"
            | "realtime"
            | "type"
            | "alloc-token"
            | "unsigned-integer-overflow"
            | "unsigned-shift-base"
            | "implicit-unsigned-integer-truncation"
            | "implicit-signed-integer-truncation"
            | "implicit-integer-sign-change"
            | "implicit-integer-truncation"
            | "implicit-integer-arithmetic-value-change"
            | "implicit-bitfield-conversion"
            | "implicit-integer-conversion"
            | "implicit-conversion"
            | "integer"
            | "nullability-arg"
            | "nullability-assign"
            | "nullability-return"
            | "nullability"
            | "undefined-trap"
            | "cfi"
            | "cfi-cast-strict"
            | "cfi-derived-cast"
            | "cfi-icall"
            | "cfi-mfcall"
            | "cfi-unrelated-cast"
            | "cfi-nvcall"
            | "cfi-vcall"
            | "kcfi"
            | "safe-stack"
            | "dataflow"
            | "scudo"
            | "fuzzer"
            | "fuzzer-no-link"
                if clang =>
                return Some(Self::Other),
            | _ => return None,
        };
        Some(Self::Checks(checks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each compiler's group, the options' order, and the checks that never
    /// recover.
    #[test]
    fn options_apply_in_order() {
        let apply = |gcc: bool, options: &[&str]| {
            let mut state = UndefinedSanitizer::initial(gcc);
            let (mut address, mut thread) = (false, false);
            for option in options {
                state.apply(
                    SanitizerOption::parse(option).expect("a sanitizer option"),
                    &mut address,
                    &mut thread,
                );
            }
            (state.finish(), address)
        };
        let (gcc, _) = apply(true, &["-fsanitize=undefined"]);
        let (clang, _) = apply(false, &["-fsanitize=undefined"]);
        assert!(!gcc.checks.contains(UbChecks::FLOAT_CAST_OVERFLOW));
        assert!(clang.checks.contains(UbChecks::FLOAT_CAST_OVERFLOW));
        assert!(gcc.recover.contains(UbChecks::SIGNED_INTEGER_OVERFLOW));
        assert!(!gcc.recover.contains(UbChecks::UNREACHABLE));
        assert!(gcc.needs_runtime());
        let (state, address) = apply(
            false,
            &[
                "-fsanitize=undefined,address",
                "-fno-sanitize=shift-base",
                "-fsanitize-trap=undefined",
                "-fno-sanitize-trap=null",
            ],
        );
        assert!(address);
        assert!(state.checks.contains(UbChecks::SHIFT_EXPONENT));
        assert!(!state.checks.contains(UbChecks::SHIFT_BASE));
        assert_eq!(state.checks.difference(state.trap), UbChecks::NULL);
        let (state, _) = apply(
            true,
            &["-fsanitize=undefined", "-fsanitize-undefined-trap-on-error"],
        );
        assert!(!state.needs_runtime());
        let (state, _) = apply(
            true,
            &[
                "-fsanitize=signed-integer-overflow",
                "-fno-sanitize-recover",
            ],
        );
        assert!(state.recover.is_empty());
        let (state, address) = apply(true, &["-fsanitize=address,null", "-fno-sanitize=all"]);
        assert!(!address && state.checks.is_empty());
        assert_eq!(
            Named::of("array-bounds", true),
            Some(Named::Checks(UbChecks::ARRAY_BOUNDS))
        );
        assert_eq!(Named::of("array-bounds", false), None);
        assert_eq!(Named::of("function", false), None);
    }
}
