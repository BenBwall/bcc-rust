//! Directive dispatch and the non-conditional directives.
//!
//! C99: the `group-part`, `control-line`, and `non-directive` grammar of
//! §6.10 paragraph 1, pp. 145-146; PDF pp. 157-158 (also §A.3, pp. 416-418;
//! PDF pp. 428-430), and the directives `#include` (§6.10.2, pp. 149-151;
//! PDF pp. 161-163), `#define` (§6.10.3, pp. 151-153; PDF pp. 163-165),
//! `#undef` (§6.10.3.5, p. 155; PDF p. 167), `#line` (§6.10.4, p. 158; PDF
//! p. 170), `#error` (§6.10.5, p. 159; PDF p. 171), `#pragma` (§6.10.6,
//! p. 159; PDF p. 171), and the null directive (§6.10.7, p. 160; PDF
//! p. 172). Conditional directives are in `conditional`.
//!
//! Directive tokens are not macro-replaced unless a clause says so (§6.10
//! paragraph 7, p. 147; PDF p. 159). Of the directives here, only the
//! operands of `#include` and `#line` are; `#pragma` operands are not, which
//! footnote 152 permits (§6.10.6 paragraph 1, p. 159; PDF p. 171).

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
        ExtensionPolicy,
        Feature,
    },
    translation_phases::{
        Context,
        SourcePosition,
        SourceVectors,
        StrExt,
        TranslationError,
        TranslationPhase,
        preprocessor_tokenizer::{
            LogicalCharacter,
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
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

/// Maximum live header depth, excluding the main source file. This is a
/// recursion guard for unguarded/self-including headers: it bounds the work
/// before diagnosing the include and continuing with the caller. The 200
/// levels leave substantial headroom above C99 §5.2.4.1's required 15 while
/// keeping a runaway include chain finite.
///
/// C99: the nesting limit is implementation-defined, §6.10.2 paragraph 6,
/// p. 150; PDF p. 162; the minimum is in §5.2.4.1 paragraph 1, p. 21; PDF
/// p. 33.
const MAX_INCLUDE_NESTING: usize = 200;

/// Whether `name` is a macro the implementation predefines beyond the names
/// of C99 §6.10.8, p. 160; PDF p. 172. §6.10.8 paragraph 4 does not protect
/// it, so `#undef` and `#define` apply as to any macro, as in GCC.
fn implementation_macro(name: &str) -> bool {
    name == "__STRICT_ANSI__"
}

/// Reads the next token of a macro definition's replacement list from
/// `source`, with whether whitespace separates it from the token before it.
/// Returns `None` at the line's end, so trailing whitespace is not part of
/// the list.
///
/// C99: §6.10.3 paragraphs 1 and 7, p. 151; PDF p. 163. A comment
/// is already part of the whitespace token that replaces it (§5.1.1.2
/// paragraph 1, phase 3, p. 10; PDF p. 22).
fn next_replacement_token(
    context: &mut Context<'_>,
    source: &mut TokenSource<'_>,
) -> Option<(PreprocessorToken, bool)> {
    let mut separated = false;
    loop {
        match source.next_item(context)? {
            | token if token.kind == PreprocessorTokenType::Whitespace => separated = true,
            | token if token.kind == PreprocessorTokenType::Newline => return None,
            | token => return Some((token, separated)),
        }
    }
}

/// Whether two replacement-list tokens are the same preprocessing token with
/// the same spelling.
///
/// C99: §6.10.3 paragraph 1, p. 151; PDF p. 163.
fn same_replacement_token(
    context: &Context<'_>,
    old: &PreprocessorToken,
    new: &PreprocessorToken,
) -> bool {
    old.kind == new.kind
        && context.string_cache.at(old.contents) == context.string_cache.at(new.contents)
}

/// How an `#include` operand is written, judged from its first token.
enum IncludeOperand {
    /// `<…>`, read from the source text.
    ///
    /// C99: §6.10.2 paragraph 2, p. 149; PDF p. 161.
    Angle,
    /// `"…"`, read from the source text.
    ///
    /// C99: §6.10.2 paragraph 3, pp. 149-150; PDF pp. 161-162.
    Quoted,
    /// Anything else, which macro replacement must turn into a header name.
    ///
    /// C99: §6.10.2 paragraph 4, p. 150; PDF p. 162.
    Other,
}

/// The header name of an `#include` directive.
///
/// C99: `header-name`, §6.4.7 paragraph 1, p. 64; PDF p. 76. The lexer forms
/// no header names; the directive reads them from source text, the one
/// context besides `#pragma` where §6.4.7 paragraph 3 recognizes them.
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
    /// extension accepts. §6.4.7p3 leaves its behavior undefined; reading it
    /// as a path character is an extension (§4p6).
    backslash:         Option<SourceVectors>,
    /// A character after `>` in the token that closes a written angle name.
    extra_tokens:      Option<SourceVectors>,
}

/// The first sequence in `name` whose behavior in a header name C99 §6.4.7p3
/// leaves undefined, with its byte offset: `'`, `//`, or `/*` in either
/// form, and also `\` or `"` between `<` and `>`. A backslash in a `"…"`
/// name is the backslash extension, which the configured
/// [`ExtensionPolicy`] governs instead.
///
/// C99: §6.4.7 paragraph 3, pp. 64-65; PDF pp. 76-77.
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

impl<'tu, 'x> Expander<'_, 'tu, '_, 'x> {
    /// Executes the directive that `token`, a `#`, introduces.
    ///
    /// C99: §6.10 paragraphs 1-3, pp. 145-147; PDF pp. 157-159. A `#` begins
    /// a directive only at the start of a line; one found elsewhere is
    /// diagnosed and, for recovery, still read as a directive. A name that is
    /// not a directive makes a `non-directive`, to which C99 gives no
    /// meaning; it is diagnosed and skipped, as GCC and Clang do.
    /// `# new-line` is the null directive (§6.10.7 paragraph 1, p. 160; PDF
    /// p. 172).
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
            // Null directive (C99 §6.10.7p1).
            | PreprocessorTokenType::Newline => {
                self.resume_at_line_start();
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
            | name @ ("elifdef" | "elifndef")
                if self.context.configuration.accepts(Feature::Elifdef) =>
            {
                self.context.report_extension(
                    Feature::Elifdef,
                    if name == "elifdef" {
                        "#elifdef"
                    } else {
                        "#elifndef"
                    },
                    directive.source_vectors,
                );
                self.parse_elif_directive(directive);
            },
            | "else" => self.parse_else_directive(directive),
            | "endif" => self.parse_endif_directive(directive),
            | "include" => self.parse_include_directive(directive),
            | "include_next" => {
                self.context.report_extension(
                    Feature::IncludeNext,
                    "#include_next",
                    directive.source_vectors,
                );
                self.parse_include_directive(directive);
            },
            | "embed" if self.context.configuration.accepts(Feature::Embed) =>
                self.parse_embed_directive(directive),
            | "ident" | "sccs" => {
                self.context.report_extension(
                    Feature::IdentDirective,
                    "#ident/#sccs",
                    directive.source_vectors,
                );
                let operand = Self::next_ignore_whitespace(&mut self.tokenizer, self.context);
                if operand.is_none_or(|t| t.kind != PreprocessorTokenType::String) {
                    self.language_error(
                        "expected a string literal after #ident/#sccs",
                        directive.source_vectors,
                    );
                }
                if operand.is_none_or(|t| t.kind != PreprocessorTokenType::Newline) {
                    self.skip_until_newline();
                }
                self.resume_at_line_start();
            },
            | "define" => self.parse_define_directive(),
            | "undef" => self.parse_undef_directive(),
            | "line" => self.parse_line_directive(),
            | "error" => self.parse_error_directive(directive),
            | "warning"
                if self
                    .context
                    .configuration
                    .accepts(Feature::WarningDirective) =>
            {
                self.context.report_extension(
                    Feature::WarningDirective,
                    "#warning",
                    directive.source_vectors,
                );
                self.parse_error_directive(directive);
                self.resume_at_line_start();
            },
            | "pragma" => {
                if !self.parse_pragma_directive() {
                    self.skip_until_newline();
                }
                self.resume_at_line_start();
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
    ///
    /// C99: §6.10.9 paragraph 1, p. 161; PDF p. 173: destringizing drops an
    /// `L` prefix and the quotes, and turns `\"` into `"` and `\\` into `\`.
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

    /// The places a lookup of a header named in `including_file` visits, in
    /// order, each with its configured entry index (`None` for the including
    /// file's own directory). A `"…"` name first looks beside the including
    /// file (the working directory for `--input`, whose name has no
    /// directory), then in every configured entry; a `<…>` name skips both
    /// and the `-iquote` entries. An `#include_next` lookup that continues
    /// after a configured entry passes `start` and visits only the entries
    /// from it. See [`HeaderSearch`](crate::headers::HeaderSearch) for the
    /// order of the configured entries.
    ///
    /// The process working directory is never searched implicitly, so the
    /// result depends on the source tree rather than where the compiler runs.
    ///
    /// C99: the places are implementation-defined, §6.10.2 paragraphs 2-3,
    /// pp. 149-150; PDF pp. 161-162.
    pub(super) fn header_search_places(
        &self,
        including_file: u32,
        is_system_header: bool,
        start: Option<usize>,
    ) -> impl Iterator<Item = (Option<usize>, &'tu Path)> + Clone + use<'tu> {
        let local = (start.is_none() && !is_system_header)
            .then(|| self.context.get_source_file(including_file).parent())
            .flatten();
        local.into_iter().map(|directory| (None, directory)).chain(
            self.context
                .configured_include_directories(is_system_header, start)
                .map(|(index, directory)| (Some(index), directory)),
        )
    }

    /// The file `path` names in `directory`, interned, when it exists there.
    /// The resource directory holds exactly the embedded headers.
    pub(super) fn probe_header(&mut self, directory: &Path, path: &Path) -> Option<u32> {
        if directory == Path::new(crate::headers::DIRECTORY) {
            return crate::headers::text(path)
                .is_some()
                .then(|| self.context.intern_builtin_header(path));
        }
        // Each candidate is spelled in the expansion arena and taken back
        // before the next one, so a lookup leaves nothing there.
        let mut buffer = ArenaVec::new_in(self.scratch);
        let candidate = join_path(&mut buffer, directory, path);
        candidate
            .is_file()
            .then(|| self.context.intern_source_file(candidate))
    }

    /// Resolves an include name through [`Self::header_search_places`], as
    /// GCC and Clang do (C99 §6.10.2p2-3 leave the places
    /// implementation-defined). An absolute name is used as written.
    ///
    /// `including_file` is the file containing the directive, captured before
    /// a macro-expanded operand can switch to its definition's tokenizer.
    /// `including_search_index` belongs to that opening, not its interned
    /// identity. GNU `#include_next` continues after it, or starts at the
    /// first configured entry for local, absolute and main files.
    ///
    /// A header that cannot be found violates the constraint of C99 §6.10.2
    /// paragraph 1, p. 149; PDF p. 161. A file that `#pragma once` marked,
    /// an implementation-defined pragma (§6.10.6 paragraph 1, p. 159; PDF
    /// p. 171), is not read again.
    fn find_header_from_path(
        &mut self,
        including_file: u32,
        including_search_index: Option<usize>,
        operand: SourceVectors,
        path: &Path,
        is_system_header: bool,
        next: bool,
    ) -> Option<(u32, Option<usize>)> {
        let found = if path.is_absolute() {
            path.is_file()
                .then(|| (self.context.intern_source_file(path), None))
        } else {
            // `#include_next` continues after the entry that provided this
            // opening of the including file, or from the first configured
            // entry.
            let start = next.then(|| including_search_index.map_or(0, |index| index + 1));
            let places = self.header_search_places(including_file, is_system_header, start);
            let mut found = None;
            for (search_index, directory) in places.clone() {
                if let Some(index) = self.probe_header(directory, path) {
                    found = Some((index, search_index));
                    break;
                }
            }
            if found.is_none() {
                let searched = self
                    .context
                    .tu_arena()
                    .alloc_slice_fill_iter(places.map(|(_, directory)| directory));
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
        let Some((source_file_index, search_index)) = found else {
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
            Some((source_file_index, search_index))
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

    /// Withdraws the extension diagnostics reported since the first
    /// `reported` while lexing a written `<...>` header name as tokens: its
    /// characters form no tokens (C99 §6.4.7p1, p. 64; PDF p. 76), so `$`
    /// there is no identifier character. Other diagnostics stay.
    #[cold]
    fn withdraw_header_name_extensions(&mut self, reported: usize) {
        let mut kept = ArenaVec::new_in(self.scratch);
        kept.extend(
            self.context
                .split_off_pending_errors(reported)
                .filter(|error| !matches!(error, TranslationError::Extension(_))),
        );
        self.context.append_pending_errors(kept);
    }

    /// Reads a `<…>` operand as written: tokens through the first one that
    /// contains `>`. The name is the source text between the delimiters, so
    /// it keeps the whitespace that the tokens between them do not spell.
    ///
    /// C99: `< h-char-sequence >`, §6.4.7 paragraph 1, p. 64; PDF p. 76, and
    /// §6.10.2 paragraph 2, p. 149; PDF p. 161.
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
            let reported = self.context.pending_error_count();
            let token = self.tokenizer.next_item(self.context);
            // A comment before the new-line ends the operand's text.
            if self.context.pending_error_count() != reported
                && token.is_some_and(|token| token.kind != PreprocessorTokenType::Newline)
            {
                self.withdraw_header_name_extensions(reported);
            }
            let Some(token) = token else {
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
                let characters = logical_characters(
                    self.scratch,
                    source,
                    vector.range(),
                    self.context.configuration.accepts(Feature::Trigraphs),
                );
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
            let open_index = logical_characters(
                self.scratch,
                source,
                open_vector.range(),
                self.context.configuration.accepts(Feature::Trigraphs),
            )
            .first()
            .expect("the operand starts with `<`")
            .index;
            let characters = logical_characters(
                self.scratch,
                source,
                open_index + 1..closing.unwrap_or(end),
                self.context.configuration.accepts(Feature::Trigraphs),
            );
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
    /// text between its quotes is the name. Where a sequence could be either
    /// a header name or a string literal, it is the header name.
    ///
    /// C99: `" q-char-sequence "`, §6.4.7 paragraph 1, p. 64; PDF p. 76;
    /// §6.4 paragraph 4, p. 50; PDF p. 62; and §6.10.2 paragraph 3,
    /// pp. 149-150; PDF pp. 161-162.
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
            let characters = logical_characters(
                self.scratch,
                source,
                vector.range(),
                self.context.configuration.accepts(Feature::Trigraphs),
            );
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
    ///
    /// C99: §6.10.2 paragraph 4, p. 150; PDF p. 162, leaves the combination
    /// implementation-defined. A string literal is taken whole; after `<`,
    /// the spellings of the following tokens, whitespace included, are
    /// joined up to the first `>`. Adjacent string literals are not
    /// concatenated here (footnote 148), so a second one is an extra token.
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
        let end = u32::try_from(self.context.source_vectors.0.len())
            .expect("preprocessor source arena exceeds u32::MAX vectors");
        let source_vectors = SourceVectors::new(start_index, end);
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

    /// Replaces an `#include` directive with the contents of the header or
    /// source file it names, which is then processed through phase 4.
    ///
    /// C99: §6.10.2 paragraphs 1-6, pp. 149-150; PDF pp. 161-162, and
    /// §5.1.1.2 paragraph 1 item 4, p. 10; PDF p. 22. Tokens after the
    /// header name do not match any of the forms of paragraphs 2-4.
    fn parse_include_directive(&mut self, directive: PreprocessorToken) {
        let including_file = self.physical_source_file_index();
        let including_search_index = self
            .tokenizer_stack
            .iter()
            .rev()
            .find_map(|frame| match frame.frame_type {
                | TokenizerFrameType::SourceFile {
                    include_search_index,
                    ..
                } => Some(include_search_index),
                | _ => None,
            })
            .flatten();
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
        // A backslash in a `"…"` name is undefined by C99 §6.4.7p3; reading
        // it as a path character is an extension (§4p6).
        if let Some(source_vectors) = header.backslash {
            let policy = self.context.configuration.extension_policy();
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
                including_search_index,
                header.source_vectors,
                Path::new(header.name),
                header.is_system_header,
                self.context.string_cache.at(directive.contents) == "include_next",
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
        let Some((header_source_index, include_search_index)) = header_source_index else {
            return;
        };
        // The main source contributes one frame. Macro frames and headers
        // whose processing has finished do not consume the nesting limit
        // (C99 §6.10.2p6).
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
                conditional_base: self.state.open_conditionals.len(),
                physical_source_file_index: header_source_index,
                include_search_index,
            },
            tokenizer,
        });
        self.resume_at_line_start();
    }

    /// Defines an object-like or function-like macro.
    ///
    /// C99: the `# define` forms of §6.10 paragraph 1, p. 146; PDF p. 158,
    /// and §6.10.3 paragraphs 1-3 and 5-10, pp. 151-152; PDF pp. 163-164. A
    /// `(` with no whitespace before it (`lparen`) makes the macro
    /// function-like. A redefinition must match the old definition
    /// (paragraph 2), and a predefined name may not be redefined (§6.10.8
    /// paragraph 4, p. 161; PDF p. 173).
    ///
    /// Missing whitespace after an object-like macro's name (§6.10.3
    /// paragraph 3) and `__VA_ARGS__` outside a variadic macro's replacement
    /// list (paragraph 5) draw warnings and keep the definition. A duplicate
    /// parameter name (paragraph 6) or `##` at either end of the replacement
    /// list (§6.10.3.3 paragraph 1, p. 154; PDF p. 166) is an error that
    /// discards the definition, as GCC does, so its uses do not expand into
    /// further errors.
    fn parse_define_directive(&mut self) {
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
        let mut old_tokenizer = match old_definition {
            | None => None,
            | Some(ref v) => match v {
                | MacroDefinition::FunctionLike { tokenizer, .. }
                | MacroDefinition::ObjectLike { tokenizer, .. } => Some(tokenizer.clone()),
                // Not one of the names C99 §6.10.8 predefines, so it may be
                // redefined, as in GCC.
                | MacroDefinition::BuiltIn
                    if implementation_macro(self.context.string_cache.at(name.contents)) =>
                    None,
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
                    if !super::language_features::overridable_gnu_builtin(
                        self.context.string_cache.at(name.contents),
                    ) {
                        self.skip_until_newline();
                        return;
                    }
                    None
                },
            },
        };
        // C99 §6.10.3p5: `__VA_ARGS__` is not a macro name.
        self.check_va_args_use(name);
        // A function-like definition requires '(' immediately after its name.
        let probe = self.tokenizer.next_item(self.context);
        match probe {
            | Some(PreprocessorToken {
                kind: PreprocessorTokenType::OpeningParenthesis,
                ..
            }) => {},
            // C99 §6.10.3p3: whitespace separates an object-like macro's name
            // from its replacement list. Without it the tokens are still the
            // replacement list, as in GCC and Clang.
            | Some(token)
                if !matches!(
                    token.kind,
                    PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
                ) =>
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingWhitespaceAfterMacroName(
                        self.context
                            .diagnostic_text(self.context.string_cache.at(name.contents)),
                    ),
                    source_vectors: token.source_vectors,
                }),
            | Some(_) => {},
            | None => self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                    "parsing macro definition",
                ),
                source_vectors: name.source_vectors,
            }),
        }
        // A definition that breaks a constraint whose meaning C99 leaves open
        // is discarded rather than guessed at.
        let mut is_valid = true;
        // Collected in the expansion arena; the definition keeps a copy.
        let mut argument_names = ArenaVec::new_in(self.scratch);
        let mut is_variadic = false;
        let mut variadic_alias = None;
        let is_function_like =
            probe.is_some_and(|token| token.kind == PreprocessorTokenType::OpeningParenthesis);
        let (body, first) = if is_function_like {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => true,
                // Overridable GNU builtins have no source macro shape.
                | MacroDefinition::FunctionLike { .. } | MacroDefinition::BuiltIn => false,
            }) {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                        ),
                    source_vectors: name.source_vectors,
                });
                // Already diagnosed; comparing the lists would repeat it.
                old_tokenizer = None;
            }
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
                    is_valid = false;
                    break;
                };
                if is_variadic {
                    is_valid = false;
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
                    // C99 §6.10.3p5: `__VA_ARGS__` is not a parameter name.
                    self.check_va_args_use(name_or_ellipsis);
                    let parameter = name_or_ellipsis.identifier_id(self.context);
                    // C99 §6.10.3p6: each parameter is declared once.
                    if argument_names.contains(&parameter) {
                        self.context.preprocessor_error(PreprocessorError {
                            error_type:     PreprocessorErrorType::DuplicateMacroParameter(
                                self.context.diagnostic_text(
                                    self.context.string_cache.at(name_or_ellipsis.contents),
                                ),
                            ),
                            source_vectors: name_or_ellipsis.source_vectors,
                        });
                        is_valid = false;
                    }
                    argument_names.push(parameter);
                } else if name_or_ellipsis.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
                let Some(comma_or_closing_parent) = self.expect_token_from_previous_phase::<true>(
                    |_, t| t.kind == PreprocessorTokenType::Comma || t.kind == PreprocessorTokenType::ClosingParenthesis || (t.kind == PreprocessorTokenType::Ellipsis && !is_variadic),
                    |_, token|
                        ControlFlow::Break(PreprocessorError {
                                error_type:     PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(token.kind),
                                source_vectors: token.source_vectors,
                            },
                        )
                    ,
                    "parsing macro definition",
                ) else {is_valid = false; break;};
                if comma_or_closing_parent.kind == PreprocessorTokenType::Ellipsis {
                    variadic_alias = argument_names.pop();
                    is_variadic = true;
                    self.context.report_extension(
                        Feature::NamedVariadicMacros,
                        "named variadic macro",
                        name_or_ellipsis.source_vectors,
                    );
                    let Some(closing) = self.expect_token_from_previous_phase::<true>(
                        |_, t| t.kind == PreprocessorTokenType::ClosingParenthesis,
                        |this, token| {
                            ControlFlow::Break(PreprocessorError {
                                error_type:
                                    PreprocessorErrorType::VariadicMacroMustBeLastParameter(
                                        this.context.diagnostic_text(
                                            this.context.string_cache.at(name.contents),
                                        ),
                                    ),
                                source_vectors: token.source_vectors,
                            })
                        },
                        "parsing named variadic macro",
                    ) else {
                        is_valid = false;
                        break;
                    };
                    _ = closing;
                    break;
                }
                if comma_or_closing_parent.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
            }
            (self.tokenizer.clone(), None)
        } else {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::FunctionLike { .. } => true,
                // Overridable GNU builtins have no source macro shape.
                | MacroDefinition::ObjectLike { .. } | MacroDefinition::BuiltIn => false,
            }) {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                        ),
                    source_vectors: name.source_vectors,
                });
                // Already diagnosed; comparing the lists would repeat it.
                old_tokenizer = None;
            }
            // The probe already read the replacement list's first token.
            (tokenizer, probe)
        };
        let (list_is_valid, lists_match) = self.read_replacement_list(
            first,
            is_variadic,
            is_variadic && variadic_alias.is_none(),
            old_tokenizer.as_mut(),
        );
        if !(is_valid && list_is_valid) {
            self.resume_at_line_start();
            return;
        }
        // C99 §6.10.3p2: a redefinition repeats the parameters and the
        // replacement list.
        let parameters_match = match &old_definition {
            | Some(MacroDefinition::FunctionLike {
                argument_names: old_argument_names,
                is_variadic: old_is_variadic,
                variadic_alias: old_alias,
                ..
            }) if is_function_like =>
                argument_names[..] == old_argument_names[..]
                    && is_variadic == *old_is_variadic
                    && variadic_alias == *old_alias,
            | _ => true,
        };
        if old_tokenizer.is_some() && !(parameters_match && lists_match) {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                    self.context
                        .diagnostic_text(self.context.string_cache.at(name.contents)),
                ),
                source_vectors: name.source_vectors,
            });
        }
        if is_variadic {
            self.context.report_extension(
                Feature::VariadicMacros,
                "variadic macro",
                name.source_vectors,
            );
        }
        if !self.validate_variadic_body(body.clone(), is_variadic) {
            self.resume_at_line_start();
            return;
        }
        let tokenizer = self.state.lexed_files.persist(&body);
        let definition = if is_function_like {
            MacroDefinition::FunctionLike {
                tokenizer,
                argument_names: self.state.arena.alloc_slice_copy(&argument_names),
                is_variadic,
                variadic_alias,
            }
        } else {
            MacroDefinition::ObjectLike { tokenizer }
        };
        _ = self
            .state
            .macro_definitions
            .insert(name.identifier_id(self.context), definition);
        self.resume_at_line_start();
    }

    /// Reads a `#define` directive's replacement list through its new-line,
    /// starting with `first` when the caller has already read that token.
    /// Returns whether the list is valid where an invalid one discards the
    /// definition, and whether it matches the replacement list that `old`
    /// reads, if any.
    ///
    /// The list is read once, by the directive's own tokenizer, so the
    /// diagnostics recorded while lexing it are reported once.
    ///
    /// C99: `##` shall not begin or end a replacement list, §6.10.3.3
    /// paragraph 1, p. 154; PDF p. 166; `__VA_ARGS__` shall occur only in
    /// that of a variadic macro, §6.10.3 paragraph 5, p. 151; PDF p. 163; and
    /// lists match under paragraph 1, p. 151; PDF p. 163.
    fn read_replacement_list(
        &mut self,
        mut first: Option<PreprocessorToken>,
        is_variadic: bool,
        allows_va_args: bool,
        mut old: Option<&mut TokenSource<'_>>,
    ) -> (bool, bool) {
        let mut lists_match = true;
        let mut first_operand = None;
        let mut last_operand = None;
        let mut operands = 0usize;
        // C99 §6.10.3p1: whitespace separations match whatever their amount,
        // and §6.10.3p7 leaves leading and trailing whitespace out of the
        // list, so the lists are compared token by token, each with whether
        // whitespace separates it from the token before it.
        let mut separated = false;
        loop {
            let next = match first.take() {
                | Some(token) => Some(token),
                | None => self.tokenizer.next_item(self.context),
            };
            let Some(token) = next.filter(|token| token.kind != PreprocessorTokenType::Newline)
            else {
                if lists_match && let Some(old) = old.as_deref_mut() {
                    lists_match = next_replacement_token(self.context, old).is_none();
                }
                break;
            };
            if token.kind == PreprocessorTokenType::Whitespace {
                separated = true;
                continue;
            }
            if lists_match && let Some(old) = old.as_deref_mut() {
                lists_match = next_replacement_token(self.context, old).is_some_and(
                    |(old_token, old_separated)| {
                        (operands == 0 || old_separated == separated)
                            && same_replacement_token(self.context, &old_token, &token)
                    },
                );
            }
            separated = false;
            if !is_variadic
                || (!allows_va_args
                    && token.kind.is_identifier()
                    && self
                        .context
                        .string_cache
                        .at(token.identifier_id(self.context))
                        == "__VA_ARGS__")
            {
                self.check_va_args_use(token);
            }
            _ = first_operand.get_or_insert(token);
            last_operand = Some(token);
            operands += 1;
        }
        let mut is_valid = true;
        if let Some(token) = first_operand.filter(|t| t.kind == PreprocessorTokenType::HashHash) {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator,
                source_vectors: token.source_vectors,
            });
            is_valid = false;
        }
        // A lone `##` is reported once, as the start of the list.
        if let Some(token) = last_operand.filter(|t| t.kind == PreprocessorTokenType::HashHash)
            && operands > 1
        {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MissingRightHandSideOfHashHashOperator,
                source_vectors: token.source_vectors,
            });
            is_valid = false;
        }
        (is_valid, lists_match)
    }

    /// Diagnoses when `token`, read in a `#define` directive, is `__VA_ARGS__`
    /// or `__VA_OPT__`
    /// where it may not appear; the caller allows a variadic macro's
    /// replacement list.
    ///
    /// C99: §6.10.3 paragraph 5, p. 151; PDF p. 163. C23 adds
    /// `__VA_OPT__`, §6.10.5p5, p. 178; PDF p. 191.
    pub(super) fn check_va_args_use(&mut self, token: PreprocessorToken) {
        if token.kind.is_identifier() {
            let policy = self.context.configuration.extension_policy();
            let error_type = match self
                .context
                .string_cache
                .at(token.identifier_id(self.context))
            {
                | "__VA_ARGS__" => PreprocessorErrorType::VaArgsOutsideVariadicMacro(policy),
                | "__VA_OPT__" => PreprocessorErrorType::VaOptOutsideVariadicMacro(policy),
                | _ => return,
            };
            self.context.preprocessor_error(PreprocessorError {
                error_type,
                source_vectors: token.source_vectors,
            });
        }
    }

    /// Ends a macro definition; a name that is not a macro is ignored.
    ///
    /// C99: §6.10.3.5 paragraph 2, p. 155; PDF p. 167. `#undef` of a
    /// predefined name, which §6.10.8 paragraph 4, p. 161; PDF p. 173
    /// forbids, is diagnosed and leaves the name defined, as a redefinition
    /// does.
    fn parse_undef_directive(&mut self) {
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
        let name_id = name.identifier_id(self.context);
        if matches!(
            self.state.macro_definitions.get(&name_id),
            Some(MacroDefinition::BuiltIn)
        ) && !implementation_macro(self.context.string_cache.at(name.contents))
        {
            // C99 §6.10.8p4: predefined macro names cannot be undefined, so
            // the built-in definition stays in effect.
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::UndefinitionOfBuiltInMacro(
                    self.context
                        .diagnostic_text(self.context.string_cache.at(name.contents)),
                ),
                source_vectors: name.source_vectors,
            });
            if super::language_features::overridable_gnu_builtin(
                self.context.string_cache.at(name.contents),
            ) {
                _ = self.state.macro_definitions.remove(&name_id);
            }
        } else {
            _ = self.state.macro_definitions.remove(&name_id);
        }
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
        self.resume_at_line_start();
    }

    /// Sets the presumed line number, and with a string literal the presumed
    /// file name, of the following line. The operands are macro-replaced
    /// first.
    ///
    /// C99: §6.10.4 paragraphs 1 and 3-5, p. 158; PDF p. 170. The line number
    /// must be a digit sequence from 1 to 2147483647; one outside that range
    /// is diagnosed and ignored. The string literal is decoded like any
    /// other; a wide or encoded one is diagnosed and its name ignored.
    /// C11: §6.10.4 paragraph 1, p. 173; PDF p. 191, retains the character
    /// string literal requirement for the newly available encoded literals.
    fn parse_line_directive(&mut self) {
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
        let mut value = digits.bytes().try_fold(0u32, |value, digit| {
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
        } else if value == Some(0) {
            // C99 §6.10.4p3: zero is undefined; like a number that is too
            // large, it is diagnosed and ignored.
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveNumberZero(
                    self.context.diagnostic_text(digits),
                ),
                source_vectors: token.source_vectors,
            });
            value = None;
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
            if let Some(token) = self.map_preprocessor_token(t) {
                match token.kind {
                    | TokenType::String(StringTokenType::String(contents)) => {
                        filename = self
                            .context
                            .literal_text_in(self.scratch, contents, false)
                            .filter(|text| !text.contains('\0'));
                        if filename.is_none() {
                            self.context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::InvalidLineFilename,
                                source_vectors: token.source_vectors,
                            });
                        }
                    },
                    // C99 §6.10.4p1 requires a character string literal. The
                    // name is ignored; the line number still applies.
                    | TokenType::String(StringTokenType::WideString(_)) =>
                        self.context.preprocessor_error(PreprocessorError {
                            error_type:     PreprocessorErrorType::WideStringInLineDirective,
                            source_vectors: token.source_vectors,
                        }),
                    | TokenType::String(StringTokenType::EncodedString(_, encoding)) =>
                        self.context.preprocessor_error(PreprocessorError {
                            error_type:     PreprocessorErrorType::EncodedStringInLineDirective(
                                encoding.prefix(),
                            ),
                            source_vectors: token.source_vectors,
                        }),
                    | _ => {},
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

    /// Reports an error whose message includes the directive's tokens, which
    /// are not macro-replaced; translation then does not succeed.
    ///
    /// C99: §6.10.5 paragraph 1, p. 159; PDF p. 171, and §4 paragraph 4,
    /// p. 7; PDF p. 19.
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
            error_type:     if self.context.string_cache.at(directive.contents) == "warning" {
                PreprocessorErrorType::WarningDirective(self.context.diagnostic_text(&contents))
            } else {
                PreprocessorErrorType::ErrorDirective(self.context.diagnostic_text(&contents))
            },
            source_vectors: directive.source_vectors,
        });
    }

    /// Executes a `#pragma` directive, or the pragma of a `_Pragma` operator,
    /// whose operands are not macro-replaced. Returns whether the directive's
    /// new-line was consumed.
    ///
    /// C99: §6.10.6 paragraphs 1-2, p. 159; PDF p. 171. `STDC` pragmas must
    /// name `FP_CONTRACT`, `FENV_ACCESS`, or `CX_LIMITED_RANGE` and an
    /// `on-off-switch`; they are checked but have no effect yet. `#pragma
    /// once` is bcc's one implementation-defined pragma. Other pragmas are
    /// ignored (paragraph 1), and one that does not begin with an identifier
    /// draws a warning first.
    pub(super) fn parse_pragma_directive(&mut self) -> bool {
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
                                | Some(extra) => {
                                    self.context.preprocessor_error(PreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::ExtraTokensAfterPragmaOnce(
                                                extra.kind,
                                            ),
                                        source_vectors: extra.source_vectors,
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
