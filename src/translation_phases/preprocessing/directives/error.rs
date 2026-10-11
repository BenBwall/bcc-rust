use super::{
    ArenaString,
    Expander,
    PreprocessorError,
    PreprocessorErrorType,
    PreprocessorToken,
    PreprocessorTokenType,
    TranslationPhase,
};

impl Expander<'_, '_, '_, '_> {
    /// Reports an error whose message includes the directive's tokens, which
    /// are not macro-replaced; translation then does not succeed.
    ///
    /// C99: §6.10.5 paragraph 1, p. 159; PDF p. 171, and §4 paragraph 4,
    /// p. 7; PDF p. 19.
    #[cold]
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn parse_error_directive(
        &mut self,
        directive: PreprocessorToken,
    ) {
        let mut contents = ArenaString::new_in(self.scratch);
        // A directive ending at end of file is complete; the missing final
        // newline is diagnosed on its own.
        while let Some(token) = self.tokenizer.next_item(self.context) {
            if token.kind == PreprocessorTokenType::Newline {
                break;
            }
            contents.push_str(
                self.context
                    .string_cache
                    .at(token.contents)
                    .trim_end_matches('\0'),
            );
        }
        self.context.preprocessor_error(PreprocessorError {
            error_type:     if self.context.string_cache.at(directive.contents) == "warning" {
                PreprocessorErrorType::WarningDirective(self.context.diagnostic_text(&contents))
            } else {
                PreprocessorErrorType::ErrorDirective(self.context.diagnostic_text(&contents))
            },
            source_vectors: directive.source_vectors,
        });
    }
}
