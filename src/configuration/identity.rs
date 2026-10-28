//! The identity policy (`docs/design/driver-modes.md`, "Identity policy"):
//! where bcc models a behaviour only as GCC and Clang have it, which of the
//! two each compiler identity follows.
//!
//! What an identity claims (its macros, its version, which types and
//! keywords exist) follows its own compiler, and code matches on
//! [`CompilerIdentity`] for that. A rule bcc implements both ways (a
//! folding rule, an evaluation format, when an extension is reported)
//! instead asks [`CompilerIdentity::rules`], or
//! [`CompilerIdentity::ms_compatible_rules`] when `cl` is known to behave
//! as Clang's model of it.

use super::CompilerIdentity;

impl CompilerIdentity {
    /// Whose rules the identity follows where GCC and Clang differ: GCC's
    /// for GCC and TCC, which implements a subset of GCC's GNU dialect;
    /// Clang's for Clang and clang-cl; and GCC's for MSVC, whose `cl` lacks
    /// most of the GNU and C23 extensions Clang accepts silently, so GCC's
    /// rules, which reject or report more, are the closer model. A rule
    /// where `cl` is known to match Clang uses
    /// [`Self::ms_compatible_rules`] instead.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the driver's -MG and -H rules call it next")
    )]
    pub(crate) const fn rules(self) -> RuleFamily {
        match self {
            | Self::Clang | Self::ClangCl => RuleFamily::Clang,
            | Self::Gcc | Self::Tcc | Self::Msvc => RuleFamily::Gcc,
        }
    }

    /// [`Self::rules`] for a rule where `cl` is known to behave as Clang's
    /// MS-compatible mode, which clang-cl models it with (MSVC's documented
    /// warnings C4311 and C4312, for instance, report only a cast to or
    /// from a narrower integer, as Clang's do): Clang's for MSVC too.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "#174's pointer cast rule adopts it; the policy names it now"
        )
    )]
    pub(crate) const fn ms_compatible_rules(self) -> RuleFamily {
        match self {
            | Self::Msvc => RuleFamily::Clang,
            | _ => self.rules(),
        }
    }
}

/// The two compilers whose differing rules bcc models, of which
/// [`CompilerIdentity::rules`] chooses one.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum RuleFamily {
    Gcc,
    Clang,
}
