//! Macro-replaced pack operands cross the batch boundary by token ordinal.
//! Semantic analysis alone interprets their stack and record-layout meaning.
//! C99: §6.10.6p1, p. 159; PDF p. 171 (implementation-defined pragma).

use super::SourceVectors;
use crate::util::string_cache::StringCacheId;

#[derive(Debug, Clone, Copy)]
pub(crate) struct PackPragma<'tu> {
    pub(crate) position: usize,
    pub(crate) operands: &'tu [PackToken],
    pub(crate) source:   SourceVectors,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PackToken {
    pub(crate) spelling:   StringCacheId,
    pub(crate) identifier: bool,
}
