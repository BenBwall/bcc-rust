//! The warning options each compiler knows, which decide whether a diagnostic
//! pragma names something real, and the warnings `cl` numbers. The lists are
//! the driver's (`scripts/update_driver_options.py`): GCC 15.2's `--help` and
//! the pinned Clang's `--autocomplete`. A real option bcc has no group for
//! stays quiet. C99: implementation-defined pragma behavior, §6.10.6 paragraph
//! 1, p. 159; PDF p. 171.

const GCC_WARNINGS: &str = include_str!("../cli/personality/options/gcc-warnings.list");
const GCC_FLAGS: &str = include_str!("../cli/personality/options/gcc-flags.list");
const CLANG_WARNINGS: &str = include_str!("../cli/personality/options/clang-warnings.list");

/// Whether GCC knows the `-W` option `option`, spelled in full and positive:
/// `-Wformat=2` when it lists `-Wformat=`.
pub(crate) fn gcc_knows_warning(option: &str) -> bool {
    let name = option.split_once('=').map_or(option, |(name, _)| name);
    GCC_WARNINGS
        .lines()
        .any(|line| line == option || line.strip_suffix('=') == Some(name))
}

/// Whether GCC knows the `-f` option `option`.
pub(crate) fn gcc_knows_flag(option: &str) -> bool {
    GCC_FLAGS.lines().any(|line| line == option)
}

/// Whether Clang knows the warning group `option`, spelled in full.
pub(crate) fn clang_knows_warning(option: &str) -> bool {
    CLANG_WARNINGS.lines().any(|line| line == option)
}

use super::DiagnosticGroup as G;

/// A warning the command line names by `cl`'s code: its level, and the
/// groups of bcc's own options that control it, if any do.
#[derive(Clone, Copy)]
pub(crate) struct ClCodedWarning {
    pub(crate) code:   u16,
    pub(crate) level:  u8,
    pub(crate) groups: &'static [G],
    /// The groups are off by default in bcc, so a level that shows the
    /// warning names them too.
    pub(crate) enable: bool,
    /// `cl` does not show the warning unless an option or a pragma names
    /// it or `/Wall` is given.
    pub(crate) off:    bool,
}

const fn coded(code: u16, level: u8, groups: &'static [G]) -> ClCodedWarning {
    ClCodedWarning {
        code,
        level,
        groups,
        enable: false,
        off: false,
    }
}

const fn enabled(code: u16, level: u8, groups: &'static [G]) -> ClCodedWarning {
    ClCodedWarning {
        code,
        level,
        groups,
        enable: true,
        off: false,
    }
}

/// A warning `cl` does not show by default.
const fn off(code: u16, level: u8, groups: &'static [G]) -> ClCodedWarning {
    ClCodedWarning {
        code,
        level,
        groups,
        enable: true,
        off: true,
    }
}

