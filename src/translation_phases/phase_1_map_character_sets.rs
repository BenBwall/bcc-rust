use std::convert::Infallible;

use super::TranslationPhase;
use crate::util::stack_queue::StackQueue;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
enum QuestionMarksSeen {
    Zero,
    One,
    Two,
}

impl QuestionMarksSeen {
    fn increment(&mut self) {
        *self = match self {
            | Self::Zero => Self::One,
            | Self::One => Self::Two,
            | Self::Two => Self::Two,
        };
    }

    fn decrement(&mut self) {
        *self = match self {
            | Self::Zero => Self::Zero,
            | Self::One => Self::Zero,
            | Self::Two => Self::One,
        };
    }

    fn value(&self) -> u8 {
        match self {
            | Self::Zero => 0,
            | Self::One => 1,
            | Self::Two => 2,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct MapCharacterSets {
    pub(crate) pending_chars:       StackQueue<char, 3>,
    pub(crate) question_marks_seen: QuestionMarksSeen,
}

impl TranslationPhase for MapCharacterSets {
    type Error = Infallible;
    type Input = char;
    type Yield = char;

    fn next_item(
        &mut self,
        input: Self::Input,
        context: &mut super::Context,
    ) -> Result<Option<Self::Yield>, Self::Error> {
        self.pending_chars.push_back(input);
        match (
            input == '?',
            self.question_marks_seen == QuestionMarksSeen::Two,
        ) {
            | (false, false) => {
                self.question_marks_seen = QuestionMarksSeen::Zero;
                return Ok(Some(self.pending_chars.pop_front().unwrap()));
            },
            | (true, false) => {
                self.question_marks_seen.increment();
                return Ok(None);
            },
            | (true, true) => {
                self.question_marks_seen.increment();
                return Ok(Some(self.pending_chars.pop_front().unwrap()));
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

    fn eoi(&mut self, _context: &mut super::Context) -> Result<Option<Self::Yield>, Self::Error> {
        Ok(self.pending_chars.pop_front())
    }
}

impl MapCharacterSets {
    pub(crate) fn new() -> Self {
        Self {
            pending_chars:       StackQueue::new(),
            question_marks_seen: QuestionMarksSeen::Zero,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::translation_phases::phase_0_newline_tracking::NewlineTracking;
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
        let phase_0 = NewlineTracking::new();
        let phase_1 = MapCharacterSets::new();
        let actual = run!(input, "<input>", phase_0, phase_1);
        assert_eq!(
            actual.into_iter().map(|r| r.unwrap()).collect::<String>(),
            expected
        );
    }
}
