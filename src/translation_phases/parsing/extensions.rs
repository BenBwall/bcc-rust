//! Syntax and diagnostic policy for later ISO and vendor extensions.
//! [`Parser::extension`] reports accepted syntax through the configured
//! diagnostic policy. The extension productions in `grammar/modern.rs`,
//! `grammar/gnu.rs`, and `grammar/msvc.rs` own their delimiters and call the
//! ordinary expression, type-name, and compound productions; the syntax they
//! build is defined here. Semantic interpretation belongs to analysis.
//!
//! For `_Alignof(int)`, the ISO construct production owns the parentheses and
//! calls the type-name production for `int`. It then consumes `)` and returns
//! the [`modern::SyntaxOperand`] that the expression production wraps.
//!
//! Read [`Parser::extension`] first for diagnostic policy, then the syntax of
//! the dialect in question.
//!
//! Files under `extensions/` are grouped by origin:
//!
//! - `modern.rs`: later ISO operands, generic selections, assertions, and
//!   attribute specifiers (including their GNU and MSVC spellings).
//! - `gnu.rs`: GNU assembly, builtins, and local labels.
//! - `msvc.rs`: Microsoft SEH and inline assembly.
//!
//! C99: translation phase 7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//! C99: extensions, §4 paragraph 6, p. 7; PDF p. 19; diagnostics, §5.1.1.3,
//! p. 11; PDF p. 23. Array-parameter syntax is §6.7.5 paragraph 1, p. 114;
//! PDF p. 126; its availability before C99 is handled by the same policy.

// Later ISO syntax
pub(crate) mod modern;

// Vendor syntax
pub(crate) mod gnu;
pub(crate) mod msvc;

use super::Parser;
use crate::translation_phases::{
    SourceVectors,
    preprocessing::Token,
};

impl Parser<'_, '_, '_> {
    /// Reports a syntax feature through the shared mode policy.
    /// C99: §5.1.1.3, p. 11; PDF p. 23. Later ISO syntax is an extension.
    pub(super) fn extension(
        &mut self,
        feature: crate::configuration::Feature,
        spelling: &'static str,
        token: Token,
    ) {
        self.extension_source(feature, spelling, token.source_vectors);
    }

    /// Out of line so the recursive productions that call it keep small
    /// native frames.
    #[inline(never)]
    pub(super) fn extension_source(
        &mut self,
        feature: crate::configuration::Feature,
        spelling: &'static str,
        source: SourceVectors,
    ) {
        if self.pedantic_suppression == 0 {
            self.context.report_extension(feature, spelling, source);
        }
    }

    /// Reports the C99 array-parameter syntax feature.
    /// C99: array declarators §6.7.5 paragraph 1, p. 114; PDF p. 126.
    pub(super) fn c99_syntax_extension(&mut self, spelling: &'static str, token: Token) {
        if self.pedantic_suppression == 0 {
            self.context.report_extension(
                crate::configuration::Feature::ArrayParameterSyntax,
                spelling,
                token.source_vectors,
            );
        }
    }
}
