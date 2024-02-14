use thiserror::Error;

use super::{
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    SourcePosition,
    TranslationPhase,
};

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
enum EoiState {
    Base,
    HasGeneratedWarning,
    HasReturnedLast,
    Done,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct RemoveEscapedNewlines {
    last:      Option<char>,
    eoi_state: EoiState,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Error)]
#[error("missing final newline")]
pub(crate) struct MissingNewlineError(pub(crate) SourcePosition);

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Error)]
pub(crate) enum RemoveEscapedNewlinesError {
    #[error(transparent)]
    MissingFinalNewLine(MissingNewlineError),
}

impl GetSeverity for RemoveEscapedNewlinesError {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::MissingFinalNewLine(_) => ErrorSeverity::Warning,
        }
    }
}

impl GetPosition for RemoveEscapedNewlinesError {
    fn position(&self) -> SourcePosition {
        match self {
            | Self::MissingFinalNewLine(e) => e.0,
        }
    }
}

impl RemoveEscapedNewlines {
    pub(crate) fn new() -> Self {
        Self {
            last:      None,
            eoi_state: EoiState::Base,
        }
    }
}

impl TranslationPhase for RemoveEscapedNewlines {
    type Error = RemoveEscapedNewlinesError;
    type Input = char;
    type Yield = char;

    fn next_item(
        &mut self,
        input: Self::Input,
        context: &mut super::Context,
    ) -> Result<Option<Self::Yield>, Self::Error> {
        let Some(last) = self.last.take() else {
            self.last = Some(input);
            return Ok(None);
        };
        if last == '\\' && input == '\n' {
            return Ok(None);
        }
        self.last = Some(input);
        Ok(Some(last))
    }

    fn eoi(&mut self, context: &mut super::Context) -> Result<Option<Self::Yield>, Self::Error> {
        match self.last.take() {
            | Some('\n') => {
                self.eoi_state = EoiState::Done;
                Ok(Some('\n'))
            },
            | Some(c) if self.eoi_state == EoiState::Base => {
                self.eoi_state = EoiState::HasGeneratedWarning;
                self.last = Some(c);
                Err(RemoveEscapedNewlinesError::MissingFinalNewLine(
                    MissingNewlineError(context.source.position()),
                ))
            },
            | Some(c) if self.eoi_state == EoiState::HasGeneratedWarning => {
                self.eoi_state = EoiState::HasReturnedLast;
                Ok(Some(c))
            },
            | None if self.eoi_state == EoiState::HasReturnedLast => {
                self.eoi_state = EoiState::Done;
                Ok(Some('\n'))
            },
            | None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::{
        translation_phases::{
            phase_0_newline_tracking::NewlineTracking,
            phase_1_map_character_sets::MapCharacterSets,
        },
        util::string_cache::Id as StringCacheId,
    };

    #[rstest]
    #[case("", vec![Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(SourcePosition {
        index:       0,
        line:        1,
        column:      1,
        source_file: StringCacheId::from(0),
    }))), Ok('\n')])]
    #[case("a", vec![Ok('a'), Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(SourcePosition {
        index:       1,
        line:        1,
        column:      2,
        source_file: StringCacheId::from(0),
    }))), Ok('\n')])]
    #[case("??=", vec![Ok('#'), Err(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(SourcePosition {
        index:       3,
        line:        1,
        column:      4,
        source_file: StringCacheId::from(0),
    }))), Ok('\n')])]
    #[case("??=define FOO 1\\\r\n\r\n", vec![Ok('#'), Ok('d'), Ok('e'), Ok('f'), Ok('i'), Ok('n'), Ok('e'), Ok(' '), Ok('F'), Ok('O'), Ok('O'), Ok(' '), Ok('1'), Ok('\n')])]
    fn test_phase_1_and_2(
        #[case] input: &str,
        #[case] expected: Vec<Result<char, RemoveEscapedNewlinesError>>,
    ) {
        let mut actual = Vec::new();
        test_run(
            TestArgs {
                phase_2: Some(&mut actual),
                input: Box::new(input),
                name: "<input>".as_ref::<Path>().to_owned(),
                ..Default::default()
            }
        );
        assert_eq!(actual, expected);
    }
}
