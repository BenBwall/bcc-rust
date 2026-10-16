//! Original source text carries a lazily built index of line starts.
//! Diagnostics reuse that index instead of rescanning the same file.
//!
//! C99: source locations for required diagnostics, §5.1.1.3 paragraph 1,
//! p. 11; PDF p. 23. This record does not interpret C source.

use std::cell::OnceCell;

/// Source text and its lazily built physical-line index share one identity.
/// Replacing the text replaces the index, while rendering another diagnostic
/// reuses it without rescanning the file.
pub(super) struct SourceText<'tu> {
    pub(super) text:        &'tu str,
    pub(super) line_starts: OnceCell<&'tu [usize]>,
}
