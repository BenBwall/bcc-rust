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
        BindingKind,
        Linkage,
        Namespace,
        SemanticErrorKind,
        TypeKind,
        facts::{
            FormatAttribute,
            FormatKind,
        },
    },
    entities::{
        FormatHolder,
        Subject,
    },
    written::WrittenAttribute,
};
use crate::{
    configuration::DiagnosticGroup as G,
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        preprocessing::{
            OperatorTokenType,
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
        let Some((holder, function, declared)) = self.format_holder(subject) else {
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
        let microsoft = self.gcc_microsoft_formats();
        let Some((kind, ms)) = archetype_kind(name, gcc, microsoft) else {
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
        let (Some(format), Some(first)) = (
            self.attribute_index(operands[1]),
            self.attribute_index(operands[2]),
        ) else {
            return;
        };
        let (format_text, first_text) = (format, first);
        let TypeKind::Function {
            parameters,
            variadic,
            prototype: true,
            ..
        } = self.types.nodes[function.index]
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
                    self.format_attribute_warning(
                        G::GccCompat,
                        "GCC requires a function with the 'format' attribute to be variadic",
                        declared,
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
        let attribute = FormatAttribute {
            kind,
            format,
            first,
            ms,
        };
        match (subject, holder) {
            | (Subject::Function { .. }, FormatHolder::Entity(entity)) =>
                self.attributes.entity(entity).format = Some(attribute),
            | (_, holder) => _ = self.attributes.pointer_formats.insert(holder, attribute),
        }
    }

    /// What holds a `format` attribute on `subject`, the function type it
    /// describes and where the declared name is: a function, or an object,
    /// parameter or member that points to a function. GCC takes the
    /// attribute for the function type's; Clang for the declaration's.
    fn format_holder(
        &mut self,
        subject: Subject,
    ) -> Option<(FormatHolder, super::super::TypeId, SourceVectors)> {
        let (holder, ty, name) = match subject {
            | Subject::Function { binding, .. } => {
                let data = self.bindings[binding];
                return Some((
                    FormatHolder::Entity(self.entity_of(binding)),
                    data.ty,
                    data.name.source_vectors,
                ));
            },
            | Subject::Object { binding, .. } | Subject::Parameter(Some(binding)) => {
                let data = self.bindings[binding];
                let holder = if data.linkage == Linkage::None {
                    FormatHolder::Binding(binding)
                } else {
                    FormatHolder::Entity(self.entity_of(binding))
                };
                (holder, data.ty, data.name.source_vectors)
            },
            | Subject::Member(Some((tag, name, ty))) => (
                FormatHolder::Member(tag, name.name),
                ty,
                name.source_vectors,
            ),
            | _ => return None,
        };
        let pointer = self.types.non_atomic(ty);
        let function = self.pointer_target(pointer)?;
        matches!(self.types.nodes[function.index], TypeKind::Function { .. })
            .then_some((holder, function, name))
    }

    /// Gives the body's binding of a parameter its prototype binding's
    /// `format`.
    pub(in crate::translation_phases::semantic_analysis) fn inherit_pointer_format(
        &mut self,
        prototype: usize,
        body: usize,
    ) {
        if let Some(attribute) = self.pointer_format(FormatHolder::Binding(prototype)) {
            _ = self
                .attributes
                .pointer_formats
                .insert(FormatHolder::Binding(body), attribute);
        }
    }

    /// The `format` attribute of the function pointer or member `holder`.
    pub(in crate::translation_phases::semantic_analysis) fn pointer_format(
        &self,
        holder: FormatHolder,
    ) -> Option<FormatAttribute> {
        self.attributes.pointer_formats.get(&holder).copied()
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
            ms: false,
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
        let Some(index) = self.attribute_index(tokens) else {
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

    /// The value of an attribute's index operand, an integer constant
    /// expression both compilers evaluate (C99 §6.6p6): integer constants,
    /// enumeration constants, parentheses and the unary, binary and
    /// conditional operators. `None` for anything else, or a value no
    /// parameter could have.
    fn attribute_index(&self, tokens: &[Token]) -> Option<u32> {
        let mut at = 0;
        let value = self.index_expression(tokens, &mut at, 0)?;
        (at == tokens.len())
            .then_some(value)
            .and_then(|value| u32::try_from(value).ok())
    }

    /// One operand of [`Self::attribute_index`] with its binary operators
    /// that bind at least as tightly as `precedence`, from `*at`.
    fn index_expression(&self, tokens: &[Token], at: &mut usize, precedence: u8) -> Option<i128> {
        use OperatorTokenType as O;
        let mut left = self.index_operand(tokens, at)?;
        loop {
            let Some(TokenType::Operator(operator)) = tokens.get(*at).map(|token| token.kind)
            else {
                return Some(left);
            };
            if operator == O::QuestionMark {
                if precedence > 1 {
                    return Some(left);
                }
                *at += 1;
                let then = self.index_expression(tokens, at, 0)?;
                if !tokens
                    .get(*at)
                    .is_some_and(|token| matches!(token.kind, TokenType::Operator(O::Colon)))
                {
                    return None;
                }
                *at += 1;
                let otherwise = self.index_expression(tokens, at, 1)?;
                left = if left != 0 { then } else { otherwise };
                continue;
            }
            let binding = match operator {
                | O::PipePipe => 2,
                | O::AmpersandAmpersand => 3,
                | O::Pipe => 4,
                | O::Caret => 5,
                | O::Ampersand => 6,
                | O::EqualsEquals | O::ExclamationMarkEquals => 7,
                | O::LessThan | O::GreaterThan | O::LessThanEquals | O::GreaterThanEquals => 8,
                | O::LessThanLessThan | O::GreaterThanGreaterThan => 9,
                | O::Plus | O::Minus => 10,
                | O::Asterisk | O::ForwardSlash | O::Percent => 11,
                | _ => return Some(left),
            };
            if binding < precedence {
                return Some(left);
            }
            *at += 1;
            let right = self.index_expression(tokens, at, binding + 1)?;
            left = match operator {
                | O::PipePipe => i128::from(left != 0 || right != 0),
                | O::AmpersandAmpersand => i128::from(left != 0 && right != 0),
                | O::Pipe => left | right,
                | O::Caret => left ^ right,
                | O::Ampersand => left & right,
                | O::EqualsEquals => i128::from(left == right),
                | O::ExclamationMarkEquals => i128::from(left != right),
                | O::LessThan => i128::from(left < right),
                | O::GreaterThan => i128::from(left > right),
                | O::LessThanEquals => i128::from(left <= right),
                | O::GreaterThanEquals => i128::from(left >= right),
                | O::LessThanLessThan => left.checked_shl(u32::try_from(right).ok()?)?,
                | O::GreaterThanGreaterThan => left.checked_shr(u32::try_from(right).ok()?)?,
                | O::Plus => left.checked_add(right)?,
                | O::Minus => left.checked_sub(right)?,
                | O::Asterisk => left.checked_mul(right)?,
                | O::ForwardSlash => left.checked_div(right)?,
                | _ => left.checked_rem(right)?,
            };
        }
    }

    /// A primary or unary operand of [`Self::attribute_index`].
    fn index_operand(&self, tokens: &[Token], at: &mut usize) -> Option<i128> {
        use OperatorTokenType as O;
        let token = *tokens.get(*at)?;
        *at += 1;
        match token.kind {
            | TokenType::Integer(_) => self.token_number(token).map(i128::from),
            | TokenType::Identifier => {
                let entry = self.lookup(Namespace::Ordinary, token.contents)?;
                let binding = self.bindings[entry.binding];
                if binding.kind != BindingKind::Enumerator {
                    return None;
                }
                binding.value?.to_i128()
            },
            | TokenType::Operator(O::OpeningParenthesis) => {
                let value = self.index_expression(tokens, at, 0)?;
                if !tokens.get(*at).is_some_and(|token| {
                    matches!(token.kind, TokenType::Operator(O::ClosingParenthesis))
                }) {
                    return None;
                }
                *at += 1;
                Some(value)
            },
            | TokenType::Operator(O::Plus) => self.index_operand(tokens, at),
            | TokenType::Operator(O::Minus) => self.index_operand(tokens, at)?.checked_neg(),
            | TokenType::Operator(O::Tilde) => Some(!self.index_operand(tokens, at)?),
            | TokenType::Operator(O::ExclamationMark) =>
                Some(i128::from(self.index_operand(tokens, at)? == 0)),
            | _ => None,
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
/// both compilers know it, and whether GCC reads it by the Microsoft C
/// runtime's rules. On Windows (`microsoft`) GCC takes `printf`, `scanf`
/// and `strftime` for the `ms_` archetypes it also knows by name
/// (MinGW's `msformat-c.cc`); elsewhere it knows those by no name.
fn archetype_kind(name: &str, gcc: bool, microsoft: bool) -> Option<(FormatKind, bool)> {
    Some(match name {
        | "printf" => (FormatKind::Printf, microsoft),
        | "gnu_printf" => (FormatKind::Printf, false),
        | "printf0" | "syslog" if !gcc => (FormatKind::Printf, false),
        | "scanf" => (FormatKind::Scanf, microsoft),
        | "gnu_scanf" => (FormatKind::Scanf, false),
        | "ms_printf" if microsoft => (FormatKind::Printf, true),
        | "ms_scanf" if microsoft => (FormatKind::Scanf, true),
        | "strftime" | "gnu_strftime" | "strfmon" | "gnu_strfmon" => (FormatKind::Unchecked, false),
        | "ms_strftime" if microsoft => (FormatKind::Unchecked, true),
        | _ => return None,
    })
}
