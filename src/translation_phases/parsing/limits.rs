//! Parser resource ceilings and terminal failure cleanup.
//!
//! [`Parser::resource_failure`] emits one resource diagnostic and ends parsing.
//! [`ParserLimits`] sets ceilings on roots, nodes, frames, and source segments.
//! The defaults are representation bounds rather than fixed grammar limits.
//! Memory exhaustion outside these counters is not handled here.
//!
//! C99: translation phase 7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//! C99: translation limits, §5.2.4.1, pp. 20-21; PDF pp. 32-33; footnote 13,
//! p. 20; PDF p. 32 asks implementations to avoid fixed limits. Resource
//! diagnostics use §5.1.1.3, p. 11; PDF p. 23.

use super::{
    ExternalDeclaration,
    Parser,
    errors::{
        ParserErrorType,
        ParserResource,
    },
};
use crate::translation_phases::{
    GetSourceFileIndex,
    preprocessing::Token,
};

impl<'tu> Parser<'_, 'tu, '_> {
    pub(super) fn resource_failure(
        &mut self,
        resource: ParserResource,
        limit: usize,
    ) -> Option<ExternalDeclaration<'tu>> {
        self.resource_failure_at(resource, limit, None)
    }

    pub(super) fn resource_failure_at(
        &mut self,
        resource: ParserResource,
        limit: usize,
        token_override: Option<Token>,
    ) -> Option<ExternalDeclaration<'tu>> {
        if self.resource_limit_reported {
            return None;
        }
        self.resource_limit_reported = true;
        self.has_external_declaration = true;
        let token = token_override.or_else(|| self.cursor.current());
        let source_vectors = token.map_or_else(
            || {
                self.context.create_retained_source_vectors(
                    self.position(),
                    self.source_file_index(),
                    0,
                )
            },
            |token| token.source_vectors,
        );
        self.report(
            ParserErrorType::ResourceLimitExceeded { resource, limit },
            token,
        );
        self.cursor.abandon();
        while let Some(frame) = self.frames.pop() {
            frame.reclaim_pooled(&mut self.pools);
        }
        self.retained_frame_nodes = 0;
        self.returned = None;
        self.recovery.abandon();
        self.scopes.restore_depth(0);
        self.scopes.clear_retained_bindings();
        self.label_scopes.exit_all();
        self.switch_scopes.clear();
        self.switch_floor = 0;
        self.pedantic_suppression = 0;
        Some(ExternalDeclaration::Error(source_vectors))
    }
}

/// Catchable ceilings on parser resources.
///
/// C99: §5.2.4.1, pp. 20-21; PDF pp. 32-33 sets only minimums, and its
/// footnote 13, p. 20; PDF p. 32 asks implementations to avoid fixed
/// translation limits. The defaults are therefore representation bounds,
/// not grammar limits.
#[derive(Debug, Clone, Copy)]
pub(super) struct ParserLimits {
    pub(super) external_declarations: usize,
    pub(super) syntax_nodes:          usize,
    pub(super) frame_depth:           usize,
    pub(super) source_segments:       usize,
}

impl Default for ParserLimits {
    fn default() -> Self {
        Self {
            // These counters and their backing collections use `usize`.
            // There is no smaller grammar or representation limit.
            external_declarations: usize::MAX,
            syntax_nodes:          usize::MAX,
            // Scope bindings store their nesting depth in a `u32`.
            frame_depth:           u32::MAX as usize,
            // Each source arena checks its own `u32` index space. Their
            // combined count has no smaller representation limit than usize.
            source_segments:       usize::MAX,
        }
    }
}