/// The warnings the command line names by `cl`'s code, each with its level
/// and the groups of bcc's own options that control it. `cl`'s command line
/// shows level 1 by default. C4047 and C4133 are warnings in `cl`, where the
/// identity's Clang rules make the groups errors. A code may hold several
/// groups (C4293 a negative and a too-large shift count), and a group
/// several codes, whose `/wd` and `/we` then set them all. A group without
/// a name (C4389's) is bcc's alone, which only these codes reach. Microsoft
/// Learn: "/w, /W0, /W1, /W2, /W3, /W4, /w1, /w2, /w3, /w4, /Wall, /wd,
/// /we, /wo, /Wv, /WX (Warning level)" and each warning's page, as for C4090,
/// C4113 and C4532.
pub(crate) const CL_CODED_WARNINGS: [ClCodedWarning; 26] = [
    coded(4013, 3, &[G::ImplicitFunctionDeclaration]),
    // One group holds both, so `/wd` and `/we` of either set both.
    coded(4033, 1, &[G::ReturnMismatch]),
    coded(4098, 1, &[G::ReturnMismatch]),
    coded(4047, 1, &[G::IntConversion]),
    coded(4133, 1, &[G::IncompatiblePointerTypes]),
    // After C4133, whose group holds theirs. bcc keeps its own words for
    // these two, but `/wd`, `/we` and the levels name them.
    enabled(4090, 1, &[G::PointerQualifiers]),
    enabled(4113, 1, &[G::IncompatibleFunctionPointerTypes]),
    coded(4311, 1, &[G::PointerToIntCast]),
    coded(4312, 1, &[G::IntToPointerCast]),
    enabled(4100, 4, &[G::UnusedParameter]),
    enabled(4101, 3, &[G::UnusedVariable]),
    // `cl`'s variable whose initializer is its only reference.
    enabled(4189, 4, &[G::UnusedButSetVariable]),
    enabled(4018, 3, &[G::SignCompare]),
    enabled(4389, 4, &[G::SignCompareEquality]),
    enabled(4706, 4, &[G::Parentheses]),
    enabled(4293, 1, &[G::ShiftCountOverflow, G::ShiftCountNegative]),
    enabled(4532, 1, &[G::JumpSehFinally]),
    enabled(4127, 4, &[G::ClConstantCondition]),
    // C4244's level depends on the conversion, so it has an entry for
    // each, lowest first; `/wd`, `/we`, `/wL` and the pragmas set them all,
    // and `default` gives each its own level.
    enabled(4244, 2, &[G::ClFloatingConversion]),
    enabled(4244, 3, &[G::ClWideConversion]),
    enabled(4244, 4, &[G::ClIntConversion]),
    enabled(4305, 2, &[G::ClTruncation]),
    enabled(4245, 4, &[G::ClSignedConversion]),
    // Off by default: what `cl` reports in place of some of C4244's
    // narrowings when it is shown.
    off(4242, 4, &[G::ClNarrowingConversion]),
    // The code generator's, about the end of a function, which its own
    // states govern (`ClWarnings::shown`) rather than a group.
    coded(4715, 1, &[]),
    coded(4716, 1, &[]),
];

/// What `cl` makes of one numbered warning: never shown (`/wd`, `disable`),
/// an error at any level (`/we`, `error`), or shown from a level from 1 to 4
/// (its own, or `/wL` and `L:`'s), every time or only the first time it is
/// shown so (`/wo`, `once`). Microsoft Learn: "warning pragma".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ClWarningState {
    Disabled,
    Error,
    Level(u8),
    Once(u8),
}

/// `cl`'s warning level and what it makes of each warning in
/// [`CL_CODED_WARNINGS`], which `#pragma warning` changes: `push, n` sets
/// the level until its `pop`, and `default` or a level specifier undoes a
/// `/wd` or `/we` as `disable` and `error` replace a level.
///
/// It packs four bits for the level and four for each warning's state, so
/// that every copy of the configuration stays small.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ClWarnings(u128);

const _: () = assert!(
    ClWarnings::BITS * (CL_CODED_WARNINGS.len() + 1) <= u128::BITS as usize,
    "the level and each warning's state take four bits of a u128"
);

impl ClWarningState {
    /// The four bits [`ClWarnings`] keeps for the state: 0 for disabled, the
    /// level from 1 to 4, 5 for an error, and the level with 8 for `once`.
    const fn bits(self) -> u128 {
        match self {
            | Self::Disabled => 0,
            | Self::Level(level) => Self::level_bits(level),
            | Self::Once(level) => 8 | Self::level_bits(level),
            | Self::Error => 5,
        }
    }

    const fn level_bits(level: u8) -> u128 {
        assert!(
            matches!(level, 1..=4),
            "cl's warning levels run from 1 to 4"
        );
        level as u128
    }

    const fn from_bits(bits: u8) -> Self {
        match bits {
            | 0 => Self::Disabled,
            | 5 => Self::Error,
            | 9..=12 => Self::Once(bits & 0b111),
            | level => Self::Level(level),
        }
    }
}

