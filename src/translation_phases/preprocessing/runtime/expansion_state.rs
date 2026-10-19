//! Output purpose, deferred-query state, and the expansion reset threshold.
//!
//! C99: translation phases 4-7, §5.1.1.2 paragraph 1 items 4-7, p. 10; PDF p.
//! 22. Deferred queries implement C23 §6.10.4.2 paragraph 3, p. 174; PDF p.
//! 187. These flags control reading and retained metadata, not language
//! analysis.

/// Parser-only diagnostics require invocation metadata; standalone token
/// production does not retain that side information.
#[derive(Debug, Clone, Copy)]
pub(in crate::translation_phases::preprocessing) enum OutputPurpose {
    Preprocessing,
    Parsing,
}

/// Whether conditional queries are evaluated or retained for a later embed
/// parameter evaluation. C23: §6.10.4.2p3, p. 174; PDF p. 187.
#[derive(Clone, Copy, PartialEq)]
pub(in crate::translation_phases::preprocessing) enum QueryExpansion {
    Evaluate,
    Defer,
}

/// Frames pushed after which the expansion arena is reset, at the next point
/// where no expansion is active. A reset copies the source-file frames, so it
/// is not worth doing after every top-level token.
pub(in crate::translation_phases::preprocessing) const RESET_EXPANSIONS_AFTER_FRAMES: usize = 256;
