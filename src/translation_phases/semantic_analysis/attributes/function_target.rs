//! GCC's and Clang's x86 `target` attribute on a function: the features its
//! code may use, read over the command line's (`target/x86_features.rs`).
//! Each compiler checks the arguments and the text in its own words: Clang
//! takes one string literal and ignores, with a warning, an attribute whose
//! text names a feature or processor it does not know; GCC takes one or
//! more strings, lists parted by commas, and refuses such a text. Each merges
//! the attribute over an entity's declarations its own way too: Clang keeps a
//! declaration's first `target`, which every later declaration inherits, and
//! takes another text on a redeclaration for multiversioning, which bcc
//! refuses; GCC reads every `target` of a declaration in order, and a later
//! declaration's replace an earlier one's, even after the definition.
//! Lowering carries the result to the IR function, the back ends to the code,
//! and calls of `always_inline` functions are checked against it there,
//! where only emitted code is (`lowering/target_features.rs`).
//! GNU extension: GCC manual, "x86 Function Attributes", `target`.
//! <https://gcc.gnu.org/onlinedocs/gcc/x86-Function-Attributes.html>
//! Clang extension: Clang Attributes Reference, "target".
//! <https://clang.llvm.org/docs/AttributeReference.html#target>
//! C99: §6.7, p. 97; PDF p. 109 (declarations, extended by GNU attributes);
//! §6.2.2p2, p. 30; PDF p. 42 (linked declarations share one entity).

use super::{
    entities::Subject,
    written::WrittenAttribute,
};
use crate::{
    configuration::DiagnosticGroup,
    diagnostics::format_in,
    target::target_attribute_faults,
    translation_phases::{
        SourceVectors,
        preprocessing::{
            LiteralEncoding,
            StringTokenType,
            Token,
            TokenType,
        },
        semantic_analysis::{
            Analyzer,
            SemanticErrorKind,
            UnsupportedAttribute,
        },
    },
};

