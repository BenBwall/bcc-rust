//! Read header operands, search for headers, and push included source files.
//!
//! C99: translation phase 4; includes, §6.10.2, pp. 149-151; PDF pp. 161-163.
//! The nesting limit is implementation-defined (§6.10.2 paragraph 6, p. 150;
//! PDF p. 162; minimum §5.2.4.1 paragraph 1, p. 21; PDF p. 33).
//! Opened files return to the shared preprocessing reader for macro
//! replacement.

use std::{
    ffi::OsStr,
    ops::ControlFlow,
    path::{
        Component,
        Path,
    },
};

use crate::{
    configuration::{
        ExtensionPolicy,
        Feature,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        SourcePosition,
        SourceVectors,
        TranslationError,
        TranslationPhase,
        preprocessing::{
            Expander,
            errors::{
                PreprocessorError,
                PreprocessorErrorType,
            },
            runtime::{
                TokenizerFrame,
                TokenizerFrameType,
            },
        },
        preprocessor_tokenizer::{
            LogicalCharacter,
            PreprocessorToken,
            PreprocessorTokenType,
            logical_characters,
            position_after,
        },
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

impl<'tu, 'x> Expander<'_, 'tu, '_, 'x> {
    /// Replaces an `#include` directive with the contents of the header or
    /// source file it names, which is then processed through phase 4.
    ///
    /// C99: §6.10.2 paragraphs 1-6, pp. 149-150; PDF pp. 161-162, and
    /// §5.1.1.2 paragraph 1 item 4, p. 10; PDF p. 22. Tokens after the
    /// header name do not match any of the forms of paragraphs 2-4.
    pub(in crate::translation_phases::preprocessing) fn parse_include_directive(
        &mut self,
        directive: PreprocessorToken,
    ) {
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
        // A backslash in a `"…"` name is undefined by C99 §6.4.7p3; reading
        // it as a path character is an extension (§4p6).
        if let Some(source_vectors) = header.backslash {
            let policy = self.context.configuration.extension_policy();
            self.context.preprocessor_extension(
                Feature::QuotedHeaderBackslash,
                crate::translation_phases::DiagnosticPolicy::Extension,
                source_vectors,
                PreprocessorErrorType::BackslashInQuotedHeaderName,
            );
            look_up = policy != ExtensionPolicy::Deny
                || self
                    .context
                    .withholds(ErrorSeverity::Error, true, source_vectors);
        }
        let start = if self.context.string_cache.at(directive.contents) == "include_next" {
            self.include_next_start("#include_next", directive.source_vectors)
        } else {
            None
        };
        let header_source_index = if look_up {
            self.find_header_from_path(
                including_file,
                header.source_vectors,
                Path::new(header.name),
                header.is_system_header,
                start,
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
            .filter(|frame| {
                matches!(frame.frame_type, TokenizerFrameType::SourceFile {
                physical_source_file_index, ..
            } if Some(physical_source_file_index) != self.state.command_line_file)
            })
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
    pub(in crate::translation_phases::preprocessing) fn header_search_places(
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

    /// The configured search entry of the innermost open source file, which
    /// `#include_next` continues after. Each opening carries its own entry,
    /// so one file reached through different entries continues from each.
    /// As in Clang, a header found beside its includer takes the includer's
    /// entry; the primary source file, a header found by an absolute path,
    /// and one found beside either have none.
    fn including_search_index(&self) -> Option<usize> {
        self.tokenizer_stack
            .iter()
            .rev()
            .find_map(|frame| match frame.frame_type {
                | TokenizerFrameType::SourceFile {
                    include_search_index,
                    ..
                } => Some(include_search_index),
                | _ => None,
            })
            .flatten()
    }

    /// Where an `#include_next` or `__has_include_next` (`spelling`) lookup
    /// starts, following Clang: after the configured entry of this opening
    /// of the current file. With no entry to continue after, in the primary
    /// source file or in a header with none, it warns and searches exactly
    /// as `#include` would (`None`).
    ///
    /// C99: an extension (§4p6, p. 7; PDF p. 19) over the
    /// implementation-defined places of §6.10.2 paragraphs 2-3, pp. 149-150;
    /// PDF pp. 161-162.
    pub(in crate::translation_phases::preprocessing) fn include_next_start(
        &mut self,
        spelling: &'static str,
        source_vectors: SourceVectors,
    ) -> Option<usize> {
        let error_type = if !self.current_is_header() {
            PreprocessorErrorType::IncludeNextInPrimarySource(spelling)
        } else if let Some(index) = self.including_search_index() {
            return Some(index + 1);
        } else {
            PreprocessorErrorType::IncludeNextWithoutSearchEntry(spelling)
        };
        self.context.preprocessor_error(PreprocessorError {
            error_type,
            source_vectors,
        });
        None
    }

    /// The file `path` names in `directory`, interned, when it exists there.
    /// The resource directory holds exactly the embedded headers.
    pub(in crate::translation_phases::preprocessing) fn probe_header(
        &mut self,
        directory: &Path,
        path: &Path,
    ) -> Option<u32> {
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
    /// `#include_next` passes where its lookup starts, from
    /// [`Self::include_next_start`].
    ///
    /// `including_file` is the file containing the directive, captured before
    /// a macro-expanded operand can switch to its definition's tokenizer. The
    /// result pairs the header with the configured entry its own
    /// `#include_next` continues after: the entry that provided it, or, for
    /// a header found beside its includer, the includer's, as in Clang.
    ///
    /// A header that cannot be found violates the constraint of C99 §6.10.2
    /// paragraph 1, p. 149; PDF p. 161. A file that `#pragma once` marked,
    /// an implementation-defined pragma (§6.10.6 paragraph 1, p. 159; PDF
    /// p. 171), is not read again.
    fn find_header_from_path(
        &mut self,
        including_file: u32,
        operand: SourceVectors,
        path: &Path,
        is_system_header: bool,
        start: Option<usize>,
    ) -> Option<(u32, Option<usize>)> {
        let found = if path.is_absolute() {
            path.is_file()
                .then(|| (self.context.intern_source_file(path), None))
        } else {
            let places = self.header_search_places(including_file, is_system_header, start);
            let mut found = None;
            for (search_index, directory) in places.clone() {
                if let Some(index) = self.probe_header(directory, path) {
                    // As in GCC, a header found through a system directory
                    // is a system header, and so is one found beside a
                    // system header.
                    let system = if let Some(search_index) = search_index {
                        self.context.is_system_include_directory(search_index)
                    } else {
                        self.context.has_system_header_part(including_file)
                    };
                    if system {
                        self.context.mark_system_header(index, 0);
                    }
                    let origin = search_index.or_else(|| self.including_search_index());
                    found = Some((index, origin));
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
        let allow_backslash = self.context.configuration.extension_policy()
            != ExtensionPolicy::Deny
            || self
                .context
                .withholds(ErrorSeverity::Error, true, token.source_vectors);
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

/// A header name read from source text, with the locations of its
/// questionable characters as positions and source byte lengths.
struct WrittenHeaderName<'a> {
    name:      &'a str,
    /// The first sequence C99 §6.4.7p3 does not allow.
    invalid:   Option<(&'static str, SourcePosition, usize)>,
    /// The first backslash of a `"…"` name.
    backslash: Option<(SourcePosition, usize)>,
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
