pub(crate) mod gnu;

pub(crate) mod modern;

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
