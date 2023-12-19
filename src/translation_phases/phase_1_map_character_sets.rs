use super::{Position, TranslationPhase};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct MapCharacterSets<Prev> {
    pub(crate) previous_phase: Prev,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct SavePoint<Inner> {
    pub(crate) inner: Inner,
}

impl<Inner> super::SavePoint for SavePoint<Inner>
where
    Inner: super::SavePoint,
{
    fn current_position(&self) -> Position {
        self.inner.current_position()
    }
}

impl<Prev> Iterator for MapCharacterSets<Prev>
where
    Prev: TranslationPhase + Iterator<Item = char>,
{
    type Item = char;
    fn next(&mut self) -> Option<char> {
        let c = self.previous_phase.next()?;
        let save_point = self.save();
        let next = self.previous_phase.next();
        if c == '\r' && next == Some('\n') {
            return Some('\n');
        }
        if c == '\r' {
            self.restore(save_point);
            return Some('\n');
        }
        if c != '?' {
            self.restore(save_point);
            return Some(c);
        }
        let Some(next) = next else {
            self.restore(save_point);
            return Some(c);
        };
        if next != '?' {
            self.restore(save_point);
            return Some(c);
        }
        let Some(trigraph) = self.previous_phase.next() else {
            self.restore(save_point);
            return Some(c);
        };
        let to_yield = match trigraph {
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
            | _ => {
                self.restore(save_point);
                return Some(c);
            },
        };
        Some(to_yield)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.previous_phase.size_hint()
    }
}

impl<Prev> TranslationPhase for MapCharacterSets<Prev>
where
    Prev: TranslationPhase + Iterator<Item = char>,
{
    type SavePoint = SavePoint<Prev::SavePoint>;
    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner: self.previous_phase.save(),
        }
    }
    fn restore(&mut self, save_point: Self::SavePoint) {
        self.previous_phase.restore(save_point.inner);
    }
    fn current_position(&self) -> Position {
        self.previous_phase.current_position()
    }
}

impl<Prev> MapCharacterSets<Prev> {
    fn new(previous_phase: Prev) -> Self {
        Self { previous_phase }
    }
}

pub(crate) fn phase_1_map_character_sets<Prev>(previous_phase: Prev) -> MapCharacterSets<Prev> {
    MapCharacterSets::new(previous_phase)
}

#[cfg(test)]
mod tests {
    use crate::translation_phases::phase_0_newline_tracking::phase_0_newline_tracking;

    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
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
        use crate::util::string_cache::StringCache;
        let actual =
            phase_1_map_character_sets(phase_0_newline_tracking(input, &mut StringCache::new()));
        assert_eq!(actual.collect::<String>(), expected);
    }
}
