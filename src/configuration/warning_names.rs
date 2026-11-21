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

/// A warning the command line names by `cl`'s code: its level, and the group
/// of bcc's own options that controls it.
#[derive(Clone, Copy)]
pub(crate) struct ClCodedWarning {
    pub(crate) code:   u16,
    pub(crate) level:  u8,
    pub(crate) group:  &'static str,
    /// The group is off by default in bcc, so a level that shows the warning
    /// names it too.
    pub(crate) enable: bool,
}

const fn coded(code: u16, level: u8, group: &'static str) -> ClCodedWarning {
    ClCodedWarning {
        code,
        level,
        group,
        enable: false,
    }
}

const fn enabled(code: u16, level: u8, group: &'static str) -> ClCodedWarning {
    ClCodedWarning {
        code,
        level,
        group,
        enable: true,
    }
}

/// The warnings the command line names by `cl`'s code, each with its level
/// and the group of bcc's own options that controls it. `cl`'s command line
/// shows level 1 by default. C4047 and C4133 are warnings in `cl`, where the
/// identity's Clang rules make the groups errors. A code may name several
/// groups (C4293 a negative and a too-large shift count), and a group
/// several codes, whose `/wd` and `/we` then set them all. Microsoft Learn:
/// "/w, /W0, /W1, /W2, /W3, /W4, /w1, /w2, /w3, /w4, /Wall, /wd, /we, /wo,
/// /Wv, /WX (Warning level)" and each warning's page, as for C4090 and C4113.
pub(crate) const CL_CODED_WARNINGS: [ClCodedWarning; 19] = [
    coded(4013, 3, "implicit-function-declaration"),
    // One group holds both, so `/wd` and `/we` of either set both.
    coded(4033, 1, "return-mismatch"),
    coded(4098, 1, "return-mismatch"),
    coded(4047, 1, "int-conversion"),
    coded(4133, 1, "incompatible-pointer-types"),
    // After C4133, whose group holds theirs. bcc keeps its own words for
    // these two, but `/wd`, `/we` and the levels name them.
    enabled(4090, 1, "incompatible-pointer-types-discards-qualifiers"),
    enabled(4113, 1, "incompatible-function-pointer-types"),
    coded(4311, 1, "pointer-to-int-cast"),
    coded(4312, 1, "int-to-pointer-cast"),
    enabled(4100, 4, "unused-parameter"),
    enabled(4101, 3, "unused-variable"),
    // `cl`'s variable whose initializer is its only reference.
    enabled(4189, 4, "unused-but-set-variable"),
    enabled(4018, 3, "sign-compare"),
    enabled(4389, 4, "sign-compare-equality"),
    enabled(4706, 4, "parentheses"),
    enabled(4293, 1, "shift-count-overflow"),
    enabled(4293, 1, "shift-count-negative"),
    // The code generator's, about the end of a function: one group holds
    // both.
    enabled(4715, 1, "return-type"),
    enabled(4716, 1, "return-type"),
];

/// What `cl` makes of one numbered warning: never shown (`/wd`, `disable`),
/// an error at any level (`/we`, `error`), or shown from a level (its own,
/// or `/wL` and `L:`'s). Microsoft Learn: "warning pragma".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ClWarningState {
    Disabled,
    Error,
    Level(u8),
}

/// `cl`'s warning level and what it makes of each warning in
/// [`CL_CODED_WARNINGS`], which `#pragma warning` changes: `push, n` sets
/// the level until its `pop`, and `default` or a level specifier undoes a
/// `/wd` or `/we` as `disable` and `error` replace a level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ClWarnings {
    level:  u8,
    states: [ClWarningState; CL_CODED_WARNINGS.len()],
}

impl ClWarnings {
    /// `cl`'s command line without options: level 1, each warning at its
    /// own.
    pub(crate) const DEFAULT: Self = {
        let mut states = [ClWarningState::Level(0); CL_CODED_WARNINGS.len()];
        let mut index = 0;
        while index < states.len() {
            states[index] = ClWarningState::Level(CL_CODED_WARNINGS[index].level);
            index += 1;
        }
        Self { level: 1, states }
    };

    pub(crate) const fn with_level(mut self, level: u8) -> Self {
        self.level = level;
        self
    }

    /// `state` for every entry of `code`.
    pub(crate) fn with_state(mut self, code: u16, state: ClWarningState) -> Self {
        for (index, warning) in CL_CODED_WARNINGS.iter().enumerate() {
            if warning.code == code {
                self.states[index] = state;
            }
        }
        self
    }

    /// `code`'s own level, which `default` restores.
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
        match self.states[index] {
            | ClWarningState::Level(level) if level > self.level => None,
            | ClWarningState::Disabled => None,
            | state => Some(state),
        }
    }

    /// Each entry's group and whether it is off, a warning or an error at
    /// the current level.
    pub(crate) fn classes(self) -> impl Iterator<Item = (&'static str, ClWarningState)> {
        CL_CODED_WARNINGS
            .iter()
            .zip(self.states)
            .map(move |(warning, state)| {
                let state = match state {
                    | ClWarningState::Level(level) if level > self.level =>
                        ClWarningState::Disabled,
                    | other => other,
                };
                (warning.group, state)
            })
    }
}
