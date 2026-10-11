use super::super::{
    Expander,
    GetPosition,
    GetSourceFileIndex,
    PreprocessorToken,
    SetPosition,
    SetSourceFileIndex,
    SourcePosition,
    SourceVector,
    TokenizerFrameType,
};
use crate::translation_phases::SourceVectors;

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
    pub(in crate::translation_phases::preprocessing) fn location_at(
        &mut self,
        position: SourcePosition,
    ) -> SourceVectors {
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
