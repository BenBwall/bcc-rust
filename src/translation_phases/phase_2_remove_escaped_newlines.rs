use thiserror::Error;

use super::{
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    Position,
    TranslationPhase,
};
use crate::bail;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct RemoveEscapedNewlines<Prev> {
    inner:            Inner<Prev>,
    last_was_newline: bool,
    state:            State,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Error)]
#[error("missing final newline")]
pub(crate) struct MissingNewlineError(pub(crate) Position);

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Error)]
pub(crate) enum RemoveEscapedNewlinesError<PrevError> {
    #[error(transparent)]
    Inner(PrevError),
    #[error(transparent)]
    MissingFinalNewLine(MissingNewlineError),
}

impl<PrevError> GetSeverity for RemoveEscapedNewlinesError<PrevError>
where
    PrevError: GetSeverity,
{
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::Inner(e) => e.severity(),
            | Self::MissingFinalNewLine(_) => ErrorSeverity::Warning,
        }
    }
}

impl<PrevError> GetPosition for RemoveEscapedNewlinesError<PrevError>
where
    PrevError: GetPosition,
{
    fn position(&self) -> Position {
        match self {
            | Self::Inner(e) => e.position(),
            | Self::MissingFinalNewLine(e) => e.0,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct SavePoint<Inner> {
    pub(crate) inner:            Inner,
    pub(crate) last_was_newline: bool,
    pub(crate) state:            State,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum State {
    Default,
    EscapedNewline,
    Done,
}

impl<Inner> super::SavePoint for SavePoint<Inner>
where
    Inner: super::SavePoint,
{
    fn current_position(&self) -> Position {
        self.inner.current_position()
    }
}

impl<Prev> RemoveEscapedNewlines<Prev> {
    pub(crate) fn new(previous_phase: Prev) -> Self {
        Self {
            inner:            Inner { previous_phase },
            last_was_newline: false,
            state:            State::Default,
        }
    }
}

impl<Prev> Iterator for RemoveEscapedNewlines<Prev>
where
    Prev: TranslationPhase<Yield = char> + Iterator<Item = Result<char, Prev::Error>>,
{
    type Item = Result<char, RemoveEscapedNewlinesError<Prev::Error>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.state {
            | State::Default => match self.inner.next() {
                | Some(Ok(c)) => {
                    self.last_was_newline = c == '\n';
                    Some(Ok(c))
                },
                | Some(Err(e)) => Some(Err(RemoveEscapedNewlinesError::Inner(e))),
                | None =>
                    if self.last_was_newline {
                        self.state = State::Done;
                        None
                    } else {
                        self.state = State::EscapedNewline;
                        Some(Err(RemoveEscapedNewlinesError::MissingFinalNewLine(
                            MissingNewlineError(self.current_position()),
                        )))
                    },
            },
            | State::EscapedNewline => {
                self.state = State::Done;
                Some(Ok('\n'))
            },
            | State::Done => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (lower, upper) = self.inner.previous_phase.size_hint();
        if let Some(upper) = upper {
            (lower, Some(upper + 1))
        } else {
            (lower, None)
        }
    }
}

impl<Prev> TranslationPhase for RemoveEscapedNewlines<Prev>
where
    Prev: TranslationPhase<Yield = char>,
{
    type Error = RemoveEscapedNewlinesError<Prev::Error>;
    type SavePoint = SavePoint<Prev::SavePoint>;
    type Yield = char;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner:            self.inner.previous_phase.save(),
            last_was_newline: self.last_was_newline,
            state:            self.state,
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.inner.previous_phase.restore(save_point.inner);
        self.last_was_newline = save_point.last_was_newline;
        self.state = save_point.state;
    }

    fn current_position(&self) -> Position {
        self.inner.previous_phase.current_position()
    }
}
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct Inner<Prev> {
    previous_phase: Prev,
}

impl<Prev> Iterator for Inner<Prev>
where
    Prev: TranslationPhase<Yield = char>,
{
    type Item = Result<char, Prev::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let c = bail!(self.previous_phase.next()?);
            let save_point = self.previous_phase.save();
            let Some(Ok(second)) = self.previous_phase.next() else {
                self.previous_phase.restore(save_point);
                return Some(Ok(c));
            };
            if c == '\\' && second == '\n' {
                continue;
            }
            self.previous_phase.restore(save_point);
            break Some(Ok(c));
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.previous_phase.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::{
        translation_phases::{
            phase_0_newline_tracking::NewlineTracking,
            phase_1_map_character_sets::{
                MapCharacterSets,
                MapCharacterSetsError,
            },
        },
        util::string_cache::{
            Id as StringCacheId,
            StringCache,
        },
    };

    #[rstest]
    #[case("", vec![Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(Position {
        index:       0,
        line:        1,
        column:      1,
        source_file: StringCacheId::from_usize(0),
    }))), Ok('\n')])]
    #[case("a", vec![Ok('a'), Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(Position {
        index:       1,
        line:        1,
        column:      2,
        source_file: StringCacheId::from_usize(0),
    }))), Ok('\n')])]
    #[case("a\n", vec![Ok('a'), Ok('\n')])]
    #[case("a\\\n", vec![Ok('a'), Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(Position {
        index:       3,
        line:        2,
        column:      1,
        source_file: StringCacheId::from_usize(0),
    }))), Ok('\n')])]
    #[case("abc\n", vec![Ok('a'), Ok('b'), Ok('c'), Ok('\n')])]
    #[case("abcabcbb", vec![Ok('a'), Ok('b'), Ok('c'), Ok('a'), Ok('b'), Ok('c'), Ok('b'), Ok('b'), Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(Position {
        index:       8,
        line:        1,
        column:      9,
        source_file: StringCacheId::from_usize(0),
    }))), Ok('\n')])]
    #[case(
        "#define MAX(a, b) (a > b) \\\n ? a \\\n : b\n",
        vec![Ok('#'), Ok('d'), Ok('e'), Ok('f'), Ok('i'), Ok('n'), Ok('e'), Ok(' '), Ok('M'), Ok('A'), Ok('X'), Ok('('), Ok('a'), Ok(','), Ok(' '), Ok('b'), Ok(')'), Ok(' '), Ok('('), Ok('a'), Ok(' '), Ok('>'), Ok(' '), Ok('b'), Ok(')'), Ok(' '), Ok(' '), Ok('?'), Ok(' '), Ok('a'), Ok(' '), Ok(' '), Ok(':'), Ok(' '), Ok('b'), Ok('\n')]
    )]
    #[case("abc\\\nabc\n", vec![Ok('a'), Ok('b'), Ok('c'), Ok('a'), Ok('b'), Ok('c'), Ok('\n')])]

    fn test_phase_2_remove_escaped_newlines(
        #[case] input: &str,
        #[case] expected: Vec<Result<char, RemoveEscapedNewlinesError<Infallible>>>,
    ) {
        let mut string_cache = StringCache::new();
        let actual =
            RemoveEscapedNewlines::new(NewlineTracking::new(input, string_cache.intern("<input>")))
                .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
    #[rstest]
    #[case("", vec![Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(Position {
        index:       0,
        line:        1,
        column:      1,
        source_file: StringCacheId::from_usize(0),
    }))), Ok('\n')])]
    #[case("a", vec![Ok('a'), Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(Position {
        index:       1,
        line:        1,
        column:      2,
        source_file: StringCacheId::from_usize(0),
    }))), Ok('\n')])]
    #[case("??=", vec![Ok('#'), Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(Position {
        index:       3,
        line:        1,
        column:      4,
        source_file: StringCacheId::from_usize(0),
    }))), Ok('\n')])]
    #[case("??=define FOO 1\\\r\n\r\n", vec![Ok('#'), Ok('d'), Ok('e'), Ok('f'), Ok('i'), Ok('n'), Ok('e'), Ok(' '), Ok('F'), Ok('O'), Ok('O'), Ok(' '), Ok('1'), Ok('\n')])]
    fn test_phase_1_and_2(
        #[case] input: &str,
        #[case] expected: Vec<
            Result<char, RemoveEscapedNewlinesError<MapCharacterSetsError<Infallible>>>,
        >,
    ) {
        let mut string_cache = StringCache::new();
        let actual = RemoveEscapedNewlines::new(MapCharacterSets::new(NewlineTracking::new(
            input,
            string_cache.intern("<input>"),
        )))
        .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
