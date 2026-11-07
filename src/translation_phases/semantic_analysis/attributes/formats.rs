//! GNU `format` and `format_arg` on a function declaration: which parameter
//! is a format string, which archetype reads it and where its data
//! arguments begin, retained for the checks `format.rs` makes at calls.
//! Both compilers take the operands as integer constants; bcc reads them as
//! written, an archetype name and two numbers, and keeps nothing from
//! operands it cannot read.
//! GNU extension: GCC manual, "Common Function Attributes", `format` and
//! `format_arg`. C99: §7.19.6.1, pp. 274-281; PDF pp. 286-293.

use super::{
    super::{
        Analyzer,
        SemanticErrorKind,
        TypeKind,
        facts::{
            FormatAttribute,
            FormatKind,
        },
    },
    entities::Subject,
    written::WrittenAttribute,
};
use crate::{
    configuration::DiagnosticGroup as G,
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        preprocessing::{
            Token,
            TokenType,
        },
    },
};

impl Analyzer<'_, '_, '_> {
    /// What GNU `format` and `format_arg` say of function `binding`.
    pub(in crate::translation_phases::semantic_analysis) fn format_attributes(
        &mut self,
        binding: usize,
    ) -> (Option<FormatAttribute>, u32) {
        let entity = self.entity_of(binding);
        let attributes = self.attributes.entity(entity);
        (attributes.format, attributes.format_arg)
    }

    /// Checks and retains `format(archetype, string-index, first-to-check)`
    /// on a function, as each compiler checks it: GCC reports at the
    /// declaration, Clang at the attribute.
    pub(super) fn format_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        let Subject::Function { binding, .. } = subject else {
            return;
        };
        let gcc = self.context.configuration.compiler_identity()
            == crate::configuration::CompilerIdentity::Gcc;
        let site = if gcc {
            self.attributes
                .declaration_start
                .unwrap_or(attribute.source)
        } else {
            attribute.source
        };
        let (count, operands) = self.format_operands(attribute.arguments);
        if count != 3 {
            if gcc {
                let words = "wrong number of arguments specified for 'format' attribute";
                let name = self.context.string_cache.intern(words);
                let count = u8::try_from(count).unwrap_or(u8::MAX);
                self.error(
                    SemanticErrorKind::FormatAttributeCount(count),
                    site,
                    Some(name),
                    Some(site),
                );
            } else {
                self.format_attribute_error(
                    site,
                    "'format' attribute requires exactly 3 arguments",
                );
            }
            return;
        }
        let [archetype] = operands[0] else {
            self.format_attribute_error(
                site,
                if gcc {
                    "unrecognized format specifier"
                } else {
                    "'format' attribute requires parameter 1 to be an identifier"
                },
            );
            return;
        };
        if !matches!(
            archetype.kind,
            TokenType::Identifier | TokenType::Keyword(_)
        ) {
            self.format_attribute_error(
                site,
                if gcc {
                    "unrecognized format specifier"
                } else {
                    "'format' attribute requires parameter 1 to be an identifier"
                },
            );
            return;
        }
        let name = self
            .context
            .string_cache
            .at(archetype.contents)
            .trim_end_matches('\0')
            .trim_matches('_');
        let Some(kind) = archetype_kind(name, gcc) else {
            let words = if gcc {
                format_in!(
                    self.scratch,
                    "'{name}' is an unrecognized format function type"
                )
            } else {
                format_in!(
                    self.scratch,
                    "'format' attribute argument not supported: {name}"
                )
            };
            let group = if gcc { G::Format } else { G::IgnoredAttributes };
            self.format_attribute_warning(group, words, site);
            return;
        };
        let ([format_token], [first_token]) = (operands[1], operands[2]) else {
            return;
        };
        let (Some(format), Some(first)) = (
            self.token_number(*format_token),
            self.token_number(*first_token),
        ) else {
            return;
        };
        let (format_text, first_text) = (format, first);
        let TypeKind::Function {
            parameters,
            variadic,
            prototype: true,
            ..
        } = self.types.nodes[self.bindings[binding].ty.index]
        else {
            return;
        };
        let count = u32::try_from(parameters.len()).unwrap_or(u32::MAX);
        if gcc {
            if format == 0 {
                let words = format_in!(
                    self.scratch,
                    "'format' attribute argument 2 value '{format_text}' does not refer to a \
                     function parameter"
                );
                self.format_attribute_warning(G::GccAttributes, words, site);
                return;
            }
            if format > count {
                let words = format_in!(
                    self.scratch,
                    "'format' attribute argument 2 value '{format_text}' exceeds the number of \
                     function parameters {count}"
                );
                self.format_attribute_warning(G::GccAttributes, words, site);
                return;
            }
            let parameter = parameters[format as usize - 1];
            if !self
                .pointer_target(parameter)
                .is_some_and(|target| self.character_type(target, true))
            {
                let ty = self.type_spelling(parameter, true).unwrap_or_default();
                let words = format_in!(
                    self.scratch,
                    "'format' attribute argument 2 value '{format_text}' refers to parameter type \
                     '{ty}'"
                );
                self.format_attribute_error(site, words);
                return;
            }
            if first != 0 && first <= count {
                let words = format_in!(
                    self.scratch,
                    "'format' attribute argument 3 value '{first_text}' does not refer to a \
                     variable argument list"
                );
                self.format_attribute_error(site, words);
                return;
            }
            if first != 0 && (!variadic || first != count + 1) {
                self.format_attribute_error(site, "argument to be formatted is not '...'");
                return;
            }
        } else {
            if format < 1 || format > count {
                self.format_attribute_error(
                    site,
                    "'format' attribute parameter 2 is out of bounds",
                );
                return;
            }
            let parameter = parameters[format as usize - 1];
            if !self
                .pointer_target(parameter)
                .is_some_and(|target| self.character_type(target, false))
            {
                self.format_attribute_error(site, "format argument not a string type");
                return;
            }
            if first != 0 {
                if variadic {
                    if first != count + 1 {
                        self.format_attribute_error(
                            site,
                            "'format' attribute parameter 3 is out of bounds",
                        );
                        return;
                    }
                } else {
                    let name = self.bindings[binding].name.source_vectors;
                    self.format_attribute_warning(
                        G::GccCompat,
                        "GCC requires a function with the 'format' attribute to be variadic",
                        name,
                    );
                    if first <= format || first > count {
                        self.format_attribute_error(
                            site,
                            "'format' attribute parameter 3 is out of bounds",
                        );
                        return;
                    }
                }
            }
        }
        let entity = self.entity_of(binding);
        self.attributes.entity(entity).format = Some(FormatAttribute {
            kind,
            format,
            first,
        });
    }

    /// How many comma-separated operands an attribute has, and the tokens
    /// of its first three.
    fn format_operands<'t>(&self, arguments: Option<&'t [Token]>) -> (usize, [&'t [Token]; 3]) {
        let mut operands: [&'t [Token]; 3] = [&[]; 3];
        let Some(tokens) = arguments.filter(|tokens| !tokens.is_empty()) else {
            return (0, operands);
        };
        let mut depth = 0usize;
        let mut start = 0;
        let mut count = 0;
        for (index, token) in tokens.iter().enumerate() {
            match self.context.string_cache.at(token.contents) {
                | "(" | "[" | "{" => depth += 1,
                | ")" | "]" | "}" => depth = depth.saturating_sub(1),
                | "," if depth == 0 => {
                    if let Some(operand) = operands.get_mut(count) {
                        *operand = &tokens[start..index];
                    }
                    count += 1;
                    start = index + 1;
                },
                | _ => {},
            }
        }
        if let Some(operand) = operands.get_mut(count) {
            *operand = &tokens[start..];
        }
        (count + 1, operands)
    }

    fn format_attribute_error(&mut self, site: SourceVectors, words: &str) {
        let name = self.context.string_cache.intern(words);
        self.error(SemanticErrorKind::FormatAttribute, site, Some(name), None);
    }

    fn format_attribute_warning(&mut self, group: G, words: &str, site: SourceVectors) {
        let name = self.context.string_cache.intern(words);
        self.group_diagnostic(
            group,
            SemanticErrorKind::Format(group, super::super::format::FormatNote::DefinedHere),
            site,
            Some(name),
            None,
        );
    }

    /// Gives function `binding` the `format` attribute Clang suggests for
    /// it, as Clang adds the attribute it diagnoses as missing.
    pub(in crate::translation_phases::semantic_analysis) fn infer_format_attribute(
        &mut self,
        binding: usize,
        kind: FormatKind,
        format: usize,
        first: usize,
    ) {
        let (Ok(format), Ok(first)) = (u32::try_from(format), u32::try_from(first)) else {
            return;
        };
        let entity = self.entity_of(binding);
        self.attributes.entity(entity).format = Some(FormatAttribute {
            kind,
            format,
            first,
        });
    }

    /// Retains `format_arg(string-index)` on a function whose parameter
    /// there is a character pointer.
    pub(super) fn format_arg_attribute(
        &mut self,
        subject: Subject,
        attribute: WrittenAttribute<'_>,
    ) {
        let Subject::Function { binding, .. } = subject else {
            return;
        };
        let Some(tokens) = attribute.arguments else {
            return;
        };
        let [token] = tokens else {
            return;
        };
        let Some(index) = self.token_number(*token) else {
            return;
        };
        let TypeKind::Function {
            parameters,
            prototype: true,
            ..
        } = self.types.nodes[self.bindings[binding].ty.index]
        else {
            return;
        };
        let Some(&parameter) = (index as usize)
            .checked_sub(1)
            .and_then(|position| parameters.get(position))
        else {
            return;
        };
        if self
            .pointer_target(parameter)
            .is_some_and(|target| self.character_type(target, false))
        {
            let entity = self.entity_of(binding);
            if index <= 32 {
                self.attributes.entity(entity).format_arg |= 1 << (index - 1);
            }
        }
    }

    /// Whether `ty` is `char`, or with `plain` false `signed char` or
    /// `unsigned char` too, however qualified.
    fn character_type(&self, ty: super::super::TypeId, plain: bool) -> bool {
        use super::super::Scalar as S;
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(S::Char) => true,
            | TypeKind::Scalar(S::SignedChar | S::UnsignedChar) => !plain,
            | _ => false,
        }
    }

    /// An integer constant token's value, without a suffix.
    fn token_number(&self, token: Token) -> Option<u32> {
        if !matches!(token.kind, TokenType::Integer(_)) {
            return None;
        }
        let text = self.context.string_cache.at(token.contents);
        let digits = text.trim_end_matches(['\0', 'u', 'U', 'l', 'L']);
        if let Some(hex) = digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"))
        {
            u32::from_str_radix(hex, 16).ok()
        } else if digits.len() > 1 && digits.starts_with('0') {
            u32::from_str_radix(&digits[1..], 8).ok()
        } else {
            digits.parse().ok()
        }
    }
}

/// The archetype a `format` attribute names, without its underscores, as
/// both compilers know it: GCC's on a GNU target and Clang's.
fn archetype_kind(name: &str, gcc: bool) -> Option<FormatKind> {
    Some(match name {
        | "printf" | "gnu_printf" => FormatKind::Printf,
        | "printf0" | "syslog" if !gcc => FormatKind::Printf,
        | "scanf" | "gnu_scanf" => FormatKind::Scanf,
        | "strftime" | "gnu_strftime" | "strfmon" | "gnu_strfmon" => FormatKind::Unchecked,
        | _ => return None,
    })
}
