//! Later-standard and dialect preprocessing operations, translation phase 4.
//!
//! C99: §5.1.1.2p1 item 4, p. 10; PDF p. 22; extensions under §4p6,
//! p. 7; PDF p. 19. C23: conditional queries §6.10.2, pp. 166-169;
//! PDF pp. 179-182; resource inclusion §6.10.4, pp. 171-176;
//! PDF pp. 184-189. Resource elements are 8-bit unsigned bytes.

use std::{
    fmt::Write,
    io::Read,
    path::Path,
};

use super::{
    Expander,
    QueryExpansion,
    driver::{
        TokenizerFrame,
        TokenizerFrameType,
    },
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        Feature,
        FeatureOrigin,
    },
    translation_phases::{
        SourceVector,
        SourceVectors,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType as T,
            TokenSource,
        },
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

pub(super) const LANGUAGE_BUILTINS: &[(&str, Feature)] = &[
    ("__COUNTER__", Feature::Counter),
    ("__has_include", Feature::HasInclude),
    ("__has_include_next", Feature::IncludeNext),
    ("__has_embed", Feature::HasEmbed),
    ("__has_c_attribute", Feature::HasCAttribute),
    ("__has_attribute", Feature::HasAttribute),
    ("__has_builtin", Feature::HasBuiltin),
    ("__pragma", Feature::MsPragma),
    ("__STDC_EMBED_NOT_FOUND__", Feature::Embed),
    ("__STDC_EMBED_FOUND__", Feature::Embed),
    ("__STDC_EMBED_EMPTY__", Feature::Embed),
];

/// Appends the compiler-identity macros to the target's predefined
/// `definitions`, as source read before user input. Following Clang, GNU
/// modes claim GCC 4.2.1 with its inline-semantics macro, and
/// `-fms-extensions` claims MSVC 19.33, Clang's default
/// `-fms-compatibility-version`. `__bcc__` and its version are defined in
/// every mode; `__clang__` never is. They are ordinary macros, so `#undef`
/// works as in Clang.
///
/// C99: further predefined macro names begin with `__` or `_` and an
/// uppercase letter, §6.10.8 paragraph 4, p. 161; PDF p. 173; reserved
/// identifiers, §7.1.3 paragraph 1, p. 166; PDF p. 178.
pub(super) fn with_identity_macros<'a>(
    arena: &'a Bump,
    definitions: &str,
    configuration: CompilerConfiguration,
) -> &'a str {
    let mut out = ArenaString::new_in(arena);
    out.push_str(definitions);
    _ = write!(
        out,
        "#define __bcc__ 1\n#define __bcc_major__ {}\n#define __bcc_minor__ {}\n#define \
         __bcc_patchlevel__ {}\n#define __bcc_version__ \"{}\"\n",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
        env!("CARGO_PKG_VERSION"),
    );
    if configuration.gnu_extensions() {
        out.push_str(
            "#define __GNUC__ 4\n#define __GNUC_MINOR__ 2\n#define __GNUC_PATCHLEVEL__ 1\n",
        );
        // GNU89 inline semantics before C99, C99 semantics from it on.
        out.push_str(if configuration.standard() < CStandard::C99 {
            "#define __GNUC_GNU_INLINE__ 1\n"
        } else {
            "#define __GNUC_STDC_INLINE__ 1\n"
        });
    }
    if configuration.msvc_compatibility() {
        out.push_str(
            "#define _MSC_VER 1933\n#define _MSC_FULL_VER 193300000\n#define _MSC_BUILD \
             1\n#define _MSC_EXTENSIONS 1\n",
        );
    }
    out.into_str()
}

/// GNU builtins can be overridden with a warning, like GCC and Clang. ISO
/// predefined macros and standard query operators retain their protection.
pub(super) fn overridable_gnu_builtin(name: &str) -> bool {
    LANGUAGE_BUILTINS.iter().any(|(spelling, feature)| {
        *spelling == name && matches!(feature.origin(), FeatureOrigin::Gnu)
    })
}

impl<'tu, 'pp: 'x, 'x> Expander<'_, 'tu, 'pp, 'x> {
    /// C99 §6.10.3p4 permits empty arguments, unlike C89 §3.8.3.
    pub(super) fn report_empty_macro_argument(&mut self, source: SourceVectors) {
        self.context
            .report_extension(Feature::EmptyMacroArguments, "empty macro argument", source);
    }

