use super::{
    Position,
    TranslationPhase,
};

#[derive(Debug, PartialEq, Eq, Hash, Clone,)]
pub(crate) struct MapCharacterSets<Prev,> {
    pub(crate) previous_phase: Prev,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy,)]
pub(crate) struct SavePoint<Inner,> {
    pub(crate) inner: Inner,
}

impl<Inner,> super::SavePoint for SavePoint<Inner,>
where
    Inner: super::SavePoint,
{
    fn current_position(&self,) -> Position {
        self.inner.current_position()
    }
}

impl<Prev,> Iterator for MapCharacterSets<Prev,>
where
    Prev: TranslationPhase<Yield = char,> + Iterator<Item = Result<char, Prev::Error,>,>,
{
    type Item = Result<char, Prev::Error,>;

    fn next(&mut self,) -> Option<Self::Item,> {
        let c = self.previous_phase.next()??;
        let save_point = self.save();
        let next = self.previous_phase.next();
        if c == '\r' && next == Some('\n',) {
            return Some(Ok('\n',),);
        }
        if c == '\r' {
            self.restore(save_point,);
            return Some(Ok('\n',),);
        }
        if c != '?' {
            self.restore(save_point,);
            return Some(Ok(c,),);
        }
        let Some(next,) = next else {
            self.restore(save_point,);
            return Some(Ok(c,),);
        };
        if next != '?' {
            self.restore(save_point,);
            return Some(Ok(c,),);
        }
        let Some(trigraph,) = self.previous_phase.next() else {
            self.restore(save_point,);
            return Some(Ok(c,),);
        };
        let trigraph = trigraph?;
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
                self.restore(save_point,);
                return Some(Ok(c,),);
            },
        };
        Some(Ok(to_yield,),)
    }

    fn size_hint(&self,) -> (usize, Option<usize,>,) {
        self.previous_phase.size_hint()
    }
}

impl<Prev,> TranslationPhase for MapCharacterSets<Prev,>
where
    Prev: TranslationPhase<Yield = char,> + Iterator<Item = Result<char, Prev::Error,>,>,
{
    type Error = Prev::Error;
    type SavePoint = SavePoint<Prev::SavePoint,>;
    type Yield = char;

    fn save(&self,) -> Self::SavePoint {
        SavePoint {
            inner: self.previous_phase.save(),
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint,) {
        self.previous_phase.restore(save_point.inner,);
    }

    fn current_position(&self,) -> Position {
        self.previous_phase.current_position()
    }
}

impl<Prev,> MapCharacterSets<Prev,> {
    fn new(previous_phase: Prev,) -> Self {
        Self { previous_phase, }
    }
}

pub(crate) fn phase_1_map_character_sets<Prev,>(previous_phase: Prev,) -> MapCharacterSets<Prev,> {
    MapCharacterSets::new(previous_phase,)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::translation_phases::phase_0_newline_tracking::phase_0_newline_tracking;
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
    fn test_phase_1_map_character_sets(#[case] input: &str, #[case] expected: &str,) {
        use crate::util::string_cache::StringCache;
        let actual =
            phase_1_map_character_sets(phase_0_newline_tracking(input, &mut StringCache::new(),),);
        assert_eq!(actual.collect::<String>(), expected);
    }
}
