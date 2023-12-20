use std::convert::Infallible;

use crate::util::{string_cache::StringCache, Captures};

use super::{Position, TranslationPhase};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct NewlineTracking<'a> {
    input: &'a str,
    position: Position,
    is_middle_of_windows_newline: bool,
}

impl<'a> NewlineTracking<'a> {
    pub(crate) fn new(input: &'a str, string_cache: &mut StringCache) -> Self {
        Self {
            is_middle_of_windows_newline: false,
            input,
            position: Position {
                index: 0,
                line: 1,
                column: 1,
                source_file: string_cache.intern("<stdin>"),
            },
        }
    }

    pub(crate) fn positions(self) -> impl Iterator<Item = Position> + Captures<&'a ()> {
        struct Positions<'b> {
            super_: NewlineTracking<'b>,
        }
        impl Iterator for Positions<'_> {
            type Item = Position;
            fn next(&mut self) -> Option<Position> {
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

impl Iterator for NewlineTracking<'_> {
    type Item = Result<char, Infallible>;
    fn next(&mut self) -> Option<Self::Item> {
        let c = self.input.get(self.position.index..)?.chars().next()?;
        let next = self
            .input
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
        self.input
            .get(self.position.index..)
            .map_or((0, Some(0)), |s| s.chars().size_hint())
    }
}

impl TranslationPhase for NewlineTracking<'_> {
    type SavePoint = Position;
    type Error = Infallible;
    type Yield = char;
    fn save(&self) -> Self::SavePoint {
        self.position
    }
    fn restore(&mut self, save_point: Self::SavePoint) {
        self.position = save_point;
    }
    fn current_position(&self) -> Position {
        self.position
    }
}

pub(crate) fn phase_0_newline_tracking<'a>(
    input: &'a str,
    string_cache: &mut StringCache,
) -> NewlineTracking<'a> {
    NewlineTracking::new(input, string_cache)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use rstest::rstest;

    use crate::{
        translation_phases::Position,
        util::string_cache::{Id, StringCache},
    };
    proptest! {
        #[test]
        fn test_noop_translation_phase(input in String::arbitrary()) {
            let phase = super::NewlineTracking::new(&input, &mut StringCache::new());
            prop_assert!(phase.collect::<String>() == input, "phase.collect() != input");
        }
    }

    const ID0: Id = Id::from_usize(0);

    fn position(index: usize, line: usize, column: usize) -> Position {
        Position {
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
    fn test_current_position(#[case] input: &str, #[case] expected: Vec<Position>) {
        let phase = super::NewlineTracking::new(input, &mut StringCache::new());
        let actual = phase.positions().collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
