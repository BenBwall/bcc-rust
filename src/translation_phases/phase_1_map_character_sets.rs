use thiserror::Error;

use super::{
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    SourcePosition,
    TranslationPhase,
};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct MapCharacterSets<Prev> {
    pub(crate) previous_phase: Prev,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Error)]
#[error(transparent)]
pub(crate) struct MapCharacterSetsError<PrevError> {
    inner: PrevError,
}

impl<PrevError> GetSeverity for MapCharacterSetsError<PrevError>
where
    PrevError: GetSeverity,
{
    fn severity(&self) -> ErrorSeverity {
        self.inner.severity()
    }
}

impl<PrevError> GetPosition for MapCharacterSetsError<PrevError>
where
    PrevError: GetPosition,
{
    fn position(&self) -> SourcePosition {
        self.inner.position()
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct SavePoint<Inner> {
    pub(crate) inner: Inner,
}

impl<Inner> super::SavePoint for SavePoint<Inner>
where
    Inner: super::SavePoint,
{
    fn current_position(&self) -> SourcePosition {
        self.inner.current_position()
    }
}

impl<Prev> Iterator for MapCharacterSets<Prev>
where
    Prev: TranslationPhase<Yield = char>,
{
    type Item = Result<char, MapCharacterSetsError<Prev::Error>>;

    fn next(&mut self) -> Option<Self::Item> {
        let c = match self.previous_phase.next()? {
            | Ok(c) => c,
            | Err(e) => return Some(Err(MapCharacterSetsError { inner: e })),
        };
        let save_point = self.save();
        let next = self.previous_phase.next();
        if c == '\r' && matches!(next, Some(Ok('\n',),)) {
            return Some(Ok('\n'));
        }
        if c == '\r' {
            self.restore(save_point);
            return Some(Ok('\n'));
        }
        if c != '?' {
            self.restore(save_point);
            return Some(Ok(c));
        }
        let Some(Ok(next)) = next else {
            self.restore(save_point);
            return Some(Ok(c));
        };
        if next != '?' {
            self.restore(save_point);
            return Some(Ok(c));
        }
        let Some(Ok(trigraph)) = self.previous_phase.next() else {
            self.restore(save_point);
            return Some(Ok(c));
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
                return Some(Ok(c));
            },
        };
        Some(Ok(to_yield))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.previous_phase.size_hint()
    }
}

impl<Prev> TranslationPhase for MapCharacterSets<Prev>
where
    Prev: TranslationPhase<Yield = char>,
{
    type Error = MapCharacterSetsError<Prev::Error>;
    type SavePoint = SavePoint<Prev::SavePoint>;
    type Yield = char;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner: self.previous_phase.save(),
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.previous_phase.restore(save_point.inner);
    }

    fn current_position(&self) -> SourcePosition {
        self.previous_phase.current_position()
    }
}

impl<Prev> MapCharacterSets<Prev> {
    pub(crate) fn new(previous_phase: Prev) -> Self {
        Self { previous_phase }
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
        use crate::util::string_cache::StringCache;

        let mut string_cache = StringCache::new();
        let actual = MapCharacterSets::new(NewlineTracking::new(
            input.into(),
            string_cache.intern("<input>"),
        ));
        assert_eq!(
            actual
                .map(|r| r.unwrap_or_else(|e| match e.inner {}))
                .collect::<String>(),
            expected
        );
    }
}
