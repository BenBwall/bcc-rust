//! Sparse endpoints associate expanded token ranges with macro invocation
//! sites. Completed endpoints can be discarded as preprocessing advances.
//!
//! C99: phase-4 provenance, §5.1.1.2 paragraph 1, p. 10; PDF p. 22.
//! Diagnostic locations: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
//! Macro expansion itself belongs to preprocessing.

use super::{
    Bump,
    SegmentedVec,
    SourceVector,
};

/// Endpoints are normally appended in source-arena order. Keep sparse keys
/// packed, and reuse end locations within each invocation. Unlike a global
/// interner, the transient pool is cleared during preprocessing compaction.
///
/// Both tables live in the translation-unit arena. They grow by segments, so
/// the parser-token table, which holds an entry for every macro-expanded
/// token until parsing reads past it, never copies itself into a larger
/// buffer and leaves the old one behind.
pub(super) struct ExpansionSites<'tu> {
    pub(super) entries: SegmentedVec<'tu, (u32, u32)>,
    pub(super) ends:    SegmentedVec<'tu, SourceVector>,
}

impl<'tu> ExpansionSites<'tu> {
    pub(super) fn insert(&mut self, end: u32, id: u32) {
        if let Some(last) = self.entries.last_mut() {
            if last.0 == end {
                last.1 = id;
                return;
            }
            if last.0 > end {
                let index = self.entries.partition_point(|&(key, _)| key < end);
                if self.entries[index].0 == end {
                    self.entries[index].1 = id;
                } else {
                    self.entries.insert(index, (end, id));
                }
                return;
            }
        }
        self.entries.push((end, id));
    }

    pub(super) fn get(&self, end: u32) -> Option<u32> {
        let &(last, id) = self.entries.last()?;
        if end >= last {
            return (end == last).then_some(id);
        }
        let index = self.entries.partition_point(|&(key, _)| key < end);
        self.entries
            .get(index)
            .filter(|&&(key, _)| key == end)
            .map(|&(_, id)| id)
    }

    pub(super) fn site_id(&mut self, site: SourceVector) -> u32 {
        if self.ends.last() == Some(&site) {
            return u32::try_from(self.ends.len() - 1)
                .expect("macro expansion site index exceeds u32::MAX");
        }
        let id =
            u32::try_from(self.ends.len()).expect("macro expansion site index exceeds u32::MAX");
        self.ends.push(site);
        id
    }

    pub(super) fn discard_before(&mut self, end: u32) {
        let count = self.entries.partition_point(|&(key, _)| key < end);
        // Compact geometrically for full-batch input; renumbering the whole
        // tail after every declaration would make parsing quadratic.
        if count == 0 || count < self.entries.len() / 2 {
            return;
        }
        self.entries.discard_front(count);
        let Some(first_id) = self.entries.iter().map(|&(_, id)| id).min() else {
            self.ends.clear();
            return;
        };
        self.ends.discard_front(first_id as usize);
        self.entries.for_each_mut(|(_, id)| *id -= first_id);
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.ends.clear();
    }

    pub(super) fn new_in(tu: &'tu Bump) -> Self {
        Self {
            entries: SegmentedVec::new_in(tu),
            ends:    SegmentedVec::new_in(tu),
        }
    }
}