/// The declaration that last wrote an entity's `target`, and its text, as
/// Clang compares a redeclaration's.
#[derive(Debug, Clone, Copy)]
pub(super) struct TargetWriter<'s> {
    binding: usize,
    text:    &'s str,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Applies `target` to `subject`.
    pub(super) fn target_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        let gcc = self.gcc_identity();
        // GCC checks only the number of arguments of an attribute it ignores
        // on what is not a function.
        if gcc && !matches!(subject, Subject::Function { .. }) {
            if attribute
                .arguments
                .is_some_and(|arguments| !arguments.is_empty())
            {
                self.ignored(attribute);
            } else {
                self.target_refusal(
                    "wrong number of arguments specified for 'target' attribute",
                    attribute.source,
                );
            }
            return;
        }
        let Some((text, literal)) = self.target_text(attribute, gcc) else {
            return;
        };
        let Subject::Function { binding, .. } = subject else {
            self.target_refusal(
                "'target' attribute only applies to functions",
                attribute.source,
            );
            return;
        };
        // The default version of a multiversioned function: bcc has no
        // multiversioning, and drops the attribute with a warning.
        if text == "default" {
            self.ignored(attribute);
            return;
        }
        if gcc && text.is_empty() {
            self.target_warning(
                DiagnosticGroup::GccAttributes,
                "empty string in attribute 'target'",
                attribute.source,
            );
            return;
        }
        let mut valid = true;
        for fault in target_attribute_faults(text, !gcc) {
            valid = false;
            let message = format_in!(self.scratch, "{fault}");
            if fault.is_error() {
                self.target_refusal(message, attribute.source);
            } else {
                self.target_warning(DiagnosticGroup::IgnoredAttributes, message, literal);
            }
        }
        if !valid {
            return;
        }
        let entity = self.entity_of(binding);
        let command_line = self.context.configuration.code_options().x86_features;
        let written = self.attributes.targets.get(&entity).copied();
        let target = match written {
            // GCC reads a declaration's every `target` in turn; Clang keeps
            // the first.
            | Some(written) if written.binding == binding => {
                if !gcc {
                    return;
                }
                let previous = self.attributes.entity(entity).target;
                previous.map_or_else(
                    || command_line.target_attribute(text),
                    |previous| previous.extended(text),
                )
            },
            // Clang's multiversioning: another text declares another version.
            | Some(written) if !gcc && written.text != text => {
                self.refuse(UnsupportedAttribute::Multiversioning, attribute.source);
                return;
            },
            | Some(_) if !gcc => return,
            | _ => command_line.target_attribute(text),
        };
        let text = self.scratch.alloc_str(text);
        _ = self
            .attributes
            .targets
            .insert(entity, TargetWriter { binding, text });
        self.attributes.entity(entity).target = Some(self.types.tu.alloc(target));
    }

    /// The text of a `target` attribute's arguments and where its first
    /// string stands, or the compiler's refusal: Clang takes one string
    /// literal, GCC one or more, read as one list.
    fn target_text(
        &mut self,
        attribute: WrittenAttribute<'_>,
        gcc: bool,
    ) -> Option<(&'tu str, SourceVectors)> {
        let tokens = attribute.arguments.unwrap_or_default();
        let count = attribute.argument_count(self.context);
        if count == 0 || (!gcc && count > 1) {
            self.target_refusal(
                if gcc {
                    "wrong number of arguments specified for 'target' attribute"
                } else {
                    "'target' attribute takes one argument"
                },
                attribute.source,
            );
            return None;
        }
        let mut text = None::<&str>;
        let mut prefixed = None;
        for argument in tokens.split(|token| self.is_comma(token)) {
            let mut part = None::<&str>;
            for token in argument {
                // Both compilers read an encoding-prefixed string as its
                // characters; Clang warns that the prefix has no effect.
                let decoded = match token.kind {
                    | TokenType::String(StringTokenType::String(id)) =>
                        self.context.literal_text_in(self.scratch, id, false),
                    | TokenType::String(kind) => {
                        let (id, wide) = match kind {
                            | StringTokenType::WideString(id) => (id, true),
                            | StringTokenType::EncodedString(id, encoding) =>
                                (id, encoding != LiteralEncoding::Utf8),
                            | StringTokenType::String(id) => (id, false),
                        };
                        _ = prefixed.get_or_insert(token.source_vectors);
                        self.context.literal_text_in(self.scratch, id, wide)
                    },
                    | _ => None,
                };
                let Some(decoded) = decoded else {
                    self.target_refusal(
                        if gcc {
                            "attribute 'target' argument is not a string"
                        } else {
                            "expected string literal as argument of 'target' attribute"
                        },
                        if gcc {
                            attribute.source
                        } else {
                            token.source_vectors
                        },
                    );
                    return None;
                };
                part =
                    Some(part.map_or(decoded, |part| format_in!(self.scratch, "{part}{decoded}")));
            }
            let part = part.unwrap_or_default();
            text = Some(text.map_or(part, |text| format_in!(self.scratch, "{text},{part}")));
        }
        if let Some(source) = prefixed {
            self.error(
                SemanticErrorKind::UnevaluatedStringPrefix,
                source,
                None,
                None,
            );
        }
        let literal = tokens
            .first()
            .map_or(attribute.source, |token| token.source_vectors);
        Some((self.types.tu.alloc_str(text.unwrap_or_default()), literal))
    }

    /// Whether `token` is a comma that parts attribute arguments. A string
    /// argument holds no parenthesis, so any comma between strings does.
    fn is_comma(&self, token: &Token) -> bool {
        self.context.string_cache.at(token.contents) == ","
    }

    /// What one compiler refuses of a `target` attribute, in its words.
    fn target_refusal(&mut self, text: &str, source: SourceVectors) {
        let text = self.context.string_cache.intern(text);
        self.error(
            SemanticErrorKind::InvalidTargetAttribute,
            source,
            Some(text),
            None,
        );
    }

    /// What one compiler warns of a `target` attribute, in its words, under
    /// its `group`.
    fn target_warning(&mut self, group: DiagnosticGroup, text: &str, source: SourceVectors) {
        let text = self.context.string_cache.intern(text);
        self.group_diagnostic(
            group,
            SemanticErrorKind::TargetAttribute(group),
            source,
            Some(text),
            None,
        );
    }
}
