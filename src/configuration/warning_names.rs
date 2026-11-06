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

/// The warnings the command line names by `cl`'s code, each with its level
/// and the group of bcc's own options that controls it. `cl`'s command line
/// shows level 1 by default. C4047 and C4133 are warnings in `cl`, where the
/// identity's Clang rules make the groups errors. Microsoft Learn: "/w,
/// /W0, /W1, /W2, /W3, /W4, /w1, /w2, /w3, /w4, /Wall, /wd, /we, /wo, /Wv,
/// /WX (Warning level)" and each warning's page, as for C4090 and C4113.
pub(crate) const CL_CODED_WARNINGS: [(u16, u8, &str); 7] = [
    (4013, 3, "implicit-function-declaration"),
    (4047, 1, "int-conversion"),
    (4133, 1, "incompatible-pointer-types"),
    // After C4133, whose group holds theirs. bcc keeps its own words for
    // these two, but `/wd`, `/we` and the levels name them.
    (4090, 1, "incompatible-pointer-types-discards-qualifiers"),
    (4113, 1, "incompatible-function-pointer-types"),
    (4311, 1, "pointer-to-int-cast"),
    (4312, 1, "int-to-pointer-cast"),
];
