//! A set of small integers, one bit each, in its own virtual-memory region.
//!
//! The words live in a [`RegionVec`], so the set grows in place by
//! committing pages as larger members arrive and never leaves an outgrown
//! copy behind. Only inserting a member past the current words grows it;
//! reading or removing a member past them is answered without touching
//! memory, because every such member is absent.

use super::region_vec::RegionVec;

type Word = u64;

const WORD_BITS: usize = Word::BITS as usize;

/// A dense bit set indexed by `usize`. See the module docs.
#[derive(Debug, Default)]
pub(crate) struct RegionBitSet {
    words: RegionVec<Word>,
}

impl RegionBitSet {
    /// An empty set. It reserves its region when the first member is
    /// inserted.
    pub(crate) const fn new() -> Self {
        Self {
            words: RegionVec::new(),
        }
    }

    const fn position(index: usize) -> (usize, Word) {
        (index / WORD_BITS, 1 << (index % WORD_BITS))
    }

    pub(crate) fn contains(&self, index: usize) -> bool {
        let (word, bit) = Self::position(index);
        self.words.get(word).is_some_and(|&value| value & bit != 0)
    }

    /// Adds `index`, first extending the words to reach it.
    pub(crate) fn insert(&mut self, index: usize) {
        let (word, bit) = Self::position(index);
        if word >= self.words.len() {
            // The vector only hands out elements it has written, and its
            // memory need not be fresh from the OS, so the new words are
            // written as zeros: one store per 64 members, done once per word.
            self.words
                .extend(std::iter::repeat_n(0, word + 1 - self.words.len()));
        }
        self.words[word] |= bit;
    }

    /// Removes `index`. A member past the words is already absent.
    pub(crate) fn remove(&mut self, index: usize) {
        let (word, bit) = Self::position(index);
        if let Some(value) = self.words.get_mut(word) {
            *value &= !bit;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RegionBitSet,
        WORD_BITS,
    };
    use crate::util::vm::{
        MAX_COMMIT_AHEAD,
        MAX_COMMIT_STEP,
        accounting,
    };

    #[test]
    fn inserts_and_removes_members() {
        let mut set = RegionBitSet::new();
        assert!(!set.contains(0));
        set.insert(3);
        set.insert(64);
        set.insert(65);
        assert!(set.contains(3));
        assert!(set.contains(64));
        assert!(set.contains(65));
        assert!(!set.contains(2));
        assert!(!set.contains(4));
        assert!(!set.contains(63));
        set.remove(64);
        assert!(!set.contains(64));
        assert!(set.contains(3));
        assert!(set.contains(65));
        set.remove(64);
        set.insert(3);
        assert!(set.contains(3));
        assert_eq!(set.words.len(), 2);
    }

    #[test]
    fn members_past_the_words_are_absent_and_cost_nothing() {
        let before = accounting::live();
        let mut set = RegionBitSet::new();
        assert!(!set.contains(usize::MAX));
        assert!(!set.contains(u32::MAX as usize));
        set.remove(u32::MAX as usize);
        set.remove(usize::MAX);
        assert!(set.words.is_empty());
        assert_eq!(
            accounting::live(),
            before,
            "reads and removals reserve nothing"
        );
        set.insert(5);
        assert!(!set.contains(u32::MAX as usize));
        set.remove(u32::MAX as usize);
        assert_eq!(set.words.len(), 1);
        assert!(set.contains(5));
    }

    #[test]
    fn grows_across_commit_steps_and_keeps_its_members() {
        let before = accounting::live();
        let mut set = RegionBitSet::new();
        // The first commit steps are 64 and 128 KiB of words.
        let past_two_steps = (64 + 128) * 1024 * 8 + 1;
        set.insert(1);
        set.insert(past_two_steps);
        set.insert(past_two_steps - 1);
        assert!(set.contains(1));
        assert!(set.contains(past_two_steps));
        assert!(set.contains(past_two_steps - 1));
        assert!(!set.contains(past_two_steps + 1));
        assert_eq!(set.words.len(), past_two_steps / WORD_BITS + 1);
        assert_eq!(set.words[0], 0b10);
        assert!(
            set.words[1..set.words.len() - 1]
                .iter()
                .all(|&word| word == 0)
        );
        let committed = accounting::live().committed - before.committed;
        assert!(committed >= set.words.len() * 8, "{committed}");
        assert!(
            committed <= set.words.len() * 8 + MAX_COMMIT_AHEAD,
            "{committed}"
        );
        assert_eq!(accounting::live().regions, before.regions + 1);
        drop(set);
        assert_eq!(accounting::live(), before);
    }

    #[cfg_attr(miri, ignore = "writes megabytes a word at a time")]
    #[test]
    fn large_members_commit_by_their_word() {
        let before = accounting::live();
        let mut set = RegionBitSet::new();
        // Ten million members span 1.2 MiB of words, past the 1 MiB step.
        let large = 10_000_000;
        set.insert(large);
        assert!(set.contains(large));
        assert!(!set.contains(large - 1));
        assert!(!set.contains(large + 1));
        let committed = accounting::live().committed - before.committed;
        assert!(committed <= large / 8 + MAX_COMMIT_STEP, "{committed}");
        set.remove(large);
        assert!(!set.contains(large));
    }
}
