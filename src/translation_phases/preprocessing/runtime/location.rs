//! Invocation, spelling, and current source locations during replacement.
//!
//! C99: translation phase 4 rescanning, §6.10.3.4 paragraph 1, p. 155; PDF p.
//! 167; predefined file and line macros, §6.10.8 paragraph 1, p. 160; PDF p.
//! 172. Locations preserve provenance for other handlers and diagnostics.

use super::{
    super::Expander,
    TokenizerFrameType,
};
use crate::{
    configuration::CompilerIdentity,
    translation_phases::{
        GetPosition,
        GetSourceFileIndex,
        SetPosition,
        SetSourceFileIndex,
        SourcePosition,
        SourceVector,
        SourceVectors,
        preprocessor_tokenizer::PreprocessorToken,
    },
};

impl Expander<'_, '_, '_, '_> {
    pub(in crate::translation_phases::preprocessing) fn invocation_location(
        &self,
        token: PreprocessorToken,
    ) -> SourceVector {
        for frame in self.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::ObjectLikeMacroInvocation { invocation, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { invocation, .. } =>
                    return invocation.clone(),
                | TokenizerFrameType::FunctionLikeMacroArgument { .. }
                | TokenizerFrameType::SourceFile { .. } => break,
                | TokenizerFrameType::Rescan { .. } | TokenizerFrameType::DeferredQuery => (),
            }
        }
        self.spelling_location(token)
    }

    /// The current source line for `__LINE__` and `__FILE__`, as GCC
    /// expands them. A token spelled in the source, an argument's included,
    /// keeps its own line. A token spelled in a replacement list belongs to
    /// the innermost macro being replaced whose name was itself spelled in
    /// the source: `T(__LINE__)` in the body of `L` is at `L`'s invocation,
    /// not on the line of `L`'s definition, and so is a macro expanded in
    /// an argument within that body (`#define C(x) ID(OBJ), x`). For GCC a
    /// name read from the source counts only when its macro was invoked
    /// there or in an argument being expanded: a name an argument supplied
    /// to a body that completes the call (`#define F(m) m()` reached as
    /// `G(\nE)` through `#define G(x) F(x)`) belongs to the outer `G`, while
    /// Clang takes that name's own line.
    /// C99: §6.10.8 paragraph 1, p. 160; PDF p. 172.
    pub(in crate::translation_phases::preprocessing) fn current_line_location(
        &self,
        token: PreprocessorToken,
    ) -> SourceVector {
        let spelling = self.spelling_location(token);
        if !self.spelled_in_definition(&spelling) {
            return spelling;
        }
        let gcc = !matches!(
            self.context.configuration.compiler_identity(),
            CompilerIdentity::Clang | CompilerIdentity::ClangCl
        );
        let frames = &self.tokenizer_stack;
        for (index, frame) in frames.iter().enumerate().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::ObjectLikeMacroInvocation { spelling, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { spelling, .. } =>
                    if !self.spelled_in_definition(spelling)
                        && (!gcc || Self::invoked_from_source(&frames[..index]))
                    {
                        return spelling.clone();
                    },
                | TokenizerFrameType::SourceFile { .. } => break,
                | TokenizerFrameType::FunctionLikeMacroArgument { .. }
                | TokenizerFrameType::Rescan { .. }
                | TokenizerFrameType::DeferredQuery => (),
            }
        }
        self.invocation_location(token)
    }

    /// Whether a macro whose frame lies on `below` was invoked while the
    /// source itself or an argument's own tokens were read, rather than a
    /// replacement list or an argument substituted into one.
    fn invoked_from_source(below: &[super::TokenizerFrame<'_>]) -> bool {
        below
            .iter()
            .rev()
            .find(|frame| {
                !matches!(
                    frame.frame_type,
                    TokenizerFrameType::DeferredQuery
                        | TokenizerFrameType::Rescan { argument: false }
                )
            })
            .is_none_or(|frame| {
                matches!(
                    frame.frame_type,
                    TokenizerFrameType::SourceFile { .. }
                        | TokenizerFrameType::FunctionLikeMacroArgument { .. }
                )
            })
    }

    /// Whether `location` lies in a `#define` directive, its replacement
    /// list or parameters, including lines its backslash-newlines join
    /// (C99 §5.1.1.2p1 item 2). Text not retained as a source file, such as
    /// a command-line definition, counts as a definition.
    /// C99: §6.10.3 paragraph 1, p. 151; PDF p. 163.
    fn spelled_in_definition(&self, location: &SourceVector) -> bool {
        let Some(text) = self.context.source_text(location.source_file_index) else {
            return true;
        };
        let bytes = text.as_bytes();
        let mut start = (location.index as usize).min(bytes.len());
        // Back to the start of the logical line.
        loop {
            let Some(newline) = bytes[..start].iter().rposition(|&byte| byte == b'\n') else {
                start = 0;
                break;
            };
            let mut end = newline;
            if end > 0 && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            if end > 0 && bytes[end - 1] == b'\\' {
                start = end - 1;
            } else {
                start = newline + 1;
                break;
            }
        }
        let line = text[start..].trim_start_matches([' ', '\t']);
        line.strip_prefix('#')
            .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with("define"))
    }

    pub(in crate::translation_phases::preprocessing) fn expansion_end(
        &self,
    ) -> Option<SourceVector> {
        for frame in self.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::ObjectLikeMacroInvocation { invocation_end, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { invocation_end, .. } =>
                    return Some(invocation_end.clone()),
                | TokenizerFrameType::FunctionLikeMacroArgument { .. }
                | TokenizerFrameType::SourceFile { .. } => return None,
                | TokenizerFrameType::Rescan { .. } | TokenizerFrameType::DeferredQuery => (),
            }
        }
        None
    }

    /// C99: §6.10.3.4p1, p. 155; PDF p. 167: retain the name's spelling
    /// separately from its invocation through nested rescanning.
    pub(in crate::translation_phases::preprocessing) fn spelling_location(
        &self,
        token: PreprocessorToken,
    ) -> SourceVector {
        self.context
            .first_source_vector_or_default(token.source_vectors)
    }

    pub(in crate::translation_phases::preprocessing) fn physical_source_file_index(&self) -> u32 {
        self.tokenizer_stack
            .iter()
            .rev()
            .find_map(|frame| match frame.frame_type {
                | TokenizerFrameType::SourceFile {
                    physical_source_file_index,
                    ..
                } => Some(physical_source_file_index),
                | _ => None,
            })
            .unwrap_or_else(|| self.source_file_index())
    }

    /// A zero-length diagnostic location at the current input position.
    pub(in crate::translation_phases::preprocessing) fn current_location(
        &mut self,
    ) -> SourceVectors {
        self.location_at(self.position())
    }

    /// A zero-length diagnostic location at `position` of the current token
    /// source.
    pub(super) fn location_at(&mut self, position: SourcePosition) -> SourceVectors {
        self.tokenizer.location_at(self.context, position)
    }
}

impl Expander<'_, '_, '_, '_> {
    /// The current position of the current token source.
    #[inline(always)]
    pub(in crate::translation_phases::preprocessing) fn position(&self) -> SourcePosition {
        self.tokenizer.position(self.context)
    }

    /// Moves the current token source to `position`.
    #[inline(always)]
    pub(in crate::translation_phases::preprocessing) fn set_position(
        &mut self,
        position: SourcePosition,
    ) {
        self.tokenizer.set_position(position);
    }

    /// Moves the current token source to `line`, keeping its index and
    /// column.
    pub(in crate::translation_phases::preprocessing) fn set_line(&mut self, line: u32) {
        let position = self.position();
        self.set_position(SourcePosition { line, ..position });
    }

    /// Attributes the current token source to another source file.
    pub(in crate::translation_phases::preprocessing) fn set_source_file_index(
        &mut self,
        source_file_index: u32,
    ) {
        self.tokenizer.set_source_file_index(source_file_index);
    }
}

impl GetSourceFileIndex for Expander<'_, '_, '_, '_> {
    #[inline(always)]
    fn source_file_index(&self) -> u32 {
        self.tokenizer.source_file_index()
    }
}
