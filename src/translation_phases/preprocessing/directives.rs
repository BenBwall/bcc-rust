//! Directive dispatch and the non-conditional directives.

use std::{
    ffi::OsStr,
    ops::ControlFlow,
    path::{
        Component,
        Path,
    },
};

use super::{
    Expander,
    driver::{
        TokenizerFrame,
        TokenizerFrameType,
    },
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    macro_expansion::MacroDefinition,
    token::{
        StringTokenType,
        TokenType,
    },
};
use crate::{
    configuration::{
        CStandard,
        ExtensionPolicy,
    },
    translation_phases::{
        Context,
        SourcePosition,
        SourceVectors,
        StrExt,
        TranslationPhase,
        preprocessor_tokenizer::{
            LogicalCharacter,
            PreprocessorToken,
            PreprocessorTokenType,
            logical_characters,
            position_after,
        },
    },
    util::{
        bump::{
            ArenaString,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// Maximum live header depth, excluding the main source file. C99 §5.2.4.1
/// requires support for at least 15 nested included files.
const MAX_INCLUDE_NESTING: usize = 200;

/// Compares one position of two macro definitions under C99 §6.10.3p2: the
/// tokens must be spelled identically, while any two whitespace separations
/// are equivalent and a line end matches the end of input.
fn same_replacement_token(
    context: &Context<'_>,
    old: Option<&PreprocessorToken>,
    new: Option<&PreprocessorToken>,
) -> bool {
    let ends = |token: Option<&PreprocessorToken>| {
        token.is_none_or(|token| token.kind == PreprocessorTokenType::Newline)
    };
    match (old, new) {
        | _ if ends(old) && ends(new) => true,
        | (Some(old), Some(new)) =>
            old.kind == new.kind
                && (old.kind == PreprocessorTokenType::Whitespace
                    || context.string_cache.at(old.contents)
                        == context.string_cache.at(new.contents)),
        | _ => false,
    }
}

/// How an `#include` operand is written, judged from its first token.
enum IncludeOperand {
    /// `<…>`, read from the source text.
    Angle,
    /// `"…"`, read from the source text.
    Quoted,
    /// Anything else, which macro replacement must turn into a header name.
    Other,
}

/// The header name of an `#include` directive.
struct HeaderName<'x> {
    name:              &'x str,
    is_system_header:  bool,
    /// The whole operand.
    source_vectors:    SourceVectors,
    /// The closing delimiter the operand lacks, and where it was expected.
    missing_delimiter: Option<(char, SourceVectors)>,
    /// The first sequence C99 §6.4.7p3 does not allow in a header name, and
    /// where it is.
    invalid:           Option<(&'static str, SourceVectors)>,
    /// Where a `"…"` name first uses a backslash, which only the backslash
    /// extension accepts.
    backslash:         Option<SourceVectors>,
    /// A character after `>` in the token that closes a written angle name.
    extra_tokens:      Option<SourceVectors>,
}

/// The first sequence in `name` whose behavior in a header name C99 §6.4.7p3
/// leaves undefined, with its byte offset: `'`, `//`, or `/*` in either
/// form, and also `\` or `"` between `<` and `>`. A backslash in a `"…"`
/// name is the backslash extension, which the configured
/// [`ExtensionPolicy`] governs instead.
fn invalid_header_sequence(name: &str, angle: bool) -> Option<(usize, &'static str)> {
    let bytes = name.as_bytes();
    bytes.iter().enumerate().find_map(|(offset, &byte)| {
        let sequence = match (byte, bytes.get(offset + 1)) {
            | (b'\'', _) => "'",
            | (b'\\', _) if angle => "\\",
            | (b'"', _) if angle => "\"",
            | (b'/', Some(b'/')) => "//",
            | (b'/', Some(b'*')) => "/*",
            | _ => return None,
        };
        Some((offset, sequence))
    })
}

/// Point into a macro-expanded token when its spelling still occupies the
/// corresponding bytes in a source file. Pasted or rewritten tokens retain
/// their original provenance when they cannot be narrowed this way.
fn expanded_header_sequence_source(
    context: &mut Context<'_>,
    token_source: SourceVectors,
    offset: usize,
    sequence: &'static str,
) -> SourceVectors {
    let vector = context.first_source_vector(token_source).clone();
    let Some(source) = context.source_text(vector.source_file_index) else {
        return token_source;
    };
    if offset + sequence.len() > vector.length as usize {
        return token_source;
    }
    let index = vector.index as usize + offset;
    if source.get(index..index + sequence.len()) != Some(sequence) {
        return token_source;
    }
    let anchor = SourcePosition {
        index:  vector.index as usize,
        line:   vector.line,
        column: vector.column,
    };
    let position = position_after(source, anchor, index);
    context.create_source_vectors(position, vector.source_file_index, sequence.len())
}

/// `directory.join(path)`, spelled in `buffer` instead of a `PathBuf`.
///
/// A plain relative `path` joins a directory by a separator, which is how
/// every ordinary include lookup joins. Other forms, such as a Windows `path`
/// with a drive or root, or a verbatim or bare-drive `directory`, follow
/// `PathBuf::push`'s platform rules, so they are joined by std and copied.
#[expect(
    clippy::disallowed_methods,
    reason = "Windows drive- or root-relative names and verbatim or bare-drive directories follow \
              std's `PathBuf::push` rules, so this rare fallback joins with std and copies the \
              result into the arena."
)]
fn join_path<'b>(buffer: &'b mut ArenaVec<'_, u8>, directory: &Path, path: &Path) -> &'b Path {
    let plain_path = matches!(
        path.components().next(),
        Some(Component::Normal(_) | Component::CurDir | Component::ParentDir)
    );
    let mut directory_components = directory.components();
    let plain_directory = match directory_components.next() {
        | Some(Component::Prefix(prefix)) =>
            !prefix.kind().is_verbatim() && directory_components.next().is_some(),
        | _ => true,
    };
    buffer.clear();
    if plain_path && plain_directory {
        let directory = directory.as_os_str().as_encoded_bytes();
        buffer.extend_from_slice(directory);
        if directory
            .last()
            .is_some_and(|&byte| !std::path::is_separator(char::from(byte)))
        {
            buffer.extend_from_slice(std::path::MAIN_SEPARATOR_STR.as_bytes());
        }
        buffer.extend_from_slice(path.as_os_str().as_encoded_bytes());
    } else {
        buffer.extend_from_slice(directory.join(path).as_os_str().as_encoded_bytes());
    }
    // SAFETY: the buffer holds encoded bytes of `OsStr`s from this process,
    // possibly joined by an ASCII separator, which is a mixture of UTF-8 and
    // encoded bytes split only at UTF-8 boundaries.
    Path::new(unsafe { OsStr::from_encoded_bytes_unchecked(buffer) })
}

/// A header name read from source text, with the locations of its
/// questionable characters as positions and source byte lengths.
struct WrittenHeaderName<'a> {
    name:      &'a str,
    /// The first sequence C99 §6.4.7p3 does not allow.
    invalid:   Option<(&'static str, SourcePosition, usize)>,
    /// The first backslash of a `"…"` name.
    backslash: Option<(SourcePosition, usize)>,
}

/// The header name that `characters` of `source` spell, spelled in
/// `arena`. `anchor` is a known position at or before the characters.
fn header_name_from_source<'a>(
    arena: &'a Bump,
    source: &str,
    anchor: SourcePosition,
    characters: &[LogicalCharacter],
    angle: bool,
) -> WrittenHeaderName<'a> {
    let mut name = ArenaString::new_in(arena);
    for character in characters {
        name.push(character.character);
    }
    let name = name.into_str();
    // Locates `length` characters starting at byte `offset` of the name.
    let locate = |offset: usize, length: usize| {
        let first = name[..offset].chars().count();
        let last = &characters[first + length - 1];
        let start = characters[first].index;
        (
            position_after(source, anchor, start),
            last.index + last.length - start,
        )
    };
    // Every sequence is ASCII, so its length counts its characters.
    let invalid = invalid_header_sequence(name, angle).map(|(offset, sequence)| {
        let (start, length) = locate(offset, sequence.len());
        (sequence, start, length)
    });
    let backslash = if angle {
        None
    } else {
        name.find('\\').map(|offset| locate(offset, 1))
    };
    WrittenHeaderName {
        name,
        invalid,
        backslash,
    }
}

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "Integer literal values are range-checked before narrowing."
)]
#[expect(
    clippy::while_let_loop,
    reason = "The macro-parameter loop has multiple semantic exit conditions."
)]
impl<'x> Expander<'_, '_, '_, 'x> {
    pub(super) fn parse_directive(&mut self, token: PreprocessorToken) {
        if !self.last_was_newline {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HashMustBeFirstCharacterOnLine,
                source_vectors: token.source_vectors,
            });
        }
        let Some(directive) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
        else {
            return;
        };
        match directive.kind {
            // Null directive.
            | PreprocessorTokenType::Newline => {
                self.last_was_newline = true;
                self.current_is_newline = true;
                return;
            },
            // This is the general case. We handle it in the function body.
            // If token is defined, it'll be handled when we match on contents.
            | PreprocessorTokenType::Defined
            | PreprocessorTokenType::Identifier
            | PreprocessorTokenType::UniversalIdentifier => (),
            | _ => {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::HashMustBeFollowedByIdentifier,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline();
                return;
            },
        }
        match self.context.string_cache.at(directive.contents) {
            | "if" => self.parse_if_directive(directive),
            | "ifdef" => self.parse_ifdef_directive(directive),
            | "ifndef" => self.parse_ifndef_directive(directive),
            | "elif" => self.parse_elif_directive(directive),
            | "else" => self.parse_else_directive(directive),
            | "endif" => self.parse_endif_directive(directive),
            | "include" => self.parse_include_directive(directive),
            | "define" => self.parse_define_directive(directive),
            | "undef" => self.parse_undef_directive(directive),
            | "line" => self.parse_line_directive(directive),
            | "error" => self.parse_error_directive(directive),
            | "pragma" => {
                if !self.parse_pragma_directive(directive) {
                    self.skip_until_newline();
                }
                self.last_was_newline = true;
                self.current_is_newline = true;
            },
            | _ => {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnknownDirective,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline();
            },
        }
    }

    /// The source text a `_Pragma` string literal stands for (C99 §6.10.9p1),
    /// with a final newline, in the translation-unit arena, where diagnostics
    /// can still quote it.
    pub(super) fn prepare_pragma_operator_string<'c>(
        context: &Context<'c>,
        string: StringCacheId,
    ) -> &'c str {
        let string = context.string_cache.at(string);
        // Nothing else is allocated in the arena while the text is written.
        let mut text = context.tu_arena().tail_vec::<u8>();
        // A quote is written only once another character follows it, so the
        // literal's closing quote is never written.
        let mut quote_pending = false;
        let mut write = |c: char| {
            if std::mem::take(&mut quote_pending) {
                text.push(b'"');
            }
            if c == '"' {
                quote_pending = true;
            } else {
                for &byte in c.encode_utf8(&mut [0; 4]).as_bytes() {
                    text.push(byte);
                }
            }
        };
        // Skip the leading quote.
        let mut index = 1;
        if string.char_at(0) == Some('L') {
            index += 1;
        }
        while let Some(c) = string.char_at(index) {
            match c {
                | '\\' => match string.char_at(index + 1) {
                    | Some('"') => {
                        write('"');
                        index += 2;
                    },
                    | Some('\\') => {
                        write('\\');
                        index += 2;
                    },
                    | _ => {
                        // Only escaped quotes and backslashes are removed by
                        // C99 §6.10.9p1. Preserve other escapes and progress.
                        write('\\');
                        index += 1;
                    },
                },
                | _ => {
                    write(c);
                    index += c.len_utf8();
                },
            }
        }
        // A final pending quote is the trailing quote, which is dropped.
        text.push(b'\n');
        let text = text.into_slice();
        // SAFETY: only complete UTF-8 encodings of characters were written.
        unsafe { std::str::from_utf8_unchecked(text) }
    }

    /// Resolves an include name the way GCC and Clang do (C99 §6.10.2p2-3
    /// leave the places implementation-defined):
    ///
    /// 1. `"name"` first looks beside the file containing the directive (for
    ///    `--input`, whose name has no directory, that is the working
    ///    directory), then in each `--iquote` directory.
    /// 2. Both forms then search each `--isystem` directory, `CPATH`, and
    ///    `C_INCLUDE_PATH`.
    ///
    /// The process working directory is never searched implicitly, so the
    /// result depends on the source tree rather than where the compiler runs.
    ///
    /// `including_file` is the file containing the directive, captured before
    /// a macro-expanded operand can switch to its definition's tokenizer.
    fn find_header_from_path(
        &mut self,
        including_file: u32,
        operand: SourceVectors,
        path: &Path,
        is_system_header: bool,
    ) -> Option<u32> {
        let source_file_index = if path.is_absolute() {
            path.is_file()
                .then(|| self.context.intern_source_file(path))
        } else {
            // The including file's directory and the quote directories come
            // first for `"…"` names.
            let directories = self
                .context
                .include_search_directories(including_file, is_system_header);
            let mut found = None;
            for directory in directories.clone() {
                // Each candidate is spelled in the expansion arena and taken
                // back before the next one, so a lookup leaves nothing there.
                let mut buffer = ArenaVec::new_in(self.scratch);
                let candidate = join_path(&mut buffer, directory, path);
                if candidate.is_file() {
                    found = Some(self.context.intern_source_file(candidate));
                    break;
                }
            }
            if found.is_none() {
                let searched = self.context.tu_arena().alloc_slice_fill_iter(directories);
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::HeaderNotFound {
                        name: self.context.diagnostic_text(&path.to_string_lossy()),
                        is_system_header,
                        searched,
                    },
                    source_vectors: operand,
                });
                return None;
            }
            found
        };
        let Some(source_file_index) = source_file_index else {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HeaderNotFound {
                    name: self.context.diagnostic_text(&path.to_string_lossy()),
                    is_system_header,
                    searched: &[],
                },
                source_vectors: operand,
            });
            return None;
        };
        if self.state.once_set.contains(&source_file_index) {
            None
        } else {
            Some(source_file_index)
        }
    }

    /// Judges an `#include` operand by its first token as written, without
    /// consuming it or reporting what reading it reports.
    fn peek_include_operand(&mut self) -> IncludeOperand {
        // Only a source file's own text can be read between the delimiters.
        if !matches!(
            self.tokenizer_stack.last().map(|frame| &frame.frame_type),
            Some(TokenizerFrameType::SourceFile { .. })
        ) {
            return IncludeOperand::Other;
        }
        let saved = self.tokenizer.clone();
        let ignored = self.context.ignore_tokenizer_errors();
        self.context.set_ignore_tokenizer_errors(true);
        let first = Self::next_ignore_whitespace(&mut self.tokenizer, self.context);
        self.context.set_ignore_tokenizer_errors(ignored);
        self.tokenizer = saved;
        let Some(first) = first else {
            return IncludeOperand::Other;
        };
        let spelling = self.context.string_cache.at(first.contents);
        if first.kind == PreprocessorTokenType::String && spelling.starts_with('"') {
            IncludeOperand::Quoted
        } else if spelling.starts_with('<') {
            IncludeOperand::Angle
        } else {
            IncludeOperand::Other
        }
    }

    /// Reads a `<…>` operand as written: tokens through the first one that
    /// contains `>`. The name is the source text between the delimiters, so
    /// it keeps the whitespace that the tokens between them do not spell.
    fn read_written_angle_header(&mut self, directive: PreprocessorToken) -> HeaderName<'x> {
        let open = Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
            .expect("the operand was peeked");
        let open_vector = self
            .context
            .first_source_vector(open.source_vectors)
            .clone();
        let file = open_vector.source_file_index;
        // `#line` may rename the file; its text is the physical file's.
        let physical = self.physical_source_file_index();
        // Where the operand ends when no token closes it.
        let mut end = open_vector.end();
        let mut closing = None;
        let mut extra_after_closing = None;
        loop {
            let Some(token) = self.tokenizer.next_item(self.context) else {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing include directive",
                    ),
                    source_vectors: directive.source_vectors,
                });
                self.current_is_newline = true;
                break;
            };
            if token.kind == PreprocessorTokenType::Newline {
                self.current_is_newline = true;
                break;
            }
            let vector = self
                .context
                .first_source_vector(token.source_vectors)
                .clone();
            if self.context.string_cache.at(token.contents).contains('>') {
                let source = self
                    .context
                    .source_text(physical)
                    .expect("source files record their text");
                let characters = logical_characters(self.scratch, source, vector.range());
                closing = characters
                    .iter()
                    .position(|character| character.character == '>')
                    .map(|index| {
                        extra_after_closing = characters
                            .get(index + 1)
                            .map(|character| (character.index, character.length));
                        characters[index].index
                    });
                if closing.is_some() {
                    break;
                }
            }
            end = vector.end();
        }
        let anchor = SourcePosition {
            index:  open_vector.index as usize,
            line:   open_vector.line,
            column: open_vector.column,
        };
        let (name, invalid, unclosed_at, extra_after_closing) = {
            let source = self
                .context
                .source_text(physical)
                .expect("source files record their text");
            let open_index = logical_characters(self.scratch, source, open_vector.range())
                .first()
                .expect("the operand starts with `<`")
                .index;
            let characters =
                logical_characters(self.scratch, source, open_index + 1..closing.unwrap_or(end));
            let written = header_name_from_source(self.scratch, source, anchor, &characters, true);
            let unclosed_at = closing
                .is_none()
                .then(|| position_after(source, anchor, end));
            let extra_after_closing = extra_after_closing
                .map(|(index, length)| (position_after(source, anchor, index), length));
            (
                written.name,
                written.invalid,
                unclosed_at,
                extra_after_closing,
            )
        };
        let length = closing.map_or(end, |index| index + 1) - anchor.index;
        // Lexing a terminal `>` may already have read the supplied final
        // newline while looking for `>=` or `>>`. Do not ask phase 4 for
        // another token: that would pop this file and read its parent.
        if closing.is_some_and(|index| {
            self.context
                .source_text(physical)
                .is_some_and(|source| index + 1 == source.len())
        }) {
            self.current_is_newline = true;
        }
        HeaderName {
            name,
            is_system_header: true,
            source_vectors: self.context.create_source_vectors(anchor, file, length),
            missing_delimiter: unclosed_at
                .map(|position| ('>', self.context.create_source_vectors(position, file, 0))),
            invalid: invalid.map(|(sequence, start, length)| {
                (
                    sequence,
                    self.context.create_source_vectors(start, file, length),
                )
            }),
            backslash: None,
            extra_tokens: extra_after_closing.map(|(position, length)| {
                self.context.create_source_vectors(position, file, length)
            }),
        }
    }

    /// Reads a `"…"` operand as written: one string literal, whose source
    /// text between its quotes is the name.
    fn read_written_quoted_header(&mut self) -> HeaderName<'x> {
        let token = Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
            .expect("the operand was peeked");
        let vector = self
            .context
            .first_source_vector(token.source_vectors)
            .clone();
        let file = vector.source_file_index;
        // `#line` may rename the file; its text is the physical file's.
        let physical = self.physical_source_file_index();
        let anchor = SourcePosition {
            index:  vector.index as usize,
            line:   vector.line,
            column: vector.column,
        };
        let allow_backslash =
            self.context.configuration.extension_policy() != ExtensionPolicy::Deny;
        let (written, unclosed_at, escaped_closing_quote, trailing) = {
            let source = self
                .context
                .source_text(physical)
                .expect("source files record their text");
            let characters = logical_characters(self.scratch, source, vector.range());
            // The first character is the opening quote. With the backslash
            // extension, a backslash is ordinary header-name text, so the
            // first following quote closes the header even if phase 3 lexed
            // it as an escaped quote.
            let mut close = characters.len();
            let mut index = 1;
            let mut escaped_closing_quote = false;
            while index < characters.len() {
                match characters[index].character {
                    | '\\' if allow_backslash => index += 1,
                    | '\\' => index += 2,
                    | '"' => {
                        close = index;
                        escaped_closing_quote =
                            index > 1 && characters[index - 1].character == '\\';
                        break;
                    },
                    | _ => index += 1,
                }
            }
            let written = header_name_from_source(
                self.scratch,
                source,
                anchor,
                &characters[1.min(close)..close],
                false,
            );
            // Under Deny, a trailing backslash escapes the quote during
            // lexing and leaves the header name unterminated too.
            let unclosed_at =
                (close == characters.len()).then(|| position_after(source, anchor, vector.end()));
            // Closing at an escaped quote can leave text of the same string
            // token after the header name: the header ends at its quote, and
            // the first character that is not whitespace starts the extra
            // tokens, as glued text after `>` does.
            let trailing = characters
                .get(close + 1..)
                .filter(|rest| !rest.is_empty())
                .map(|rest| {
                    let close = &characters[close];
                    (
                        close.index + close.length - anchor.index,
                        rest.iter()
                            .find(|character| !character.character.is_whitespace())
                            .map(|character| {
                                (
                                    position_after(source, anchor, character.index),
                                    character.length,
                                )
                            }),
                    )
                });
            (written, unclosed_at, escaped_closing_quote, trailing)
        };
        if escaped_closing_quote {
            self.context.withdraw_quoted_header_lexer_error(&vector);
        }
        let source_vectors = match trailing {
            | Some((length, _)) => self.context.create_source_vectors(anchor, file, length),
            | None => token.source_vectors,
        };
        let extra_tokens = trailing
            .and_then(|(_, extra)| extra)
            .map(|(position, length)| self.context.create_source_vectors(position, file, length));
        if unclosed_at.is_some()
            && self
                .context
                .source_text(physical)
                .is_some_and(|source| vector.end() == source.len())
        {
            self.current_is_newline = true;
        }
        HeaderName {
            name: written.name,
            is_system_header: false,
            source_vectors,
            missing_delimiter: unclosed_at
                .map(|position| ('"', self.context.create_source_vectors(position, file, 0))),
            invalid: written.invalid.map(|(sequence, start, length)| {
                (
                    sequence,
                    self.context.create_source_vectors(start, file, length),
                )
            }),
            backslash: written
                .backslash
                .map(|(start, length)| self.context.create_source_vectors(start, file, length)),
            extra_tokens,
        }
    }

    /// Reads an operand not written as a header name: macros that expand to
    /// one (C99 §6.10.2p4), whose tokens are combined by their spellings.
    fn read_expanded_header(&mut self, directive: PreprocessorToken) -> Option<HeaderName<'x>> {
        let include_string =
            self.expect_token_without_rewind::<true>(
                |preprocessor, token| {
                    let spelling = preprocessor.context.string_cache.at(token.contents);
                    (token.kind == PreprocessorTokenType::String && spelling.starts_with('"'))
                        || spelling.starts_with('<')
                },
                |_, token| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(
                                token.kind,
                            ),
                        source_vectors: token.source_vectors,
                    })
                },
                "parsing include directive",
            )?;
        if include_string.kind == PreprocessorTokenType::String {
            let spelling = self.context.string_cache.at(include_string.contents);
            let name = spelling.strip_prefix('"').unwrap_or(spelling);
            let name = &*self
                .scratch
                .alloc_str(name.strip_suffix('"').unwrap_or(name));
            let invalid = invalid_header_sequence(name, false).map(|(offset, sequence)| {
                (
                    sequence,
                    expanded_header_sequence_source(
                        self.context,
                        include_string.source_vectors,
                        offset + 1,
                        sequence,
                    ),
                )
            });
            let backslash = name.contains('\\').then_some(include_string.source_vectors);
            return Some(HeaderName {
                name,
                is_system_header: false,
                source_vectors: include_string.source_vectors,
                missing_delimiter: None,
                invalid,
                backslash,
                extra_tokens: None,
            });
        }
        let mut contents = ArenaString::new_in(self.scratch);
        contents.push_str(&self.context.string_cache.at(include_string.contents)[1..]);
        // Each name byte belongs to a token; keep that token's provenance so
        // an invalid sequence does not share the whole operand's location.
        let mut token_spans = ArenaVec::new_in(self.scratch);
        token_spans.push((0..contents.len(), include_string.source_vectors, 1));
        let start_index = Context::duplicate_source_vectors(
            &mut self.context.source_vectors.0,
            include_string.source_vectors,
        );
        let mut closed = false;
        loop {
            match self.next_preprocessor_token::<false>() {
                | Some(token) => {
                    if token.kind == PreprocessorTokenType::Newline {
                        break;
                    }
                    let token_contents = self.context.string_cache.at(token.contents);
                    _ = Context::duplicate_source_vectors(
                        &mut self.context.source_vectors.0,
                        token.source_vectors,
                    );
                    if let Some(idx) = token_contents.find('>') {
                        let start = contents.len();
                        contents.push_str(&token_contents[..idx]);
                        token_spans.push((start..contents.len(), token.source_vectors, 0));
                        closed = true;
                        break;
                    }
                    let start = contents.len();
                    contents.push_str(self.context.string_cache.at(token.contents));
                    token_spans.push((start..contents.len(), token.source_vectors, 0));
                },
                | None => {
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                            "parsing include directive",
                        ),
                        source_vectors: directive.source_vectors,
                    });
                    break;
                },
            }
        }
        let source_vectors =
            SourceVectors::new(start_index, self.context.source_vectors.0.len() as u32);
        let name = contents.into_str();
        let invalid = invalid_header_sequence(name, true).map(|(offset, sequence)| {
            let (range, token_source, token_offset) = token_spans
                .iter()
                .find(|(range, ..)| range.contains(&offset))
                .expect("invalid character belongs to a header token");
            (
                sequence,
                expanded_header_sequence_source(
                    self.context,
                    *token_source,
                    offset - range.start + token_offset,
                    sequence,
                ),
            )
        });
        Some(HeaderName {
            name,
            is_system_header: true,
            source_vectors,
            missing_delimiter: (!closed).then_some(('>', source_vectors)),
            invalid,
            backslash: None,
            extra_tokens: None,
        })
    }

    /// Whether the remainder of this directive ends before another token.
    /// Inspect the active expansion frames and the including file without
    /// advancing them; reading through the file's end here could enter its
    /// parent's next line before the include frame is pushed.
    fn include_tail_is_empty(&mut self) -> bool {
        let ignored = self.context.ignore_tokenizer_errors();
        self.context.set_ignore_tokenizer_errors(true);
        let mut empty = false;
        'frames: for index in (0..self.tokenizer_stack.len()).rev() {
            let frame = &self.tokenizer_stack[index];
            let mut tokenizer = if index + 1 == self.tokenizer_stack.len() {
                self.tokenizer.clone()
            } else {
                frame.tokenizer.clone()
            };
            loop {
                match tokenizer.next_item(self.context) {
                    | Some(token) if token.kind == PreprocessorTokenType::Whitespace => (),
                    | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                        // A replacement-list newline ends that expansion,
                        // not the directive in its caller's source file.
                        empty = matches!(frame.frame_type, TokenizerFrameType::SourceFile { .. });
                        break;
                    },
                    | Some(_) => break 'frames,
                    | None if matches!(frame.frame_type, TokenizerFrameType::SourceFile { .. }) => {
                        empty = true;
                        break;
                    },
                    | None => break,
                }
            }
            if empty || matches!(frame.frame_type, TokenizerFrameType::SourceFile { .. }) {
                break;
            }
        }
        self.context.set_ignore_tokenizer_errors(ignored);
        empty
    }

    fn parse_include_directive(&mut self, directive: PreprocessorToken) {
        let including_file = self.physical_source_file_index();
        let header = match self.peek_include_operand() {
            | IncludeOperand::Angle => Some(self.read_written_angle_header(directive)),
            | IncludeOperand::Quoted => Some(self.read_written_quoted_header()),
            | IncludeOperand::Other => self.read_expanded_header(directive),
        };
        let Some(header) = header else {
            if !self.current_is_newline {
                self.skip_and_expand_until_newline();
            }
            return;
        };
        if header.missing_delimiter.is_none()
            && !self.current_is_newline
            && self.include_tail_is_empty()
        {
            self.current_is_newline = true;
        }
        if let Some((delimiter, source_vectors)) = header.missing_delimiter {
            self.context.preprocessor_error(PreprocessorError {
                error_type: PreprocessorErrorType::UnterminatedHeaderName(delimiter),
                source_vectors,
            });
        }
        if let Some((sequence, source_vectors)) = header.invalid {
            self.context.preprocessor_error(PreprocessorError {
                error_type: PreprocessorErrorType::InvalidCharacterInHeaderName(sequence),
                source_vectors,
            });
        }
        let mut look_up = true;
        if let Some(source_vectors) = header.backslash {
            let policy = match self.context.configuration.standard() {
                | CStandard::C99 => self.context.configuration.extension_policy(),
            };
            if policy != ExtensionPolicy::Allow {
                self.context.preprocessor_error(PreprocessorError {
                    error_type: PreprocessorErrorType::BackslashInQuotedHeaderName(policy),
                    source_vectors,
                });
            }
            look_up = policy != ExtensionPolicy::Deny;
        }
        let header_source_index = if look_up {
            self.find_header_from_path(
                including_file,
                header.source_vectors,
                Path::new(header.name),
                header.is_system_header,
            )
        } else {
            None
        };
        if let Some(source_vectors) = header.extra_tokens {
            self.context.preprocessor_error(PreprocessorError {
                error_type: PreprocessorErrorType::ExtraTokensAfterIncludeDirective,
                source_vectors,
            });
            if !self.current_is_newline {
                self.skip_and_expand_until_newline();
            }
        } else if !self.current_is_newline
            && self
                // An extra token is discarded with the directive tail. Rewinding
                // after an expansion frame ends can target a different source.
                .expect_token_without_rewind::<true>(
                    |_, token| token.kind == PreprocessorTokenType::Newline,
                    |_, token| {
                        ControlFlow::Break(PreprocessorError {
                            error_type:     PreprocessorErrorType::ExtraTokensAfterIncludeDirective,
                            source_vectors: token.source_vectors,
                        })
                    },
                    "parsing include directive",
                )
                .is_none()
        {
            self.skip_and_expand_until_newline();
        }
        let Some(header_source_index) = header_source_index else {
            return;
        };
        // The main source contributes one frame. Macro frames and headers
        // whose processing has finished do not consume the nesting limit.
        let source_depth = self
            .tokenizer_stack
            .iter()
            .filter(|frame| matches!(frame.frame_type, TokenizerFrameType::SourceFile { .. }))
            .count();
        if source_depth > MAX_INCLUDE_NESTING {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::IncludeNestingLimitExceeded(
                    MAX_INCLUDE_NESTING,
                ),
                source_vectors: directive.source_vectors,
            });
            return;
        }
        let Ok(header_string) = self
            .context
            .read_source_file(header_source_index)
            .map_err(|e| {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::HeaderFileInaccessible(
                        self.context.diagnostic_format(format_args!("{e}")),
                    ),
                    source_vectors: directive.source_vectors,
                });
            })
        else {
            return;
        };
        let tokenizer =
            self.state
                .lexed_files
                .open(self.context, header_source_index, header_string);
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::SourceFile {
                conditional_base:           self.state.open_conditionals.len(),
                physical_source_file_index: header_source_index,
            },
            tokenizer,
        });
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_define_directive(&mut self, _directive: PreprocessorToken) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            |_, t| t.kind.is_identifier(),
            |_, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInDefineDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing define directive",
        ) else {
            self.skip_until_newline();
            return;
        };
        let old_definition = self
            .state
            .macro_definitions
            .get(&name.identifier_id(self.context))
            .cloned();
        let tokenizer = self.tokenizer.clone();
        let old_tokenizer = match old_definition {
            | None => None,
            | Some(ref v) => match v {
                | MacroDefinition::FunctionLike { tokenizer, .. }
                | MacroDefinition::ObjectLike { tokenizer, .. } => Some(tokenizer.clone()),
                | MacroDefinition::BuiltIn => {
                    // C99 §6.10.8p4: predefined macro names cannot be
                    // redefined, so the built-in definition stays in effect.
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::RedefinitionOfBuiltInMacro(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                        ),
                        source_vectors: name.source_vectors,
                    });
                    self.skip_until_newline();
                    return;
                },
            },
        };
        // A function-like definition requires '(' immediately after its name.
        // The probe may consume the directive's newline when the replacement
        // list is empty; remember that so the next line is not skipped too.
        let mut line_ended = false;
        let opening_paren = match self.tokenizer.next_item(self.context) {
            | Some(
                token @ PreprocessorToken {
                    kind: PreprocessorTokenType::OpeningParenthesis,
                    ..
                },
            ) => Some(token),
            | Some(token) => {
                line_ended = token.kind == PreprocessorTokenType::Newline;
                None
            },
            | None => {
                line_ended = true;
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing macro definition",
                    ),
                    source_vectors: name.source_vectors,
                });
                None
            },
        };
        if opening_paren.is_some() {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => true,
                | MacroDefinition::FunctionLike { .. } => false,
                | MacroDefinition::BuiltIn => {
                    unreachable!("The case where name is a built-in macro is handled above")
                },
            }) {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                        ),
                    source_vectors: name.source_vectors,
                });
            }
            // Collected in the expansion arena; the definition keeps a copy.
            let mut argument_names = ArenaVec::new_in(self.scratch);
            let mut is_variadic = false;
            loop {
                let Some(name_or_ellipsis) = self.expect_token_from_previous_phase::<true>(
                    |_, t| {
                        t.kind.is_identifier()
                            || t.kind == PreprocessorTokenType::Ellipsis
                            || (t.kind == PreprocessorTokenType::ClosingParenthesis
                                && argument_names.is_empty())
                    },
                    |_, token| {
                        ControlFlow::Break(PreprocessorError {
                            error_type:
                                PreprocessorErrorType::ExpectedIdentifierInMacroDefinition(
                                    token.kind,
                                ),
                            source_vectors: token.source_vectors,
                        })
                    },
                    "parsing macro definition",
                ) else {
                    break;
                };
                if is_variadic {
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::VariadicMacroMustBeLastParameter(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                        ),
                        source_vectors: name.source_vectors,
                    });
                }
                if name_or_ellipsis.kind == PreprocessorTokenType::Ellipsis {
                    is_variadic = true;
                } else if name_or_ellipsis.kind.is_identifier() {
                    argument_names.push(name_or_ellipsis.identifier_id(self.context));
                } else if name_or_ellipsis.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
                let Some(comma_or_closing_parent) = self.expect_token_from_previous_phase::<true>(
                    |_, t| t.kind == PreprocessorTokenType::Comma || t.kind == PreprocessorTokenType::ClosingParenthesis,
                    |_, token|
                        ControlFlow::Break(PreprocessorError {
                                error_type:     PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(token.kind),
                                source_vectors: token.source_vectors,
                            },
                        )
                    ,
                    "parsing macro definition",
                ) else {break;};
                if comma_or_closing_parent.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
            }
            let tokenizer = self.tokenizer.clone();
            _ = self.state.macro_definitions.insert(
                name.identifier_id(self.context),
                MacroDefinition::FunctionLike {
                    tokenizer: self.state.lexed_files.persist(&tokenizer),
                    argument_names: self.state.arena.alloc_slice_copy(&argument_names),
                    is_variadic,
                },
            );
        } else {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => false,
                | MacroDefinition::FunctionLike { .. } => true,
                | MacroDefinition::BuiltIn => {
                    unreachable!("The case where name is a built-in macro is handled above")
                },
            }) {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                        ),
                    source_vectors: name.source_vectors,
                });
            }
            _ = self.state.macro_definitions.insert(
                name.identifier_id(self.context),
                MacroDefinition::ObjectLike {
                    tokenizer: self.state.lexed_files.persist(&tokenizer),
                },
            );
        }
        let mut last = Option::<PreprocessorToken>::None;
        if let Some(mut old_tokenizer) = old_tokenizer {
            // Compare replacement lists body to body, and parameter lists
            // separately (C99 §6.10.3p2).
            let (mut new_tokenizer, parameters_match) = match (
                self.state
                    .macro_definitions
                    .get(&name.identifier_id(self.context)),
                &old_definition,
            ) {
                | (
                    Some(MacroDefinition::FunctionLike {
                        tokenizer,
                        argument_names,
                        is_variadic,
                    }),
                    Some(MacroDefinition::FunctionLike {
                        argument_names: old_argument_names,
                        is_variadic: old_is_variadic,
                        ..
                    }),
                ) => (
                    tokenizer.clone(),
                    argument_names == old_argument_names && is_variadic == old_is_variadic,
                ),
                | (
                    Some(
                        MacroDefinition::FunctionLike { tokenizer, .. }
                        | MacroDefinition::ObjectLike { tokenizer },
                    ),
                    _,
                ) => (tokenizer.clone(), true),
                | _ => (tokenizer, true),
            };
            let mut error_has_been_generated = false;
            if !parameters_match {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                        self.context
                            .diagnostic_text(self.context.string_cache.at(name.contents)),
                    ),
                    source_vectors: name.source_vectors,
                });
                error_has_been_generated = true;
            }
            loop {
                let old_next = old_tokenizer.next_item(self.context);
                let new_next = new_tokenizer.next_item(self.context);
                if let Some(t) = new_next.as_ref()
                    && t.kind == PreprocessorTokenType::HashHash
                    && last.is_none()
                {
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator,
                        source_vectors: t.source_vectors,
                    });
                }
                if !same_replacement_token(self.context, old_next.as_ref(), new_next.as_ref())
                    && !error_has_been_generated
                {
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                                self.context
                                    .diagnostic_text(self.context.string_cache.at(name.contents)),
                            ),
                        source_vectors: name.source_vectors,
                    });
                    error_has_been_generated = true;
                }
                if !new_next
                    .as_ref()
                    .is_some_and(|t| t.kind != PreprocessorTokenType::Newline)
                {
                    if last
                        .as_ref()
                        .is_some_and(|t| t.kind == PreprocessorTokenType::HashHash)
                    {
                        self.context.preprocessor_error(PreprocessorError {
                            error_type:
                                PreprocessorErrorType::MissingRightHandSideOfHashHashOperator,
                            source_vectors: last.unwrap().source_vectors,
                        });
                    }
                    break;
                }
                last = new_next;
            }
        }
        if !line_ended {
            loop {
                match self.tokenizer.next_item(self.context) {
                    | Some(token) if token.kind == PreprocessorTokenType::Newline => break,
                    | None => break,
                    | Some(_) => continue,
                }
            }
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_undef_directive(&mut self, _directive: PreprocessorToken) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            |_, t| t.kind.is_identifier(),
            |_, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInUndefDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing undef directive",
        ) else {
            self.skip_until_newline();
            return;
        };
        _ = self
            .state
            .macro_definitions
            .remove(&name.identifier_id(self.context));
        if self
            .expect_token_from_previous_phase::<true>(
                |_, t| t.kind == PreprocessorTokenType::Newline,
                |_, token| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:     PreprocessorErrorType::ExpectedNewlineAfterUndefDirective(
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    })
                },
                "parsing undef directive",
            )
            .is_none()
        {
            self.skip_until_newline();
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_line_directive(&mut self, _directive: PreprocessorToken) {
        let Some(token) = self.expect_token_without_rewind::<true>(
            |_, t| t.kind == PreprocessorTokenType::Number,
            |_, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNumberInLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        ) else {
            if !self.current_is_newline {
                self.skip_and_expand_until_newline();
            }
            return;
        };
        let digits = self
            .context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0');
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveIsNotASimpleDigitSequence,
                source_vectors: token.source_vectors,
            });
            self.skip_and_expand_until_newline();
            return;
        }
        let value = digits.bytes().try_fold(0u32, |value, digit| {
            value
                .checked_mul(10)?
                .checked_add(u32::from(digit - b'0'))
                .filter(|value| i32::try_from(*value).is_ok())
        });
        if value.is_none() {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveNumberTooLarge(
                    self.context.diagnostic_text(digits),
                ),
                source_vectors: token.source_vectors,
            });
        }
        let name = self.expect_token_without_rewind::<true>(
            |_, t| {
                matches!(
                    t.kind,
                    PreprocessorTokenType::String
                        | PreprocessorTokenType::GeneratedString
                        | PreprocessorTokenType::Newline
                )
            },
            |_, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        );
        let Some(name) = name else {
            self.skip_and_expand_until_newline();
            return;
        };
        let mut filename = None;
        if let t @ PreprocessorToken {
            kind: PreprocessorTokenType::String | PreprocessorTokenType::GeneratedString,
            ..
        } = name
        {
            // Unlike include names, #line names use normal string-literal
            // decoding. Save the result until the directive is complete.
            if let Some(token) = self.map_preprocessor_token(t)
                && let TokenType::String(
                    StringTokenType::String(contents) | StringTokenType::WideString(contents),
                ) = token.kind
            {
                let wide = matches!(
                    token.kind,
                    TokenType::String(StringTokenType::WideString(_))
                );
                filename = self
                    .context
                    .literal_text_in(self.scratch, contents, wide)
                    .filter(|text| !text.contains('\0'));
                if filename.is_none() {
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::InvalidLineFilename,
                        source_vectors: token.source_vectors,
                    });
                }
            }
            if self
                .expect_token_without_rewind::<true>(
                    |_, t| t.kind == PreprocessorTokenType::Newline,
                    |_, t| {
                        ControlFlow::Break(PreprocessorError {
                            error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(
                                t.kind,
                            ),
                            source_vectors: t.source_vectors,
                        })
                    },
                    "parsing line directive",
                )
                .is_none()
            {
                self.skip_and_expand_until_newline();
                return;
            }
        }
        // The newline ends any macro operand frames and returns us to the
        // containing source file. C99 §6.10.4p3 assigns the following line.
        if let Some(value) = value {
            self.set_line(value);
        }
        if let Some(filename) = filename {
            let source_file_index = self.context.intern_source_file(Path::new(filename));
            self.set_source_file_index(source_file_index);
        }
    }

    fn parse_error_directive(&mut self, directive: PreprocessorToken) {
        let mut contents = ArenaString::new_in(self.scratch);
        // A directive ending at end of file is complete; the missing final
        // newline is diagnosed on its own.
        while let Some(token) = self.tokenizer.next_item(self.context) {
            if token.kind == PreprocessorTokenType::Newline {
                break;
            }
            contents.push_str(
                self.context
                    .string_cache
                    .at(token.contents)
                    .trim_end_matches('\0'),
            );
        }
        self.context.preprocessor_error(PreprocessorError {
            error_type:     PreprocessorErrorType::ErrorDirective(
                self.context.diagnostic_text(&contents),
            ),
            source_vectors: directive.source_vectors,
        });
    }

    pub(super) fn parse_pragma_directive(&mut self, _directive: PreprocessorToken) -> bool {
        let mut consumed_newline = false;
        let mut completed_stdc = false;
        'base: loop {
            let Some(token) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
            else {
                let source_vectors = self.current_location();
                self.context.preprocessor_error(PreprocessorError {
                    error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing pragma directive",
                    ),
                    source_vectors,
                });
                break 'base;
            };
            match token.kind {
                | PreprocessorTokenType::Whitespace => continue 'base,
                | PreprocessorTokenType::Newline => {
                    consumed_newline = true;
                    break 'base;
                },
                | PreprocessorTokenType::Identifier
                | PreprocessorTokenType::UniversalIdentifier => {
                    match self.context.string_cache.at(token.contents) {
                        | "once" => {
                            if !self.current_is_header() {
                                self.context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::PragmaOnceInNonHeader,
                                    source_vectors: token.source_vectors,
                                });
                            }
                            _ = self
                                .state
                                .once_set
                                .insert(self.physical_source_file_index());
                            match Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    consumed_newline = true;
                                    break 'base;
                                },
                                | Some(t) => {
                                    self.context.preprocessor_error(PreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::ExtraTokensAfterPragmaOnce(
                                                t.kind,
                                            ),
                                        source_vectors: token.source_vectors,
                                    });
                                    break 'base;
                                },
                                | None => {
                                    let source_vectors = self.current_location();
                                    self.context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                        },
                        | "STDC" => {
                            match Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    consumed_newline = true;
                                    self.context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutArgument,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = self.context.string_cache.at(token.contents);
                                    if !token.kind.is_identifier()
                                        || !matches!(
                                            s,
                                            "FP_CONTRACT" | "FENV_ACCESS" | "CX_LIMITED_RANGE"
                                        )
                                    {
                                        self.context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnknownPragmaSTDCArgument(
                                                    self.context.diagnostic_text(s),
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = self.current_location();
                                    self.context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }

                            match Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    consumed_newline = true;
                                    self.context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutOnOffSwitch,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = self.context.string_cache.at(token.contents);
                                    if !token.kind.is_identifier()
                                        || !matches!(s, "ON" | "OFF" | "DEFAULT")
                                    {
                                        self.context.preprocessor_error(PreprocessorError {
                                                    error_type:     PreprocessorErrorType::MissingOnOffSwitchInSTDCPragma(self.context.diagnostic_text(s)),
                                                    source_vectors: token.source_vectors,
                                                },
                                            );
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = self.current_location();
                                    self.context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                            completed_stdc = true;
                        },
                        | _ => {
                            if !completed_stdc {
                                // An unknown pragma is ignored as a whole.
                                // Preserve the caller's line state for _Pragma,
                                // whose payload uses a temporary tokenizer.
                                let line_state = (self.last_was_newline, self.current_is_newline);
                                self.skip_until_newline();
                                (self.last_was_newline, self.current_is_newline) = line_state;
                                consumed_newline = true;
                            }
                            break 'base;
                        },
                    }
                },
                | _ => {
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::UnknownPragmaDirective,
                        source_vectors: token.source_vectors,
                    });
                    break 'base;
                },
            }
        }
        consumed_newline
    }
}
