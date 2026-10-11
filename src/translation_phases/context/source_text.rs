use super::OnceCell;

/// Source text and its lazily built physical-line index share one identity.
/// Replacing the text replaces the index, while rendering another diagnostic
/// reuses it without rescanning the file.
pub(super) struct SourceText<'tu> {
    pub(super) text:        &'tu str,
    pub(super) line_starts: OnceCell<&'tu [usize]>,
}
