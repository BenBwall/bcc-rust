use super::{Position, TranslationPhase};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct RemoveEscapedNewlines<Prev> {
    inner: Inner<Prev>,
    last_was_newline: bool,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct SavePoint<Inner> {
    pub(crate) inner: Inner,
    pub(crate) last_was_newline: bool,
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
    fn new(previous_phase: Prev) -> Self {
        Self {
            inner: Inner { previous_phase },
            last_was_newline: false,
        }
    }
}
impl<Prev: TranslationPhase> RemoveEscapedNewlines<Prev> {
    pub(crate) fn save(&self) -> SavePoint<Prev::SavePoint> {
        SavePoint {
            inner: self.inner.previous_phase.save(),
            last_was_newline: self.last_was_newline,
        }
    }
    pub(crate) fn restore(&mut self, save_point: SavePoint<Prev::SavePoint>) {
        self.inner.previous_phase.restore(save_point.inner);
        self.last_was_newline = save_point.last_was_newline;
    }
}

impl<Prev> Iterator for RemoveEscapedNewlines<Prev>
where
    Prev: TranslationPhase + Iterator<Item = char>,
{
    type Item = char;
    fn next(&mut self) -> Option<char> {
        if let Some(c) = self.inner.next() {
            self.last_was_newline = c == '\n';
            Some(c)
        } else if !self.last_was_newline {
            self.last_was_newline = true;
            // Add final newline if it's missing.
            Some('\n')
        } else {
            None
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
    Prev: TranslationPhase + Iterator<Item = char>,
{
    type SavePoint = SavePoint<Prev::SavePoint>;
    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner: self.inner.previous_phase.save(),
            last_was_newline: self.last_was_newline,
        }
    }
    fn restore(&mut self, save_point: Self::SavePoint) {
        self.inner.previous_phase.restore(save_point.inner);
        self.last_was_newline = save_point.last_was_newline;
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
    Prev: TranslationPhase + Iterator<Item = char>,
{
    type Item = char;
    fn next(&mut self) -> Option<char> {
        loop {
            let c = self.previous_phase.next()?;
            let save_point = self.previous_phase.save();
            let Some(second) = self.previous_phase.next() else {
                return Some(c);
            };
            if c == '\\' && second == '\n' {
                continue;
            }
            self.previous_phase.restore(save_point);
            break Some(c);
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.previous_phase.size_hint()
    }
}

pub(crate) fn phase_2_remove_escaped_newlines<Prev>(
    previous_stage: Prev,
) -> RemoveEscapedNewlines<Prev> {
    RemoveEscapedNewlines::new(previous_stage)
}

#[cfg(test)]
mod tests {
    use crate::{
        translation_phases::phase_0_newline_tracking::NewlineTracking,
        util::string_cache::StringCache,
    };

    use super::super::phase_1_map_character_sets::phase_1_map_character_sets;
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    #[rstest]
    #[case("", "\n")]
    #[case("a", "a\n")]
    #[case("a\n", "a\n")]
    #[case("a\\\n", "a\n")]
    #[case("abc\n", "abc\n")]
    #[case("abcabcbb", "abcabcbb\n")]
    #[case(
        "#define MAX(a, b) (a > b) \\\n ? a \\\n : b\n",
        "#define MAX(a, b) (a > b)  ? a  : b\n"
    )]
    #[case("abc\\\nabc\n", "abcabc\n")]

    fn test_phase_2_remove_escaped_newlines(#[case] input: &str, #[case] expected: &str) {
        let actual =
            phase_2_remove_escaped_newlines(NewlineTracking::new(input, &mut StringCache::new()))
                .collect::<String>();
        assert_eq!(actual, expected);
    }
    #[rstest]
    #[case("", "\n")]
    #[case("a", "a\n")]
    #[case("??=", "#\n")]
    #[case("??=define FOO 1\\\r\n\r\n", "#define FOO 1\n")]
    fn test_phase_1_and_2(#[case] input: &str, #[case] expected: &str) {
        let actual = phase_2_remove_escaped_newlines(phase_1_map_character_sets(
            NewlineTracking::new(input, &mut StringCache::new()),
        ))
        .collect::<String>();
        assert_eq!(actual, expected);
    }
}
