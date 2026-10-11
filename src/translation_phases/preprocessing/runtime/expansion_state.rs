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
