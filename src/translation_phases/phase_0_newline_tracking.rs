use std::convert::Infallible;

use super::{
    Context,
    TranslationPhase,
};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct NewlineTracking {
    last: Option<char>,
    source: SharedString,
    index: usize,
    line_number: usize,
    column_number: usize,
}

impl NewlineTracking {
    pub(crate) fn new() -> Self {
        Self { last: None }
    }

    fn next(
        &mut self,
        curr: Option<char>,
        context: &mut Context,
    ) -> Result<Option<char>, Infallible> {
        let last = self.last.take();
        if last == Some('\r') && curr == Some('\n') {
            context.source.column_number = 1;
            context.source.line_number += 1;
            // Don't set last here, because we want to ignore the '\n' in the next
            // iteration.
            return Ok(Some('\n'));
        }

        if matches!(last, Some('\n' | '\r')) {
            context.source.column_number = 1;
            context.source.line_number += 1;
            self.last = curr;
            return Some(Ok('\n'));
        }
        self.last = curr;
        Some(None)
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
        self.next(Some(curr), context)
    }

    fn eoi(&mut self, context: &mut Context) -> Result<Option<Self::Yield>, Self::Error> {
        self.next(None, context)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use crate::{
        translation_phases::{
            test_run,
            SourcePosition,
        },
        util::string_cache::Id,
    };

    use std::path::{Path, PathBuf};

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
        let mut actual = Vec::new();
        test_run(TestArgs {
            phase_0: Some(&mut actual),
            input: Box::new(input),
            name: "<input>".as_ref::<Path>().to_owned(),
            ..Default::default()
        });
        assert_eq!(actual, expected_chars);
    }
}
