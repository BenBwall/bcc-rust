use std::convert::Infallible;

use super::{
    Context,
    TranslationPhase,
};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct NewlineTracking<'ctx> {
    next:        Option<Input>,
    context:     &'ctx RefCell<Context>,
    next_pos:    SourcePosition,
    current_pos: SourcePosition,
}

impl GetPosition for NewlineTracking<'_> {
    fn position(&self) -> SourcePosition {
        self.current_pos
    }
}

impl<'ctx> NewlineTracking<'ctx> {
    pub(crate) fn new(context: &'ctx RefCell<Context>) -> Self {
        let pos = context.borrow().position();
        Self {
            next: None,
            context,
            next_pos: pos,
            current_pos: pos,
        }
    }

    fn get_next(&mut self) -> Option<char> {
        if let Some(next) = self.next.take() {
            return Some(next);
        }
        match self.borrow_context_mut().next_char() {
            | Err(e) => match e {},
            | Ok(v) => v,
        }
    }

    pub(crate) fn borrow_context(&self) -> Ref<Context> {
        self.context.borrow()
    }

    pub(crate) fn borrow_context_mut(&self) -> RefMut<Context> {
        self.context.borrow_mut()
    }
}

impl TranslationPhase for NewlineTracking {
    type Error = Infallible;
    type Yield = char;

    fn next(&mut self) -> Result<Option<char>, Infallible> {
        self.current_pos = self.next_pos;
        let Some(curr) = self.get_next() else {
            return Ok(None);
        };
        self.next_pos = self.borrow_context().position();
        let next = match self.borrow_context_mut().next_char() {
            | Ok(v) => v,
            | Err(e) => match e {},
        };
        if cur == '\r' && matches!(next, Some('\n')) {
            let mut borrow = self.borrow_context_mut();
            borrow.source.column_number = NonZeroU32::MIN();
            borrow.source.line_number = borrow.source.line_number.saturating_add(1);
            return Ok(Some('\n'));
        }
        if matches!(curr.value, b'\n' | b'\r') {
            let mut borrow = self.borrow_context_mut();
            borrow.source.column_number = NonZeroU32::MIN();
            borrow.source.line_number = borrow.source.line_number.saturating_add(1);
            self.next = next;
            return Ok(Some('\n'));
        }
        borrow.source.column_number = borrow.source.column_number.saturating_add(1);
        self.next = next;
        Ok(Some(curr))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{
        Path,
        PathBuf,
    };

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use crate::{
        translation_phases::{
            test_run,
            SourcePosition,
        },
        util::string_cache::Id,
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
