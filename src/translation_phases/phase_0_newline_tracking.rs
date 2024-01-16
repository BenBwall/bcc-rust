use std::{
    convert::Infallible,
    sync::Arc,
};

use super::{
    SourcePosition,
    TranslationPhase,
};
use crate::util::string_cache::Id as StringCacheId;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct NewlineTracking {
    source: Arc<str>,
    position: SourcePosition,
    is_middle_of_windows_newline: bool,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct SavePoint {
    pub(crate) source: Arc<str>,
    pub(crate) position: SourcePosition,
    pub(crate) is_middle_of_windows_newline: bool,
}

impl super::SavePoint for SavePoint {
    fn current_position(&self) -> SourcePosition {
        self.position
    }
}

impl NewlineTracking {
    pub(crate) fn new(source: Arc<str>, source_file: StringCacheId) -> Self {
        Self {
            is_middle_of_windows_newline: false,
            source,
            position: SourcePosition {
                index: 0,
                line: 1,
                column: 1,
                source_file,
            },
        }
    }

    pub(crate) fn positions(self) -> impl Iterator<Item = SourcePosition> {
        struct Positions {
            super_: NewlineTracking,
        }
        impl Iterator for Positions {
            type Item = SourcePosition;

            fn next(&mut self) -> Option<SourcePosition> {
                let ret = self.super_.position;
                let _ = self.super_.next()?;
                Some(ret)
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.super_.size_hint()
            }
        }
        Positions { super_: self }
    }
}

impl Iterator for NewlineTracking {
    type Item = Result<char, Infallible>;

    fn next(&mut self) -> Option<Self::Item> {
        let c = self.source.get(self.position.index..)?.chars().next()?;
        let next = self
            .source
            .get(self.position.index + c.len_utf8()..)
            .and_then(|s| s.chars().next());

        if c == '\r' && next == Some('\n') {
            self.position.index += 1;
            self.position.column = 1;
            self.position.line += 1;
            self.is_middle_of_windows_newline = true;
            return Some(Ok(c));
        }

        if c == '\n' && !self.is_middle_of_windows_newline {
            self.position.index += 1;
            self.position.column = 1;
            self.position.line += 1;
            return Some(Ok(c));
        }

        if c == '\r' {
            self.position.index += 1;
            self.position.column = 1;
            self.position.line += 1;
            self.is_middle_of_windows_newline = false;
            return Some(Ok('\n'));
        }
        self.is_middle_of_windows_newline = false;
        self.position.index += c.len_utf8();
        self.position.column += c.len_utf8();
        Some(Ok(c))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source
            .get(self.position.index..)
            .map_or((0, Some(0)), |s| s.chars().size_hint())
    }
}

impl TranslationPhase for NewlineTracking {
    type Error = Infallible;
    type SavePoint = SavePoint;
    type Yield = char;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            source: self.source.clone(),
            position: self.position,
            is_middle_of_windows_newline: self.is_middle_of_windows_newline,
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.source = save_point.source;
        self.position = save_point.position;
        self.is_middle_of_windows_newline = save_point.is_middle_of_windows_newline;
    }

    fn current_position(&self) -> SourcePosition {
        self.position
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use rstest::rstest;

    use crate::{
        translation_phases::SourcePosition,
        util::string_cache::{
            Id,
            StringCache,
        },
    };
    proptest! {
        #[test]
        fn test_noop_translation_phase(input in String::arbitrary()) {
            let mut string_cache = StringCache::new();
            let phase = super::NewlineTracking::new(input.as_str().into(), string_cache.intern("<input>"));
            prop_assert!(phase.map(|r| r.unwrap_or_else(|e| match e{})).collect::<String>() == input, "phase.collect() != input");
        }
    }

    const ID0: Id = Id::from_usize(0);

    fn position(index: usize, line: usize, column: usize) -> SourcePosition {
        SourcePosition {
            index,
            line,
            column,
            source_file: ID0,
        }
    }

    #[rstest]
    #[case("", vec![])]
    #[case("abc\ndef\n", vec![
        position(0, 1, 1),
        position(1, 1, 2),
        position(2, 1, 3),
        position(3, 1, 4),
        position(4, 2, 1),
        position(5, 2, 2),
        position(6, 2, 3),
        position(7, 2, 4),
    ])]
    #[case("a\r\n\r\nb", vec![
        position(0, 1, 1),
        position(1, 1, 2),
        position(2, 2, 1),
        position(3, 2, 2),
        position(4, 3, 1),
        position(5, 3, 2),
    ])]
    #[case("a\rb", vec![
        position(0, 1, 1),
        position(1, 1, 2),
        position(2, 2, 1),
    ])]
    fn test_current_position(#[case] input: &str, #[case] expected: Vec<SourcePosition>) {
        let mut string_cache = StringCache::new();
        let phase = super::NewlineTracking::new(input.into(), string_cache.intern("<input>"));
        let actual = phase.positions().collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
