//! Queues phase errors and keeps their source ranges valid until reporting.
//! Tokenizer errors can be withdrawn when include handling has interpreted
//! the same source as a quoted header name.
//!
//! C99: required diagnostics, §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
//! Error creation remains the responsibility of each translation phase.

use super::{
    Context,
    ErrorSeverity,
    GetSeverity,
    InitialProcessorError,
    ParserError,
    PreprocessorError,
    PreprocessorTokenizerError,
    SourceVector,
    TranslationError,
};

impl<'tu> Context<'tu> {
    #[cold]
    #[inline(never)]
    pub(crate) fn parser_error(&mut self, error: ParserError<'tu>) {
        if self.withholds(error.severity, false, error.source_vectors) {
            return;
        }
        self.pending_errors
            .push_back(TranslationError::Parsing(error));
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn preprocessor_error(&mut self, error: PreprocessorError<'tu>) {
        if self.withholds(
            error.severity(),
            error.error_type.is_extension(),
            error.source_vectors,
        ) {
            return;
        }
        self.pending_errors
            .push_back(TranslationError::Preprocessing(error));
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn preprocessor_tokenizer_error(&mut self, error: PreprocessorTokenizerError) {
        if !self.ignore_tokenizer_errors() {
            self.pending_errors
                .push_back(TranslationError::PreprocessorTokenizining(error));
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn raw_preprocessor_error(
        self_pending_errors: &mut impl Extend<TranslationError<'tu>>,
        error: PreprocessorError<'tu>,
    ) {
        self_pending_errors.extend([TranslationError::Preprocessing(error)]);
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn pop_pending_error(&mut self) -> Option<TranslationError<'tu>> {
        loop {
            let error = self.pending_errors.pop_front()?;
            self.relocated_errors = self.relocated_errors.saturating_sub(1);
            if matches!(&error, TranslationError::Extension(x) if x.suppressed.get()) {
                self.suppressed_errors -= 1;
            } else {
                self.drained_errors += usize::from(error.severity() == ErrorSeverity::Error);
                return Some(error);
            }
        }
    }

    pub(crate) fn ignore_tokenizer_errors(&self) -> bool {
        self.ignore_tokenizer_errors
    }

    pub(crate) fn set_ignore_tokenizer_errors(&mut self, value: bool) {
        self.ignore_tokenizer_errors = value;
    }

    #[inline(always)]
    pub(crate) fn missing_final_newline(&mut self, vector: SourceVector) {
        if !self.ignore_tokenizer_errors() && !self.in_system_header(&vector) {
            self.pending_errors
                .push_back(TranslationError::InitialProcessing(
                    InitialProcessorError::MissingFinalNewline(vector),
                ));
        }
    }

    pub(crate) fn escaped_final_newline(&mut self, vector: SourceVector) {
        if !self.ignore_tokenizer_errors() && !self.in_system_header(&vector) {
            self.pending_errors
                .push_back(TranslationError::InitialProcessing(
                    InitialProcessorError::EscapedFinalNewline(vector),
                ));
        }
    }

    /// The quoted include extension treats the first quote after a backslash
    /// as the header delimiter, even though phase 3 lexed it as an escape.
    pub(crate) fn withdraw_quoted_header_lexer_error(&mut self, source: &SourceVector) {
        self.pending_errors.retain(|error| {
            !matches!(error, TranslationError::PreprocessorTokenizining(error) if error.is_unclosed_header_string_at(source))
        });
        // Retaining can remove a diagnostic from the relocated prefix.
        // Rechecking already-retained ranges during the next compaction is
        // safe.
        self.relocated_errors = 0;
    }

    /// Error-severity diagnostics reported in every phase so far, whether
    /// still pending or already taken by a reporter; warnings are excluded.
    /// Withdrawn and suppressed diagnostics do not count, so the result can
    /// fall when a phase withdraws one. Code generation must not start while
    /// it is nonzero.
    /// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    pub(crate) fn error_count(&self) -> usize {
        self.drained_errors
            + self
                .pending_errors
                .iter()
                .filter(|error| {
                    !matches!(error, TranslationError::Extension(x) if x.suppressed.get())
                        && error.severity() == ErrorSeverity::Error
                })
                .count()
    }

    pub(crate) fn pending_error_count(&self) -> usize {
        if self.pending_errors.is_empty() {
            return 0;
        }
        self.pending_errors.len() - self.suppressed_errors
    }

    /// Marks one arena-stable occurrence without scanning the diagnostic FIFO.
    pub(crate) fn suppress_extension(&mut self, marker: &std::cell::Cell<bool>) {
        if !marker.replace(true) {
            self.suppressed_errors += 1;
        }
    }

    #[cfg(test)]
    #[expect(
        clippy::disallowed_types,
        reason = "Test-only owned list of the pending errors, compiled only under `cfg(test)`."
    )]
    pub(crate) fn take_pending_errors(&mut self) -> Vec<TranslationError<'tu>> {
        self.relocated_errors = 0;
        std::iter::from_fn(|| self.pop_pending_error()).collect()
    }

    /// Removes and yields the pending errors after the first `keep`, in
    /// order.
    ///
    /// `keep` is a raw queue position, and callers take it from
    /// [`Self::pending_error_count`], which excludes suppressed entries.
    /// The two agree because only phase 7 suppresses diagnostics, and it
    /// starts after phases 4-6 have preprocessed the whole translation unit
    /// into a fresh context. A split with suppressed entries pending would
    /// land too early and could drop one without updating the count.
    pub(crate) fn split_off_pending_errors(
        &mut self,
        keep: usize,
    ) -> impl Iterator<Item = TranslationError<'tu>> + '_ {
        debug_assert_eq!(
            self.suppressed_errors, 0,
            "pending-error splits happen only before parsing suppresses diagnostics"
        );
        self.relocated_errors = self.relocated_errors.min(keep);
        self.pending_errors.split_off(keep)
    }

    pub(crate) fn append_pending_errors(
        &mut self,
        errors: impl IntoIterator<Item = TranslationError<'tu>>,
    ) {
        self.pending_errors.extend(errors);
    }
}
