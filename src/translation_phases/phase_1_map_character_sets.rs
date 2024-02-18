use std::convert::Infallible;

use super::{
    phase_0_newline_tracking::NewlineTracking,
    TranslationPhase,
};
use crate::util::stack_queue::StackQueue;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct MapCharacterSets<'ctx> {
    pub(crate) pending_question_marks: StackQueue<SourcePosition, 3>,
    pub(crate) is_popping_question_marks: bool,
    pub(crate) prev: NewlineTracking<'ctx>,
}

impl<'ctx> TranslationPhase for MapCharacterSets<'ctx> {
    type Error = Infallible;
    type Yield = char;

    fn next(&mut self) -> Result<Option<Self::Yield>, Self::Error> {
        if self.pending_question_marks > 0 {
            self.pending_question_marks -= 1;
            return Ok(Some(b'?'));
        }
        let Ok(Some(next)) = self.prev.next() else {
            return Ok(None);
        };
        self.pending_bytes.push_back(next);
        match (
            next == b'?',
            self.question_marks_seen == QuestionMarksSeen::Two,
        ) {
            | (false, false) => {
                self.question_marks_seen = QuestionMarksSeen::Zero;
                return Ok(Some(self.pending_bytes.pop_front().unwrap()));
            },
            | (true, false) => {
                self.question_marks_seen.increment();
                return Ok(None);
            },
            | (true, true) => {
                self.question_marks_seen.increment();
                return Ok(Some(self.pending_bytes.pop_front().unwrap()));
            },
            | (false, true) => (),
        }
        self.question_marks_seen = QuestionMarksSeen::Zero;
        let to_yield = match input {
            // Top left to bottom right order based on the table in the C99 standard.
            | '=' => '#',
            | ')' => ']',
            | '!' => '|',
            | '(' => '[',
            | '\'' => '^',
            | '>' => '}',
            | '/' => '\\',
            | '<' => '{',
            | '-' => '~',
            | _ => return Ok(Some(self.pending_chars.pop_front().unwrap())),
        };
        self.pending_chars.clear();
        Some(Ok(to_yield))
    }
}

impl<'ctx> MapCharacterSets<'ctx> {
    pub(crate) fn new(context: &'ctx RefCell<Context>) -> Self {
        Self {
            pending_chars:       StackQueue::new(),
            question_marks_seen: QuestionMarksSeen::Zero,
            prev:                NewlineTracking::new(context),
        }
    }

    pub(crate) fn borrow_context(&self) -> Ref<Context> {
        self.prev.borrow_context()
    }

    pub(crate) fn borrow_context_mut(&self) -> RefMut<Context> {
        self.prev.borrow_context_mut()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::translation_phases::test_run;
    #[rstest]
    #[case("", "")]
    #[case("a", "a")]
    #[case("?", "?")]
    #[case("??", "??")]
    #[case("??=", "#")]
    #[case("int main() ??<??>\n", "int main() {}\n")]
    #[case("??=define FOO 1\r\n", "#define FOO 1\n")]
    #[case("int x = 1;\n", "int x = 1;\n")]
    #[case("int long y = 5;\r", "int long y = 5;\n")]
    fn test_phase_1_map_character_sets(#[case] input: &str, #[case] expected: &str) {
        let mut actual = Vec::new();
        test_run(TestArgs {
            phase_1: Some(&mut actual),
            input: Box::new(input),
            name: "<input>".as_ref::<Path>().to_owned(),
            ..Default::default()
        });
        assert_eq!(actual.into_iter().collect::<String>(), expected);
    }
}
