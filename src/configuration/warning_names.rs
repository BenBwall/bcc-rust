//! The warning options each compiler knows, which decide whether a diagnostic
//! pragma names something real. The lists are the driver's
//! (`scripts/update_driver_options.py`): GCC 15.2's `--help` and the pinned
//! Clang's `--autocomplete`. A real option bcc has no group for stays quiet.
//! C99: implementation-defined pragma behavior, §6.10.6 paragraph 1, p. 159;
//! PDF p. 171.

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
