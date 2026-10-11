//! Extension frames parse later ISO and vendor syntax on the same machine.
//! [`Parser::extension`] reports accepted syntax through the configured
//! diagnostic policy. Each extension frame owns its delimiters and pushes
//! ordinary expression, type-name, or compound children. Semantic
//! interpretation belongs to analysis.
//!
//! For `_Alignof(int)`, the modern frame owns the parentheses and pushes a
//! type-name frame for `int`. The child returns its type-name syntax, then the
//! modern frame consumes `)` and returns its operand to the expression frame.
//!
//! Read [`Parser::extension`] first for diagnostic policy, then
//! [`modern::ModernFrame::step`], [`gnu::GnuFrame::step`], or
//! [`msvc::MsvcFrame::step`] for the dialect in question.
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

use super::{
    Parser,
    syntax,
};
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
