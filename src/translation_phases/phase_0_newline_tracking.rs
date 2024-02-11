use std::convert::Infallible;

use super::{
    Context,
    TranslationPhase,
};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct NewlineTracking {
    last: Option<char>,
}

impl NewlineTracking {
    pub(crate) fn new() -> Self {
        Self { last: None }
    }
}

impl TranslationPhase for NewlineTracking {
    type Error = Infallible;
    type Input = char;
    type Yield = char;

    fn next_item(
        &mut self,
        curr: Self::Input,
        context: &mut Context,
    ) -> Result<Option<Self::Yield>, Self::Error> {
        let last = self.last.take();
        if last == Some('\r') && curr == '\n' {
            context.source.column_number = 1;
            context.source.line_number += 1;
            // Don't set last here, because we want to ignore the '\n' in the next
            // iteration.
            return Ok(Some('\n'));
        }

        if matches!(last, Some('\n' | '\r')) {
            context.source.column_number = 1;
            context.source.line_number += 1;
            self.last = Some(curr);
            return Some(Ok('\n'));
        }
        self.last = Some(curr);
        Some(None)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::Context;
    use crate::{
        translation_phases::SourcePosition,
        util::string_cache::{
            Id,
            StringCache,
        },
    };

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
    fn test_current_position(#[case] input: &str, #[case] expected_chars: Vec<char>) {
        let mut phase = super::NewlineTracking::new();
        let actual = run!(input, "<input>", phase);
        let expected = expected_chars
            .into_iter()
            .map(|c| Ok(c))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