impl ClWarnings {
    const BITS: usize = 4;
    /// `cl`'s command line without options: level 1, each warning at its
    /// own.
    pub(crate) const DEFAULT: Self = {
        let mut warnings = Self(1);
        let mut index = 0;
        while index < CL_CODED_WARNINGS.len() {
            let warning = CL_CODED_WARNINGS[index];
            let state = if warning.off {
                ClWarningState::Disabled
            } else {
                ClWarningState::Level(warning.level)
            };
            warnings = warnings.with_index(index, state);
            index += 1;
        }
        warnings
    };
    const MASK: u128 = 0b1111;

    const fn with_index(self, index: usize, state: ClWarningState) -> Self {
        let shift = Self::BITS * (index + 1);
        Self(self.0 & !(Self::MASK << shift) | state.bits() << shift)
    }

    const fn state(self, index: usize) -> ClWarningState {
        ClWarningState::from_bits((self.0 >> (Self::BITS * (index + 1)) & Self::MASK) as u8)
    }

    /// The warning level, from 0 to 4.
    pub(crate) const fn level(self) -> u8 {
        (self.0 & Self::MASK) as u8
    }

    /// The warning level, from 0 to 4.
    pub(crate) const fn with_level(self, level: u8) -> Self {
        assert!(level <= 4, "cl's warning level runs from 0 to 4");
        Self(self.0 & !Self::MASK | level as u128)
    }

    /// `state` for each entry of `code`, whose level, if it has one, runs
    /// from 1 to 4.
    pub(crate) fn with_state(self, code: u16, state: ClWarningState) -> Self {
        let mut warnings = self;
        for (index, warning) in CL_CODED_WARNINGS.iter().enumerate() {
            if warning.code == code {
                warnings = warnings.with_index(index, state);
            }
        }
        warnings
    }

    /// Each entry of `code` shown from its own level again (`default`), or
    /// only the first time (`once`, `/wo`).
    pub(crate) fn with_default(self, code: u16, once: bool) -> Self {
        let mut warnings = self;
        for (index, warning) in CL_CODED_WARNINGS.iter().enumerate() {
            if warning.code == code {
                let state = if once {
                    ClWarningState::Once(warning.level)
                } else {
                    ClWarningState::Level(warning.level)
                };
                warnings = warnings.with_index(index, state);
            }
        }
        warnings
    }

    /// `code`'s own level, the lowest of its entries', which `default`
    /// restores.
    pub(crate) fn default_level(code: u16) -> Option<u8> {
        CL_CODED_WARNINGS
            .iter()
            .find(|warning| warning.code == code)
            .map(|warning| warning.level)
    }

    /// What the settings make of `code` at the current level: `None` when
    /// it is not shown.
    pub(crate) fn shown(self, code: u16) -> Option<ClWarningState> {
        let index = CL_CODED_WARNINGS
            .iter()
            .position(|warning| warning.code == code)?;
        self.shown_at(index)
    }

    fn shown_at(self, index: usize) -> Option<ClWarningState> {
        match self.state(index) {
            | ClWarningState::Level(level) | ClWarningState::Once(level) if level > self.level() =>
                None,
            | ClWarningState::Disabled => None,
            | state => Some(state),
        }
    }

    /// The index of `code` in [`CL_CODED_WARNINGS`], if the settings show it
    /// only once.
    pub(crate) fn shown_once(self, code: u16) -> Option<usize> {
        let index = CL_CODED_WARNINGS
            .iter()
            .position(|warning| warning.code == code)?;
        matches!(self.shown_at(index), Some(ClWarningState::Once(_))).then_some(index)
    }

    /// Each entry with whether it is off, a warning or an error at the
    /// current level.
    pub(crate) fn classes(self) -> impl Iterator<Item = (&'static ClCodedWarning, ClWarningState)> {
        CL_CODED_WARNINGS
            .iter()
            .enumerate()
            .map(move |(index, warning)| {
                (
                    warning,
                    self.shown_at(index).unwrap_or(ClWarningState::Disabled),
                )
            })
    }
}