    /// Reports malformed later-standard operations with their original
    /// provenance. C99: §5.1.1.3p1, p. 11; PDF p. 23.
    pub(super) fn language_error(&mut self, message: &'static str, source_vectors: SourceVectors) {
        self.context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::LanguageConstraint(message),
            source_vectors,
        });
    }

    fn integer_pp_token(&mut self, value: u64, source_vectors: SourceVectors) -> PreprocessorToken {
        let mut spelling = ArenaString::new_in(self.scratch);
        _ = write!(spelling, "{value}\0");
        PreprocessorToken {
            kind: T::Number,
            contents: self.context.string_cache.intern(&*spelling),
            source_vectors,
        }
    }

    /// Leaves a rejected token available to its enclosing directive or source
    /// frame. C99: directives end at a newline, §6.10p2, pp. 146-147;
    /// PDF pp. 158-159.
    fn replay_query_boundary(&mut self, token: PreprocessorToken) {
        let tokenizer = TokenSource::replay(
            self.context,
            self.scratch,
            &[&[token]],
            SourceVector::default(),
        );
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::Rescan,
            tokenizer,
        });
    }

    /// Collects an operator argument with an explicit delimiter counter,
    /// retaining its delimiters and their provenance for deferred replay.
    /// GNU/MSVC extension: ordinary-text invocations treat new-lines as
    /// whitespace, following C99 §6.10.3p10-11, p. 152; PDF p. 164.
    /// Directives and replacement lists retain their new-line boundary:
    /// C99: §6.10p2, pp. 146-147; PDF pp. 158-159.
    fn query_arguments<const KEEP_DELIMITERS: bool>(
        &mut self,
        operator: PreprocessorToken,
    ) -> Option<ArenaVec<'x, PreprocessorToken>> {
        let newline_ends_operand = self.state.in_directive
            || matches!(
                self.tokenizer_stack.last().map(|frame| &frame.frame_type),
                Some(
                    TokenizerFrameType::ObjectLikeMacroInvocation { .. }
                        | TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                )
            );
        let open = loop {
            match self.next_preprocessor_token::<false>() {
                | Some(token)
                    if token.kind == T::Whitespace
                        || (token.kind == T::Newline && !newline_ends_operand) => {},
                | token => break token,
            }
        };
        let Some(open) = open else {
            self.language_error(
                "expected '(' after preprocessing operator",
                operator.source_vectors,
            );
            return None;
        };
        if open.kind != T::OpeningParenthesis {
            self.language_error(
                "expected '(' after preprocessing operator",
                open.source_vectors,
            );
            self.replay_query_boundary(open);
            return None;
        }
        let mut tokens = ArenaVec::new_in(self.scratch);
        if KEEP_DELIMITERS {
            tokens.push(open);
        }
        let resource_query = matches!(
            self.context.string_cache.at(operator.contents),
            "__has_include" | "__has_include_next" | "__has_embed"
        );
        if resource_query && !self.collect_written_resource(&mut tokens) {
            self.language_error("unterminated resource query", operator.source_vectors);
            return None;
        }
        let mut resource_started = tokens
            .iter()
            .skip(usize::from(KEEP_DELIMITERS))
            .any(|t| t.kind != T::Whitespace);
        let mut in_header = false;
        let mut depth = 1usize;
        while let Some(mut token) = self.next_preprocessor_token::<false>() {
            if token.kind == T::Newline && !newline_ends_operand {
                token.kind = T::Whitespace;
                token.contents = self.context.string_cache.intern(" ");
            }
            if resource_query && !resource_started && token.kind != T::Whitespace {
                resource_started = true;
                in_header = self
                    .context
                    .string_cache
                    .at(token.contents)
                    .starts_with('<');
            }
            if in_header && token.kind != T::Newline {
                in_header = !self.context.string_cache.at(token.contents).contains('>');
                tokens.push(token);
                continue;
            }
            match token.kind {
                | T::OpeningParenthesis => depth += 1,
                | T::ClosingParenthesis => {
                    depth -= 1;
                    if depth == 0 {
                        if KEEP_DELIMITERS {
                            tokens.push(token);
                        }
                        return Some(tokens);
                    }
                },
                | T::Newline => {
                    self.language_error(
                        "unterminated preprocessing operator",
                        operator.source_vectors,
                    );
                    self.replay_query_boundary(token);
                    return None;
                },
                | _ => {},
            }
            tokens.push(token);
        }
        self.language_error(
            "unterminated preprocessing operator",
            operator.source_vectors,
        );
        None
    }

    /// A written header name is not macro-replaced inside its delimiters.
    /// Returns false when a new-line ends the line, or the replacement list,
    /// before the name does; the new-line is left unread for the frame that
    /// owns it.
    /// C99: §6.10.2p2-4, pp. 149-150; PDF pp. 161-162.
    fn collect_written_resource(&mut self, tokens: &mut ArenaVec<'x, PreprocessorToken>) -> bool {
        let position = self.position();
        let ignored = self.context.ignore_tokenizer_errors();
        self.context.set_ignore_tokenizer_errors(true);
        let first = Self::next_ignore_whitespace(&mut self.tokenizer, self.context);
        self.context.set_ignore_tokenizer_errors(ignored);
        self.set_position(position);
        let Some(first) = first.filter(|t| {
            t.kind == T::String || self.context.string_cache.at(t.contents).starts_with('<')
        }) else {
            return true;
        };
        self.context.set_ignore_tokenizer_errors(true);
        let mut terminated = true;
        loop {
            let position = self.position();
            let Some(token) = self.tokenizer.next_item(self.context) else {
                break;
            };
            if token.kind == T::Newline {
                self.set_position(position);
                terminated = false;
                break;
            }
            tokens.push(token);
            if (first.kind == T::String && token.kind == T::String)
                || self.context.string_cache.at(token.contents).contains('>')
            {
                break;
            }
        }
        self.context.set_ignore_tokenizer_errors(ignored);
        terminated
    }

    /// Reads the macro-expanded header operand common to queries and embedding.
    /// C23: §6.10.2p7, pp. 166-167; PDF pp. 179-180.
    fn resource_operand(
        &mut self,
        tokens: &[PreprocessorToken],
        source: SourceVectors,
    ) -> Option<(&'x str, bool, usize)> {
        let mut index = 0;
        while tokens.get(index).is_some_and(|t| t.kind == T::Whitespace) {
            index += 1;
        }
        let Some(&first) = tokens.get(index) else {
            self.language_error("expected a quoted or angle-bracket resource name", source);
            return None;
        };
        let spelling = self.context.string_cache.at(first.contents);
        let mut name = ArenaString::new_in(self.scratch);
        if let Some(rest) = spelling.strip_prefix('<') {
            name.push_str(rest);
            index += 1;
            while let Some(token) = tokens.get(index) {
                index += 1;
                let spelling = self.context.string_cache.at(token.contents);
                if let Some(end) = spelling.find('>') {
                    if end + 1 != spelling.len() {
                        self.language_error(
                            "extra tokens after resource header name",
                            token.source_vectors,
                        );
                        return None;
                    }
                    name.push_str(&spelling[..end]);
                    if let ([open], [close]) = (
                        self.context.get_source_vectors(first.source_vectors),
                        self.context.get_source_vectors(token.source_vectors),
                    ) && open.source_file_index == close.source_file_index
                        && open.index < close.index
                        && let Some(text) = self.context.source_text(open.source_file_index)
                        && text.as_bytes().get(open.index as usize) == Some(&b'<')
                        && let Some(closing) =
                            text.get(close.index as usize..(close.index + close.length) as usize)
                        && let Some(offset) = closing.find('>')
                    {
                        name.clear();
                        for c in
                            crate::translation_phases::preprocessor_tokenizer::logical_characters(
                                self.scratch,
                                text,
                                open.index as usize + 1..close.index as usize + offset,
                                self.context.configuration.accepts(Feature::Trigraphs),
                            )
                        {
                            name.push(c.character);
                        }
                    }
                    if name.is_empty() {
                        self.language_error("resource name must not be empty", source);
                        return None;
                    }
                    return Some((name.into_str(), true, index));
                }
                name.push_str(spelling.trim_end_matches('\0'));
            }
        } else if first.kind == T::GeneratedString {
            if !spelling.is_empty() {
                name.push_str(spelling);
                return Some((name.into_str(), false, index + 1));
            }
        } else if first.kind == T::String && spelling.starts_with('"') {
            if let [location] = self.context.get_source_vectors(first.source_vectors)
                && let Some(text) = self.context.source_text(location.source_file_index)
                && text.as_bytes().get(location.index as usize) == Some(&b'"')
            {
                let characters =
                    crate::translation_phases::preprocessor_tokenizer::logical_characters(
                        self.scratch,
                        text,
                        location.index as usize + 1..(location.index + location.length) as usize,
                        self.context.configuration.accepts(Feature::Trigraphs),
                    );
                let Some(end) = characters.iter().position(|c| c.character == '"') else {
                    self.language_error("unterminated quoted resource name", source);
                    return None;
                };
                if end + 1 != characters.len() {
                    self.language_error("extra tokens after resource header name", source);
                    return None;
                }
                for c in &characters[..end] {
                    name.push(c.character);
                }
            } else if spelling.ends_with('"') && spelling.len() > 2 {
                name.push_str(&spelling[1..spelling.len() - 1]);
            }
            if !name.is_empty() {
                return Some((name.into_str(), false, index + 1));
            }
        }
        self.language_error("expected a quoted or angle-bracket resource name", source);
        None
    }

    /// Resolves a resource without diagnostics or pragma-once filtering,
    /// searching the places an `#include` of the same name would.
    /// C99: header search is implementation-defined, §6.10.2p2-3,
    /// pp. 149-150; PDF pp. 161-162. C23: embed queries use the embed
    /// search, §6.10.2p7, pp. 166-167; PDF pp. 179-180. `start` continues an
    /// `__has_include_next` lookup.
    fn find_resource(
        &mut self,
        name: &str,
        system: bool,
        embed: bool,
        start: Option<usize>,
    ) -> Option<u32> {
        let path = Path::new(name);
        if path.is_absolute() {
            return path
                .is_file()
                .then(|| self.context.intern_source_file(path));
        }
        let places = self.header_search_places(self.physical_source_file_index(), system, start);
        places.into_iter().find_map(|(_, directory)| {
            // Built-in headers supply source text, not filesystem resources.
            // C23 §6.10.2p7: the query must use the matching #embed search.
            if embed && directory == Path::new(crate::headers::DIRECTORY) {
                None
            } else {
                self.probe_header(directory, path)
            }
        })
    }

    /// Evaluates query operators and consumes the MSVC token-form pragma
    /// operator. C23: §6.10.2p6-11, pp. 165-167; PDF pp. 178-180.
    pub(super) fn language_builtin(
        &mut self,
        token: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        // Invalid nested query operands must not exhaust the native call stack.
        if self.state.query_depth == 64 {
            self.language_error(
                "preprocessing query nesting limit exceeded",
                token.source_vectors,
            );
            return Some(self.integer_pp_token(0, token.source_vectors));
        }
        self.state.query_depth += 1;
        let result = self.language_builtin_inner(token);
        self.state.query_depth -= 1;
        // A malformed conditional operand still occupies its expression
        // position. C99: diagnostics and recovery, §5.1.1.3p1, p. 11;
        // PDF p. 23. Keep its boundary replay, but avoid diagnosing a second
        // missing operand at the following directive. Token-form pragmas
        // intentionally disappear and must never introduce an integer.
        if result.is_none() && self.context.string_cache.at(token.contents) != "__pragma" {
            Some(self.integer_pp_token(0, token.source_vectors))
        } else {
            result
        }
    }

    fn language_builtin_inner(&mut self, token: PreprocessorToken) -> Option<PreprocessorToken> {
        let name = self.context.string_cache.at(token.contents);
        // C23 §6.10.4.2p3: only the limit's evaluation uses conditional
        // inclusion rules. Keep query calls intact during the directive's
        // initial replacement, including their written header names.
        if self.query_expansion == QueryExpansion::Defer
            && matches!(name, "__has_include" | "__has_embed" | "__has_c_attribute")
        {
            let arguments = self.query_arguments::<true>(token)?.leak();
            let tokenizer = TokenSource::replay(
                self.context,
                self.scratch,
                &[arguments],
                SourceVector::default(),
            );
            self.push_tokenizer_frame(TokenizerFrame {
                frame_type: TokenizerFrameType::DeferredQuery,
                tokenizer,
            });
            return Some(token);
        }
        let &(name, feature) = LANGUAGE_BUILTINS
            .iter()
            .find(|(spelling, _)| *spelling == name)
            .expect("registered language builtin");
        self.context
            .report_extension(feature, name, token.source_vectors);
        let name = self.context.string_cache.at(token.contents);
        // C23 §6.10.2p11, p. 167; PDF p. 180: these names are
        // conditional-inclusion operators, not ordinary expression macros.
        // GNU `__has_include_next` follows `__has_include`.
        if matches!(
            name,
            "__has_include" | "__has_include_next" | "__has_embed" | "__has_c_attribute"
        ) && !self.state.conditional_queries
        {
            self.language_error(
                "resource and C attribute queries require a preprocessing conditional expression",
                token.source_vectors,
            );
            drop(self.query_arguments::<false>(token));
            return Some(self.integer_pp_token(0, token.source_vectors));
        }
        let value = match name {
            | "__COUNTER__" => {
                let value = self.state.counter;
                self.state.counter = value.wrapping_add(1);
                value
            },
            | "__STDC_EMBED_NOT_FOUND__" => 0,
            | "__STDC_EMBED_FOUND__" => 1,
            | "__STDC_EMBED_EMPTY__" => 2,
            | _ => {
                let tokens = self.query_arguments::<false>(token)?.leak();
                let name = self.context.string_cache.at(token.contents);
                if name == "__pragma" {
                    let newline = PreprocessorToken {
                        kind:           T::Newline,
                        contents:       self.context.string_cache.intern("\n"),
                        source_vectors: token.source_vectors,
                    };
                    let old = std::mem::replace(
                        &mut self.tokenizer,
                        TokenSource::replay(
                            self.context,
                            self.scratch,
                            &[tokens, &[newline]],
                            SourceVector::default(),
                        ),
                    );
                    _ = self.parse_pragma_directive();
                    self.tokenizer = old;
                    return None;
                }
                if matches!(name, "__has_include" | "__has_include_next" | "__has_embed") {
                    let embed = name == "__has_embed";
                    // GNU `__has_include_next` answers whether
                    // `#include_next` would find the header, with its
                    // diagnostics for a lookup that cannot continue.
                    let start = (name == "__has_include_next")
                        .then(|| {
                            self.include_next_start("__has_include_next", token.source_vectors)
                        })
                        .flatten();
                    let (name, system, end) =
                        self.resource_operand(tokens, token.source_vectors)?;
                    let params =
                        self.embed_parameters(&tokens[end..], token.source_vectors, embed, true)?;
                    if !params.supported {
                        return Some(self.integer_pp_token(0, token.source_vectors));
                    }
                    if let Some(path) = self.find_resource(name, system, embed, start) {
                        if embed
                            && (params.limit == Some(0)
                                || std::fs::metadata(self.context.get_source_file(path))
                                    .is_ok_and(|m| m.len() == 0))
                        {
                            2
                        } else {
                            1
                        }
                    } else {
                        0
                    }
                } else {
                    let mut operand = ArenaString::new_in(self.scratch);
                    for t in tokens.iter().filter(|t| t.kind != T::Whitespace) {
                        let spelling = self.context.string_cache.at(t.contents);
                        let spelling = if name != "__has_builtin" && t.kind.is_identifier() {
                            spelling
                                .strip_prefix("__")
                                .and_then(|s| s.strip_suffix("__"))
                                .unwrap_or(spelling)
                        } else {
                            spelling
                        };
                        operand.push_str(spelling);
                    }
                    let mut significant = ArenaVec::new_in(self.scratch);
                    significant.extend(tokens.iter().filter(|t| t.kind != T::Whitespace).copied());
                    if !matches!(&significant[..], [t] if t.kind.is_identifier())
                        && !matches!(&significant[..], [a,b,c,d] if name != "__has_builtin" && a.kind.is_identifier() && b.kind == T::Colon && c.kind == T::Colon && d.kind.is_identifier())
                    {
                        self.language_error(
                            "expected an identifier in preprocessing feature query",
                            token.source_vectors,
                        );
                        return Some(self.integer_pp_token(0, token.source_vectors));
                    }
                    match (name, &*operand) {
                        // C23 §6.7.13.2p2, p. 143; PDF p. 156: the standard
                        // attributes, `_Noreturn` included (§6.7.13.7p1).
                        | (
                            "__has_c_attribute",
                            "deprecated" | "fallthrough" | "nodiscard" | "maybe_unused"
                            | "noreturn" | "_Noreturn" | "unsequenced" | "reproducible",
                        ) => 202_311,
                        | (
                            "__has_attribute",
                            "unused" | "deprecated" | "aligned" | "packed" | "noreturn" | "weak"
                            | "section" | "visibility" | "format" | "always_inline" | "noinline",
                        )
                        | (
                            "__has_builtin",
                            "__builtin_va_arg"
                            | "__builtin_va_start"
                            | "__builtin_va_end"
                            | "__builtin_va_copy"
                            | "__builtin_offsetof"
                            | "__builtin_types_compatible_p"
                            | "__builtin_choose_expr",
                        ) => 1,
                        | _ => 0,
                    }
                }
            },
        };
        Some(self.integer_pp_token(value, token.source_vectors))
    }

    /// Scans parameter clauses using the balanced-token grammar shared by
    /// resource inclusion and its conditional query.
    /// C23: §6.10.1p1, pp. 163-164; PDF pp. 176-177.
    fn embed_parameters(
        &mut self,
        tokens: &'x [PreprocessorToken],
        source: SourceVectors,
        allowed: bool,
        query: bool,
    ) -> Option<EmbedParameters<'x>> {
        let mut result = EmbedParameters {
            limit:     None,
            prefix:    &[],
            suffix:    &[],
            if_empty:  &[],
            supported: true,
        };
        let mut index = 0;
        let mut seen = 0u8;
        while index < tokens.len() {
            if tokens[index].kind == T::Whitespace {
                index += 1;
                continue;
            }
            if !allowed {
                self.language_error(
                    "extra tokens after header query",
                    tokens[index].source_vectors,
                );
                return None;
            }
            let name = self.context.string_cache.at(tokens[index].contents);
            let bit = match name {
                | "limit" | "__limit__" => 1,
                | "prefix" | "__prefix__" => 2,
                | "suffix" | "__suffix__" => 4,
                | "if_empty" | "__if_empty__" => 8,
                | _ =>
                    if query {
                        result.supported = false;
                        0
                    } else {
                        self.language_error(
                            "unsupported embed parameter",
                            tokens[index].source_vectors,
                        );
                        return None;
                    },
            };
            if seen & bit != 0 {
                self.language_error("duplicate embed parameter", tokens[index].source_vectors);
                return None;
            }
            seen |= bit;
            index += 1;
            let significant = |mut index: usize| {
                while tokens.get(index).is_some_and(|t| t.kind == T::Whitespace) {
                    index += 1;
                }
                index
            };
            if bit == 0 {
                // C23 §6.10.1p1: `pp-prefixed-parameter: identifier ::
                // identifier`.
                let first = significant(index);
                let second = significant(first + 1);
                let suffix = significant(second + 1);
                if tokens.get(first).is_some_and(|t| t.kind == T::Colon)
                    && tokens.get(second).is_some_and(|t| t.kind == T::Colon)
                    && tokens.get(suffix).is_some_and(|t| t.kind.is_identifier())
                {
                    index = suffix + 1;
                }
            }
            index = significant(index);
            if tokens
                .get(index)
                .is_none_or(|t| t.kind != T::OpeningParenthesis)
            {
                // Only a standard parameter requires its clause (C23
                // §6.10.4.2p1, §6.10.4.3p1, §6.10.4.4p1, §6.10.4.5p1).
                if bit == 0 {
                    continue;
                }
                self.language_error("expected '(' after embed parameter", source);
                return None;
            }
            index += 1;
            let start = index;
            let mut delimiters = ArenaVec::new_in(self.scratch);
            delimiters.push(T::ClosingParenthesis);
            while index < tokens.len() {
                // A query's header-name is opaque, even if its characters
                // include delimiters (C99 §6.4.7p1, C23 §6.10.2p7).
                if tokens[index].kind.is_identifier()
                    && matches!(
                        self.context.string_cache.at(tokens[index].contents),
                        "__has_include" | "__has_embed"
                    )
                {
                    let open = significant(index + 1);
                    if tokens
                        .get(open)
                        .is_some_and(|t| t.kind == T::OpeningParenthesis)
                    {
                        let header = significant(open + 1);
                        let (_, _, end) =
                            self.resource_operand(&tokens[header..], tokens[index].source_vectors)?;
                        delimiters.push(T::ClosingParenthesis);
                        index = header + end;
                        continue;
                    }
                }
                match tokens[index].kind {
                    | T::OpeningParenthesis => delimiters.push(T::ClosingParenthesis),
                    | T::OpeningSquareBracket => delimiters.push(T::ClosingSquareBracket),
                    | T::OpeningCurlyBrace => delimiters.push(T::ClosingCurlyBrace),
                    | T::ClosingParenthesis | T::ClosingSquareBracket | T::ClosingCurlyBrace => {
                        if delimiters.pop() != Some(tokens[index].kind) {
                            self.language_error(
                                "mismatched delimiter in embed parameter",
                                tokens[index].source_vectors,
                            );
                            return None;
                        }
                        if delimiters.is_empty() {
                            break;
                        }
                    },
                    | _ => {},
                }
                index += 1;
            }
            if !delimiters.is_empty() {
                self.language_error("unterminated embed parameter", source);
                return None;
            }
            let body = &tokens[start..index];
            index += 1;
            match bit {
                | 1 => {
                    let value = self.eval_fenced_resource_limit(body, source);
                    result.limit = value;
                    _ = value?;
                },
                | 2 => result.prefix = body,
                | 4 => result.suffix = body,
                | 8 => result.if_empty = body,
                | _ => {},
            }
        }
        Some(result)
    }

    /// Evaluates a `limit` operand as its own fenced frame, ended by a
    /// sentinel new-line, like an argument prescan. The operand may come from
    /// a replacement list that is still being read: its frames and cursors
    /// stay below the fence, so the sentinel cannot end them.
    /// C23: §6.10.4.2 paragraphs 1, 3, p. 174; PDF p. 187.
    fn eval_fenced_resource_limit(
        &mut self,
        body: &[PreprocessorToken],
        source: SourceVectors,
    ) -> Option<u64> {
        let end = PreprocessorToken {
            kind:           T::Newline,
            contents:       self.context.string_cache.intern("\n"),
            source_vectors: source,
        };
        let location = self
            .context
            .get_source_vectors(source)
            .first()
            .cloned()
            .unwrap_or_default();
        let tokenizer = TokenSource::replay(self.context, self.scratch, &[body, &[end]], location);
        let hash_hash_stack =
            std::mem::replace(&mut self.hash_hash_stack, ArenaVec::new_in(self.scratch));
        let generate_placeholders = self.generate_placeholders;
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::Rescan,
            tokenizer,
        });
        let depth = self.tokenizer_stack.len();
        let operand_fence = std::mem::replace(&mut self.operand_fence, 0);
        let expansion_fence = std::mem::replace(&mut self.expansion_fence, depth);
        let verbatim_fence = std::mem::replace(&mut self.verbatim_fence, 0);
        let query_expansion =
            std::mem::replace(&mut self.query_expansion, QueryExpansion::Evaluate);
        let conditional_queries = std::mem::replace(&mut self.state.conditional_queries, true);
        let value = self.eval_resource_limit();
        self.state.conditional_queries = conditional_queries;
        self.query_expansion = query_expansion;
        // An expression that stopped early leaves its remaining operand
        // frames above the fence.
        while self.tokenizer_stack.len() >= depth {
            self.pop_tokenizer_frame();
        }
        self.operand_fence = operand_fence;
        self.expansion_fence = expansion_fence;
        self.verbatim_fence = verbatim_fence;
        self.generate_placeholders = generate_placeholders;
        self.hash_hash_stack = hash_hash_stack;
        value
    }

    fn embed_error(
        &mut self,
        error_type: PreprocessorErrorType<'tu>,
        directive: PreprocessorToken,
    ) {
        self.context.preprocessor_error(PreprocessorError {
            error_type,
            source_vectors: directive.source_vectors,
        });
    }

    #[cold]
    fn embed_unreadable(
        &mut self,
        name: &str,
        error: &std::io::Error,
        directive: PreprocessorToken,
    ) {
        let mut reason = ArenaString::new_in(self.scratch);
        _ = write!(reason, "{error}");
        let error_type = PreprocessorErrorType::EmbeddedResourceUnreadable {
            name:   self.context.diagnostic_text(name),
            reason: self.context.diagnostic_text(&reason),
        };
        self.embed_error(error_type, directive);
    }

    /// Replaces resource inclusion with ordinary integer preprocessing tokens.
    /// C23: §6.10.4.1p7, p. 171; PDF p. 184, and §6.10.4.2p4,
    /// p. 174; PDF p. 187.
    pub(super) fn parse_embed_directive(&mut self, directive: PreprocessorToken) {
        self.context
            .report_extension(Feature::Embed, "#embed", directive.source_vectors);
        let mut tokens = ArenaVec::new_in(self.scratch);
        _ = self.collect_written_resource(&mut tokens);
        let query_expansion = std::mem::replace(&mut self.query_expansion, QueryExpansion::Defer);
        // The rest of the line, through its new-line, belongs to the
        // directive whether or not the resource is valid. C99: §6.10p2,
        // pp. 146-147; PDF pp. 158-159.
        while let Some(token) = self.next_preprocessor_token::<false>() {
            if token.kind == T::Newline {
                break;
            }
            tokens.push(token);
        }
        self.query_expansion = query_expansion;
        self.resume_at_line_start();
        let tokens = tokens.leak();
        let Some((name, system, end)) = self.resource_operand(tokens, directive.source_vectors)
        else {
            return;
        };
        let Some(params) =
            self.embed_parameters(&tokens[end..], directive.source_vectors, true, false)
        else {
            return;
        };
        let Some(path) = self.find_resource(name, system, true, None) else {
            self.embed_error(
                PreprocessorErrorType::EmbeddedResourceNotFound(self.context.diagnostic_text(name)),
                directive,
            );
            return;
        };
        let read = std::fs::File::open(self.context.get_source_file(path)).and_then(|file| {
            let length = file.metadata()?.len();
            Ok((file, length))
        });
        let (mut file, length) = match read {
            | Ok(opened) => opened,
            | Err(error) => {
                self.embed_unreadable(name, &error, directive);
                return;
            },
        };
        let length = length.min(params.limit.unwrap_or(u64::MAX));
        let Ok(length) = usize::try_from(length) else {
            self.embed_error(
                PreprocessorErrorType::EmbeddedResourceTooLarge(self.context.diagnostic_text(name)),
                directive,
            );
            return;
        };
        let bytes = self
            .scratch
            .alloc_slice_fill_iter(std::iter::repeat_n(0u8, length));
        if let Err(error) = file.read_exact(bytes) {
            self.embed_unreadable(name, &error, directive);
            return;
        }
        let mut output = ArenaVec::new_in(self.scratch);
        if bytes.is_empty() {
            output.extend_from_slice(params.if_empty);
        } else {
            output.extend_from_slice(params.prefix);
            for (index, byte) in bytes.iter().enumerate() {
                if index != 0 {
                    output.push(PreprocessorToken {
                        kind:           T::Comma,
                        contents:       self.context.string_cache.intern(","),
                        source_vectors: directive.source_vectors,
                    });
                }
                output.push(self.integer_pp_token(u64::from(*byte), directive.source_vectors));
            }
            output.extend_from_slice(params.suffix);
        }
        output.push(PreprocessorToken {
            kind:           T::Newline,
            contents:       self.context.string_cache.intern("\n"),
            source_vectors: directive.source_vectors,
        });
        let tokenizer = TokenSource::replay(
            self.context,
            self.scratch,
            &[&output],
            SourceVector::default(),
        );
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::Rescan,
            tokenizer,
        });
        self.resume_at_line_start();
    }
}

struct EmbedParameters<'x> {
    supported: bool,
    limit:     Option<u64>,
    prefix:    &'x [PreprocessorToken],
    suffix:    &'x [PreprocessorToken],
    if_empty:  &'x [PreprocessorToken],
}
