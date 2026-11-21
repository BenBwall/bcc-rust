//! Whether the pragmas and attributes of a translation unit may give its
//! functions a strict floating-point environment, which decides who
//! computes their invalid operations ([`super::constants::NanSource`]).
//! bcc does not model where such a pragma or attribute is in effect: when
//! the translation unit holds one the compiler it follows honors, every
//! function computes its invalid operations at run time, as bcc did before
//! it folded their NaNs. A static initializer, and code outside a function,
//! still fold with the compiler's NaN.
//!
//! - **Clang's rules.** `#pragma STDC FENV_ACCESS ON`, Microsoft's
//!   `fenv_access(on)`, a strict or `maytrap` exception behavior (`clang fp
//!   exceptions`, `float_control(except, on)`), and a constant `FENV_ROUND`
//!   direction other than to nearest each make Clang compute invalid operations
//!   at run time, at least at `-O0`, in the code they reach. Any of these
//!   pragmas, with any operand and anywhere in the translation unit
//!   ([`FloatingPragmas::environment`]), keeps every function's NaNs at run
//!   time; so does `-ftrapping-math` on the command line, where
//!   [`Analyzer::settle_strict_initializer`] still folds an automatic aggregate
//!   Clang's evaluator folds whole. `cl` reads the same pragmas, so they count
//!   for its identity too.
//! - **GCC's rules.** An `optimize` attribute or `#pragma GCC optimize` can
//!   turn trapping math back on for a function (`trapping-math`,
//!   `no-fast-math`, or an `-O` level after `-Ofast`). `#pragma GCC optimize`
//!   or `push_options` anywhere in the translation unit
//!   ([`FloatingPragmas::gcc_options`]) keeps every function's NaNs at run
//!   time, and so does an `optimize` attribute for every function analyzed
//!   after it: GCC applies one only to the function it declares, from that
//!   declaration on, and ignores one after the definition.
//!
//! C99: §7.6.1 paragraph 2, p. 189; PDF p. 201; §F.7.4 paragraph 1, p.
//! 449; PDF p. 461. C23: §7.6.2 paragraph 2, p. 213; PDF p. 226. Clang
//! extension: Clang Language Extensions, "Extensions to specify
//! floating-point flags". GNU extension: GCC manual, "Function Specific
//! Option Pragmas".
//!
//! [`FloatingPragmas::environment`]: crate::translation_phases::pragma_floating::FloatingPragmas::environment
//! [`FloatingPragmas::gcc_options`]: crate::translation_phases::pragma_floating::FloatingPragmas::gcc_options

use super::Analyzer;
use crate::configuration::CompilerIdentity;

impl Analyzer<'_, '_, '_> {
    /// Whether the functions of this translation unit may have a strict
    /// exception behavior under Clang's rules (and `cl`'s pragmas), so that
    /// their invalid operations are computed at run time: it holds a
    /// floating-point environment pragma, or `-ftrapping-math` is given.
    pub(super) fn clang_strict_exceptions(&self) -> bool {
        self.context.floating_pragmas.environment
            || self.context.configuration.requests_trapping_math()
    }

    /// Whether [`Self::settle_strict_initializer`] may fold an automatic
    /// aggregate's NaNs: only when the strict exception behavior comes
    /// from `-ftrapping-math` alone. A pragma can also set a constant
    /// directed rounding, under which Clang's evaluator refuses other
    /// operations (an exact subnormal result, for one), and bcc does not
    /// follow where a pragma is in effect, so every NaN stays at run time.
    pub(super) fn strict_aggregates_settle(&self) -> bool {
        !self.context.floating_pragmas.environment
    }

    /// Whether the functions of this translation unit compute their invalid
    /// operations at run time under GCC's rules although trapping math is
    /// off on the command line: it holds `#pragma GCC optimize` or
    /// `push_options`, an `optimize` attribute came before, or, for `cl`, it
    /// holds one of the pragmas Clang's rules read.
    pub(super) fn gcc_function_runtime(&self) -> bool {
        let pragmas = self.context.floating_pragmas;
        pragmas.gcc_options
            || self.functions.optimize_attribute
            || (self.context.configuration.compiler_identity() == CompilerIdentity::Msvc
                && pragmas.environment)
    }
}
