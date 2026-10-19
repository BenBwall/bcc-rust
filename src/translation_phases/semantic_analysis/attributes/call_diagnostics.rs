//! GNU error/warning attributes describe calls rather than function-pointer
//! types. Validation retains the first declaration's message; lowering emits
//! diagnostics only for evaluated direct calls in emitted, reachable blocks.
//! C99: §6.7p3-4, p. 97; PDF p. 109 (declarations), extended by GNU
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>.

use super::{
    super::{
        Analyzer,
        AttributeSpecifier,
        FunctionCallDiagnostic,
        SemanticErrorKind,
    },
    entities::Subject,
    written::WrittenAttribute,
};
use crate::{
    configuration::DiagnosticGroup,
    translation_phases::SourceVectors,
    util::{
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn check_call_diagnostic(
        &mut self,
        specifier: &'tu AttributeSpecifier<'tu>,
        mut attribute: WrittenAttribute<'_>,
    ) {
        // Clang accepts parenthesized adjacent literals in this unevaluated
        // string argument. The shared decoder diagnoses encoding prefixes.
        if let Some(mut tokens) = attribute.arguments {
            while tokens.len() >= 2
                && self.context.string_cache.at(tokens[0].contents) == "("
                && self
                    .context
                    .string_cache
                    .at(tokens[tokens.len() - 1].contents)
                    == ")"
            {
                tokens = &tokens[1..tokens.len() - 1];
            }
            attribute.arguments = Some(tokens);
        }
        let text = self
            .string_argument(attribute)
            .map(|text| self.context.string_cache.intern(text));
        _ = self.attributes.call_diagnostics.insert(
            (std::ptr::from_ref(specifier).addr(), attribute.index),
            ParsedDiagnostic {
                text,
                source: attribute.source,
                warning: attribute.name == "warning",
                applied: false,
            },
        );
    }

    pub(super) fn apply_call_diagnostic(
        &mut self,
        subject: Subject,
        specifier: &'tu AttributeSpecifier<'tu>,
        attribute: WrittenAttribute<'_>,
    ) {
        let key = (std::ptr::from_ref(specifier).addr(), attribute.index);
        let Some(parsed) = self.attributes.call_diagnostics.get_mut(&key) else {
            return;
        };
        parsed.applied = true;
        let parsed = *parsed;
        let Some(text) = parsed.text else { return };
        let Subject::Function { binding, .. } = subject else {
            self.call_declaration_error(
                parsed.source,
                "'error' and 'warning' attributes only apply to functions",
                None,
            );
            return;
        };
        let entity = self.entity_of(binding);
        let previous = self
            .attributes
            .entities
            .get(&entity)
            .and_then(|a| a.call_diagnostic);
        if let Some(previous) = previous {
            if previous.warning != parsed.warning {
                self.call_declaration_error(
                    parsed.source,
                    "'error' and 'warning' attributes are not compatible",
                    Some(previous.source),
                );
            } else if previous.text != text {
                let message = self
                    .context
                    .string_cache
                    .intern("attribute is already applied with different arguments");
                self.group_diagnostic(
                    DiagnosticGroup::IgnoredAttributes,
                    SemanticErrorKind::CallAttribute,
                    parsed.source,
                    Some(message),
                    Some(previous.source),
                );
            }
            return;
        }
        if entity.0 != binding {
            self.call_declaration_error(
                parsed.source,
                "'error' or 'warning' attribute does not appear on the first declaration",
                Some(self.bindings[entity.0].name.source_vectors),
            );
            return;
        }
        let message = crate::diagnostics::format_in!(
            self.scratch,
            "call to '{}' declared with '{}' attribute: {}",
            self.context
                .string_cache
                .at(self.bindings[binding].name.name),
            if parsed.warning { "warning" } else { "error" },
            self.context.string_cache.at(text)
        );
        let message = self.context.string_cache.intern(message);
        self.attributes.entity(entity).call_diagnostic = Some(FunctionCallDiagnostic {
            message,
            text,
            source: parsed.source,
            warning: parsed.warning,
        });
    }

    pub(super) fn finish_call_diagnostics(&mut self) {
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.extend(
            self.attributes
                .call_diagnostics
                .values()
                .copied()
                .filter(|p| !p.applied && p.text.is_some()),
        );
        for parsed in pending {
            self.call_declaration_error(
                parsed.source,
                "'error' and 'warning' attributes only apply to functions",
                None,
            );
        }
    }

    fn call_declaration_error(
        &mut self,
        source: SourceVectors,
        message: &str,
        previous: Option<SourceVectors>,
    ) {
        let message = self.context.string_cache.intern(message);
        self.error(
            SemanticErrorKind::CallAttribute,
            source,
            Some(message),
            previous,
        );
    }
}

#[derive(Clone, Copy)]
pub(super) struct ParsedDiagnostic {
    text:    Option<StringCacheId>,
    source:  SourceVectors,
    warning: bool,
    applied: bool,
}
