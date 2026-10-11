use std::fmt::Debug;

use super::{
    syntax::ExternalDeclaration,
    token_cursor,
};
use crate::util::region_vec::RegionVec;

/// One completely parsed translation unit: its source-ordered roots, which
/// borrow the syntax tree from the translation-unit arena.
///
/// The roots themselves stay in the region the parser collected them in,
/// which the unit owns and releases when it is dropped.
///
/// This is the shared boundary for callers, inspection, tests, and the future
/// semantic-analysis phase. Parser-machine state is deliberately not exposed.
///
/// C99: `translation-unit`, §6.9 paragraph 1, p. 140; PDF p. 152.
#[derive(Debug)]
pub(crate) struct ParsedTranslationUnit<'tu> {
    pub(super) roots: RegionVec<ExternalDeclaration<'tu>>,
}

/// [`ParsedTranslationUnit::raw_debug`]'s view.
struct RawRoots<'a, 'tu>(&'a [ExternalDeclaration<'tu>]);

/// One root in compact debug form, whatever the formatter's flags.
struct CompactRoot<'a, 'tu>(&'a ExternalDeclaration<'tu>);

/// The roots as a list of compact entries.
struct RawList<'a, 'tu>(&'a [ExternalDeclaration<'tu>]);

impl<'tu> ParsedTranslationUnit<'tu> {
    pub(crate) fn external_declarations(&self) -> &[ExternalDeclaration<'tu>] {
        &self.roots
    }

    /// The whole tree as Rust debug output, for storage debugging. Each root
    /// prints on one line in compact form even under `{:#?}`: indenting a
    /// deeply nested tree would make the output grow with the square of its
    /// depth.
    pub(crate) fn raw_debug(&self) -> impl Debug + Send + '_ {
        RawRoots(&self.roots)
    }
}

impl Debug for RawRoots<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedTranslationUnit")
            .field("roots", &RawList(self.0))
            .finish()
    }
}

impl Debug for RawList<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(CompactRoot))
            .finish()
    }
}

impl Debug for CompactRoot<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// Fully preprocessed parser input. The preprocessor can be dropped before
/// parser working memory is created.
///
/// C99: the output of translation phases 1-6 for one translation unit
/// (§5.1.1.1, p. 9; PDF p. 21; §5.1.1.2 paragraph 1, pp. 9-10;
/// PDF pp. 21-22).
pub(crate) struct PreprocessedTranslationUnit {
    pub(super) upstream: token_cursor::Upstream,
}
