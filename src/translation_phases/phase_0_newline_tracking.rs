use std::convert::Infallible;

use super::{
    Context,
    TranslationPhase,
};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct NewlineTracking<'ctx> {
    next:    Option<Input>,
    context: &'ctx RefCell<Context>,
}

impl<'ctx> NewlineTracking<'ctx> {
    pub(crate) fn new(context: &'ctx RefCell<Context>) -> Self {
        Self {
            next: None,
            context,
        }
    }

    fn get_next(&mut self) -> Option<Input> {
        if let Some(next) = self.next.take() {
            return Some(next);
        }
        match self.context.borrow_mut().next_byte() {
            | Err(e) => match e {},
            | Ok(Some(next)) => Some(next),
            | Ok(None) => None,
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
    type Yield = Input;

    fn next(&mut self) -> Result<Option<Input>, Infallible> {
        let Some(curr) = self.get_next() else {
            return Ok(None);
        };
        let next = match self.borrow_context_mut().next_value() {
            | Ok(Some(next)) => Some(next),
            | Ok(None) => None,
            | Err(e) => match e {},
        };
        if curr.value == b'\r' && matches!(next, Some(Input { value: b'\n', .. })) {
            let mut borrow = self.borrow_context_mut();
            borrow.source.column_number = 1;
            borrow.source.line_number += 1;
            curr.value = b'\n';
            curr.length = 2;
            return Ok(Some(curr));
        }
        if matches!(curr.value, b'\n' | b'\r') {
            let mut borrow = self.borrow_context_mut();
            borrow.source.column_number = 1;
            borrow.source.line_number += 1;
            self.next = next;
            curr.value = b'\n';
            return Ok(Some(curr));
        }
        context.source.column_number += 1;
        self.next = next;
        Ok(Some(None))
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
