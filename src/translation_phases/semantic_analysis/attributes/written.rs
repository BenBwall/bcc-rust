//! Attribute entries as written: the vendor namespace, the name and the
//! tokens of its parenthesized arguments. Only names at attribute-list
//! depth count; identifiers inside arguments never name attributes.
//! GNU extension: GCC manual, "Attribute Syntax".
//! <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
//! C23: §6.7.13.2p1, pp. 142-143; PDF pp. 155-156 (standard attribute
//! syntax and prefixed names).

use crate::translation_phases::{
    Context,
    SourceVectors,
    parsing::AttributeSpecifier,
    preprocessing::Token,
};

/// One attribute of a specifier list.
#[derive(Clone, Copy)]
pub(super) struct WrittenAttribute<'a> {
    /// `gnu` in `[[gnu::weak]]`, without surrounding underscores.
    pub(super) namespace: Option<&'a str>,
    /// The name without surrounding underscores: `weak` for `__weak__`.
    pub(super) name:      &'a str,
    /// The name's position in the specifier's tokens, which keys parsed
    /// argument expressions.
    pub(super) index:     usize,
    /// The tokens between the argument parentheses, if there are any.
    pub(super) arguments: Option<&'a [Token]>,
    pub(super) source:    SourceVectors,
}

impl WrittenAttribute<'_> {
    /// The comma-separated arguments, split at parenthesis depth zero.
    pub(super) fn argument_count(&self, context: &Context<'_>) -> usize {
        let Some(tokens) = self.arguments else {
            return 0;
        };
        if tokens.is_empty() {
            return 0;
        }
        let mut depth = 0usize;
        let mut count = 1;
        for token in tokens {
            match context.string_cache.at(token.contents) {
                | "(" | "[" | "{" => depth += 1,
                | ")" | "]" | "}" => depth = depth.saturating_sub(1),
                | "," if depth == 0 => count += 1,
                | _ => {},
            }
        }
        count
    }
}

/// Every attribute of `specifier`, in source order.
pub(super) fn written<'a>(
    specifier: &'a AttributeSpecifier<'_>,
    context: &'a Context<'_>,
) -> impl Iterator<Item = WrittenAttribute<'a>> {
    let tokens = specifier.tokens;
    let spelling = move |index: usize| {
        tokens.get(index).map_or("", |token: &Token| {
            context
                .string_cache
                .at(token.contents)
                .trim_end_matches('\0')
        })
    };
    let level = match spelling(0).trim_matches('_') {
        | "attribute" => 2,
        | "declspec" => 1,
        | _ => 0,
    };
    let mut depth = 0usize;
    let mut index = 0;
    std::iter::from_fn(move || {
        while index < tokens.len() {
            let current = index;
            index += 1;
            match spelling(current) {
                | "(" => depth += 1,
                | ")" => depth = depth.saturating_sub(1),
                | "," | ":" | "[" | "]" => {},
                | "" => {},
                | text if depth == level => {
                    let mut namespace = None;
                    let mut name_index = current;
                    let mut name = text;
                    if spelling(current + 1) == ":" && spelling(current + 2) == ":" {
                        namespace = Some(text.trim_matches('_'));
                        name_index = current + 3;
                        name = spelling(name_index);
                        index = name_index + 1;
                        if name.is_empty() {
                            continue;
                        }
                    }
                    let mut arguments = None;
                    if spelling(index) == "(" {
                        let start = index + 1;
                        let mut inner = 1usize;
                        let mut end = start;
                        while end < tokens.len() {
                            match spelling(end) {
                                | "(" => inner += 1,
                                | ")" => {
                                    inner -= 1;
                                    if inner == 0 {
                                        break;
                                    }
                                },
                                | _ => {},
                            }
                            end += 1;
                        }
                        arguments = Some(&tokens[start..end.min(tokens.len())]);
                        index = (end + 1).min(tokens.len());
                    }
                    return Some(WrittenAttribute {
                        namespace,
                        name: name.trim_matches('_'),
                        index: name_index,
                        arguments,
                        source: tokens[name_index].source_vectors,
                    });
                },
                | _ => {},
            }
        }
        None
    })
}
