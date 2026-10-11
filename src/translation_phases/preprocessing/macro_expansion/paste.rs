use super::{
    ArenaString,
    ArenaVec,
    Expander,
    Feature,
    HashHash,
    MacroArguments,
    PreprocessorError,
    PreprocessorErrorType,
    PreprocessorToken,
    PreprocessorTokenType,
    RangeBounds,
    TokenSource,
    TokenizerFrame,
    TokenizerFrameType,
    find_argument,
};

impl<'x> Expander<'_, '_, '_, 'x> {
    /// Reads the next token, applying any `##` that follows it. The flag
    /// tells whether `##` formed the token, which then names no parameter.
    ///
    /// C99: §6.10.3.3 paragraph 3, p. 154; PDF p. 166: `##` is an operator in
    /// the replacement list of either form of macro, but not one that comes
    /// from an argument.
    pub(in crate::translation_phases::preprocessing) fn handle_hash_hash_operator<
        const SHOULD_IGNORE_WHITESPACE: bool,
    >(
        &mut self,
    ) -> Option<(PreprocessorToken, bool)> {
        'base: loop {
            let lhs = self.handle_hash_operator::<SHOULD_IGNORE_WHITESPACE>()?;
            // An empty argument's placemarker arrives as its frame is popped.
            // A fenced read of that argument ends there, so it reads no `##`
            // from the replacement list below the fence.
            let fenced = self.tokenizer_stack.len() < self.operand_fence.max(self.expansion_fence);
            let replacement_list = !fenced
                && match self.tokenizer_stack.last().map(|frame| &frame.frame_type) {
                    | Some(
                        TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                        | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                    ) => true,
                    | Some(TokenizerFrameType::FunctionLikeMacroArgument { argument, .. }) =>
                        argument.enclosing_arguments.is_some(),
                    | _ => false,
                };
            let hash_hash = if replacement_list && self.hash_hash_follows() {
                Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
            } else {
                None
            };
            if let Some(h) = hash_hash {
                let Some(rhs) = self.hash_hash_right_operand() else {
                    return Some((lhs, false));
                };
                if self.hash_hash_follows() {
                    self.paste_hash_hash_chain(lhs, rhs);
                    continue 'base;
                }
                if let Some(r) = self.parse_hash_hash_operator(lhs, h, rhs) {
                    return Some((r, true));
                }
                continue 'base;
            }
            return Some((lhs, false));
        }
    }

    /// Pastes `lhs ## rhs`. A parameter operand stands for its argument as
    /// written, the last token of a left argument and the first of a right
    /// one taking part; an empty argument is a placemarker.
    ///
    /// C99: §6.10.3.3 paragraphs 2-3, p. 154; PDF p. 166.
    fn parse_hash_hash_operator(
        &mut self,
        lhs: PreprocessorToken,
        _hash_hash: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        let arguments = self.get_arguments();
        if let Some(argument) = arguments
            .and_then(|arguments| find_argument(arguments, lhs.identifier_id(self.context)))
        {
            let mut left = self.read_argument(argument, false);
            if left.len() > 1 {
                // Rescanning an argument prefix here can enter another macro
                // frame and prematurely apply the pending paste to that macro's
                // first token. Paste the complete raw operand first instead.
                let right = self.paste_operand(arguments, rhs);
                self.paste_onto(&mut left, right);
                self.replay_pasted_tokens(&left);
                return None;
            }
        }
        let lhs_frame = self.operand_frame(lhs);
        let rhs_frame = self.operand_frame(rhs);
        self.hash_hash_stack.push(HashHash::Empty);
        let rhs_is_macro_argument = if let Some(frame) = rhs_frame {
            self.push_tokenizer_frame(frame);
            true
        } else {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Rhs(rhs);
            false
        };
        if let Some(frame) = lhs_frame {
            self.push_tokenizer_frame(frame);
        } else if rhs_is_macro_argument {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Lhs(lhs);
        } else {
            _ = self.hash_hash_stack.pop();
            return self.merge_tokens(lhs, rhs);
        }
        None
    }

    /// Concatenates two preprocessing tokens for `##`, or `None` when both
    /// are placemarkers. A result that is not one valid preprocessing token
    /// is undefined behavior; it is diagnosed, keeping the left operand.
    /// Forming a universal character name by concatenation is undefined too;
    /// an identifier that pasting leaves with a `\` is read as a universal
    /// identifier.
    ///
    /// C99: §6.10.3.3 paragraph 3, p. 154; PDF p. 166, and §5.1.1.2
    /// paragraph 1 item 4, p. 10; PDF p. 22.
    pub(in crate::translation_phases::preprocessing) fn merge_tokens(
        &mut self,
        mut lhs: PreprocessorToken,
        mut rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        for token in [&mut lhs, &mut rhs] {
            if token.kind.is_identifier() {
                token.kind = PreprocessorTokenType::Identifier;
            }
        }
        let mut token = self.merge_tokens_impl(lhs, rhs)?;
        if !self.context.configuration.is_native(Feature::Digraphs)
            && let spelling @ ("<:" | ":>" | "<%" | "%>" | "%:" | "%:%:") =
                self.context.string_cache.at(token.contents)
        {
            let spelling = match spelling {
                | "<:" => "<:",
                | ":>" => ":>",
                | "<%" => "<%",
                | "%>" => "%>",
                | "%:" => "%:",
                | _ => "%:%:",
            };
            self.context
                .report_extension(Feature::Digraphs, spelling, token.source_vectors);
        }
        if token.kind == PreprocessorTokenType::Identifier
            && self.context.string_cache.at(token.contents).contains('\\')
        {
            (token.kind, token.contents) =
                crate::translation_phases::preprocessor_tokenizer::ucn::identifier(
                    self.context,
                    self.scratch,
                    token.contents,
                );
        }
        Some(token)
    }

    fn merge_tokens_impl(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        if !self.context.configuration.accepts(Feature::Digraphs)
            && matches!(
                (lhs.kind, rhs.kind),
                (
                    PreprocessorTokenType::LessThan,
                    PreprocessorTokenType::Colon | PreprocessorTokenType::Percent
                ) | (
                    PreprocessorTokenType::Colon | PreprocessorTokenType::Percent,
                    PreprocessorTokenType::GreaterThan
                ) | (PreprocessorTokenType::Percent, PreprocessorTokenType::Colon)
            )
        {
            return Some(self.create_merge_error(lhs, rhs));
        }
        match (lhs.kind, rhs.kind) {
            // C99 §6.10.3.3p3: two placemarkers give one placemarker, and a
            // placemarker with another token gives that token.
            | (PreprocessorTokenType::Placeholder, PreprocessorTokenType::Placeholder) => None,
            | (PreprocessorTokenType::Placeholder, _) => Some(rhs),
            | (_, PreprocessorTokenType::Placeholder) => Some(lhs),
            | (
                PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined,
                PreprocessorTokenType::Identifier
                | PreprocessorTokenType::Defined
                | PreprocessorTokenType::Number,
            ) => {
                let rhs_contents = self.context.string_cache.at(rhs.contents);
                // A pp-number with `.` or an exponent sign continues no
                // identifier.
                if rhs_contents.contains(['.', '+', '-', '\'']) {
                    return Some(self.create_merge_error(lhs, rhs));
                }
                let rhs_range = if rhs.kind == PreprocessorTokenType::Number {
                    0..rhs_contents.len() - 1
                } else {
                    0..rhs_contents.len()
                };
                let new = self.merge_token_contents_with_ranges(
                    lhs,
                    rhs,
                    ..,
                    rhs_range,
                    PreprocessorTokenType::Identifier,
                );
                let kind = if self.context.string_cache.at(new.contents) == "defined" {
                    PreprocessorTokenType::Defined
                } else {
                    PreprocessorTokenType::Identifier
                };
                Some(PreprocessorToken {
                    kind,
                    contents: new.contents,
                    source_vectors: new.source_vectors,
                })
            },
            | (
                PreprocessorTokenType::Identifier,
                PreprocessorTokenType::String
                | PreprocessorTokenType::Character
                | PreprocessorTokenType::GeneratedString,
            ) => {
                // C99 §6.10.3.3p3: `L ## "x"` forms the wide literal `L"x"`,
                // so the right-hand side must still be a narrow literal.
                let prefix = self.context.string_cache.at(lhs.contents);
                let enabled = prefix == "L"
                    || (matches!(prefix, "u" | "U" | "u8")
                        && self
                            .context
                            .configuration
                            .accepts(Feature::UnicodeLiteralPrefixes)
                        && (prefix != "u8"
                            || rhs.kind != PreprocessorTokenType::Character
                            || self
                                .context
                                .configuration
                                .accepts(Feature::Utf8CharacterConstants)));
                if enabled
                    && (rhs.kind == PreprocessorTokenType::GeneratedString
                        || self
                            .context
                            .string_cache
                            .at(rhs.contents)
                            .starts_with(['"', '\'']))
                {
                    Some(self.merge_token_contents(
                        lhs,
                        rhs,
                        match rhs.kind {
                            | PreprocessorTokenType::GeneratedString =>
                                PreprocessorTokenType::WideGeneratedString,
                            | _ => rhs.kind,
                        },
                    ))
                } else {
                    Some(self.create_merge_error(lhs, rhs))
                }
            },
            | (
                PreprocessorTokenType::Period | PreprocessorTokenType::Number,
                PreprocessorTokenType::Number,
            ) => {
                let lhs_contents = self.context.string_cache.at(lhs.contents);
                let lhs_range = if lhs.kind == PreprocessorTokenType::Number {
                    0..lhs_contents.len() - 1
                } else {
                    0..lhs_contents.len()
                };
                // We don't remove the trailing null byte from the right-hand
                // side because we're generating a new number
                // token, and number tokens should always have a
                // trailing null byte.
                Some(self.merge_token_contents_with_ranges(
                    lhs,
                    rhs,
                    lhs_range,
                    ..,
                    PreprocessorTokenType::Number,
                ))
            },
            // C99 §6.4.8: a pp-number continues with identifier characters,
            // periods, and a sign after `e`, `E`, `p`, or `P`.
            | (
                PreprocessorTokenType::Number,
                PreprocessorTokenType::Identifier
                | PreprocessorTokenType::Defined
                | PreprocessorTokenType::Period,
            ) => Some(self.extend_number(lhs, rhs)),
            | (
                PreprocessorTokenType::Number,
                PreprocessorTokenType::Plus | PreprocessorTokenType::Minus,
            ) if self
                .context
                .string_cache
                .at(lhs.contents)
                .trim_end_matches('\0')
                .ends_with(['e', 'E', 'p', 'P']) =>
                Some(self.extend_number(lhs, rhs)),
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Plus) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PlusPlus)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Minus) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::MinusMinus)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::Arrow)),
            // Digraphs retain their source spelling when pasted (C99 6.4.6).
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Colon) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::OpeningSquareBracket),
            ),
            | (PreprocessorTokenType::Colon, PreprocessorTokenType::GreaterThan) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::ClosingSquareBracket),
            ),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Percent) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::OpeningCurlyBrace)),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::ClosingCurlyBrace)),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::Colon) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::Hash)),
            | (PreprocessorTokenType::Hash, PreprocessorTokenType::Hash) => {
                // Mixing the spellings, such as `#` and `%:`, produces two
                // preprocessing tokens rather than one valid paste.
                let lhs_contents = self.context.string_cache.at(lhs.contents);
                let rhs_contents = self.context.string_cache.at(rhs.contents);
                if (lhs_contents == "#" && rhs_contents == "#")
                    || (lhs_contents == "%:" && rhs_contents == "%:")
                {
                    Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::HashHash))
                } else {
                    Some(self.create_merge_error(lhs, rhs))
                }
            },
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PlusEquals)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::MinusEquals)),
            | (PreprocessorTokenType::Asterisk, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::AsteriskEquals)),
            | (PreprocessorTokenType::ForwardSlash, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::ForwardSlashEquals)),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PercentEquals)),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::LessThan) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::LessThanLessThan)),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::GreaterThan) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::GreaterThanGreaterThan),
            ),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::LessThanEquals)),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::GreaterThanEquals)),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::LessThanEquals)
            | (PreprocessorTokenType::LessThanLessThan, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::LessThanLessThanEquals),
            ),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::GreaterThanEquals)
            | (PreprocessorTokenType::GreaterThanGreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanGreaterThanEquals,
                )),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::AmpersandEquals)),
            | (PreprocessorTokenType::Caret, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::CaretEquals)),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PipeEquals)),
            | (PreprocessorTokenType::ExclamationMark, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::ExclamationMarkEquals),
            ),
            | (PreprocessorTokenType::Equals, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::EqualsEquals)),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Pipe) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PipePipe)),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Ampersand) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::AmpersandAmpersand)),

            | _ => Some(self.create_merge_error(lhs, rhs)),
        }
    }

    fn merge_token_contents(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        self.merge_token_contents_with_ranges(lhs, rhs, .., .., result_token_type)
    }

    fn merge_token_contents_with_ranges(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        lhs_range: impl RangeBounds<usize>,
        rhs_range: impl RangeBounds<usize>,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        let mut new_contents = ArenaString::new_in(self.scratch);
        new_contents.push_str(
            &self.context.string_cache.at(lhs.contents)[(
                lhs_range.start_bound().cloned(),
                lhs_range.end_bound().cloned(),
            )],
        );
        new_contents.push_str(
            &self.context.string_cache.at(rhs.contents)[(
                rhs_range.start_bound().cloned(),
                rhs_range.end_bound().cloned(),
            )],
        );
        let source_vectors = self
            .context
            .merge_vectors(lhs.source_vectors, rhs.source_vectors);
        PreprocessorToken {
            kind: result_token_type,
            contents: self.context.string_cache.intern(new_contents.as_str()),
            source_vectors,
        }
    }

    #[cold]
    #[inline(never)]
    fn create_merge_error(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> PreprocessorToken {
        // Number spellings carry a trailing NUL that is not source text.
        let spell = |token: PreprocessorToken| {
            let contents = self.context.string_cache.at(token.contents);
            self.context.diagnostic_text(match token.kind {
                | PreprocessorTokenType::Number => contents.strip_suffix('\0').unwrap_or(contents),
                | _ => contents,
            })
        };
        let lhs_contents = spell(lhs);
        let rhs_contents = spell(rhs);
        let source_vectors = self
            .context
            .merge_vectors(lhs.source_vectors, rhs.source_vectors);
        self.context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::TokenMergingError(lhs_contents, rhs_contents),
            source_vectors,
        });
        lhs
    }

    /// Appends `rhs` to the pp-number `lhs`, keeping the trailing NUL that
    /// number spellings carry.
    fn extend_number(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> PreprocessorToken {
        let lhs_contents = self.context.string_cache.at(lhs.contents);
        let mut contents = ArenaString::new_in(self.scratch);
        contents.push_str(lhs_contents.strip_suffix('\0').unwrap_or(lhs_contents));
        contents.push_str(self.context.string_cache.at(rhs.contents));
        contents.push_str("\0");
        let source_vectors = self
            .context
            .merge_vectors(lhs.source_vectors, rhs.source_vectors);
        PreprocessorToken {
            kind: PreprocessorTokenType::Number,
            contents: self.context.string_cache.intern(contents.as_str()),
            source_vectors,
        }
    }

    /// Whether the replacement list being read continues with `##`.
    fn hash_hash_follows(&mut self) -> bool {
        let save = self.position();
        self.context.set_ignore_tokenizer_errors(true);
        let hash_hash = Self::next_ignore_whitespace(&mut self.tokenizer, self.context);
        self.context.set_ignore_tokenizer_errors(false);
        self.set_position(save);
        hash_hash.is_some_and(|v| v.kind == PreprocessorTokenType::HashHash)
    }

    /// Reads the right operand of the `##` just read, applying `#` to it.
    fn hash_hash_right_operand(&mut self) -> Option<PreprocessorToken> {
        let operand = self.handle_hash_operator::<true>();
        if operand.is_none() {
            let source_vectors = self.current_location();
            self.context.preprocessor_error(PreprocessorError {
                error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                    "parsing hash-hash operator. Hash hash operator must be followed by a \
                     preprocessor token on the same line.",
                ),
                source_vectors,
            });
        }
        operand
    }

    /// Applies a chain of two or more `##` operators, `first ## second ##
    /// ...`, whose second `##` is next, and replays the result to be
    /// rescanned with the rest of the replacement list.
    ///
    /// The operators paste left to right, each pasting the result so far with
    /// its right operand. The whole chain is pasted here, because the paste
    /// of a single `##` waits for its argument frames to be read, and so
    /// cannot be the operand of another `##`.
    ///
    /// C99: §6.10.3.3 paragraphs 2-3, p. 154; PDF p. 166: the order of
    /// evaluation of `##` operators is unspecified; §6.10.3.4 paragraph 1,
    /// p. 155; PDF p. 167.
    fn paste_hash_hash_chain(&mut self, first: PreprocessorToken, second: PreprocessorToken) {
        let arguments = self.get_arguments();
        let mut result = self.paste_operand(arguments, first);
        let mut operand = Some(second);
        while let Some(token) = operand {
            let right = self.paste_operand(arguments, token);
            self.paste_onto(&mut result, right);
            operand = if self.hash_hash_follows() {
                _ = Self::next_ignore_whitespace(&mut self.tokenizer, self.context);
                self.hash_hash_right_operand()
            } else {
                None
            };
        }
        self.replay_pasted_tokens(&result);
    }

    fn replay_pasted_tokens(&mut self, result: &[PreprocessorToken]) {
        let Some(location) = result.first().map(|token| {
            self.context
                .first_source_vector_or_default(token.source_vectors)
        }) else {
            // Every operand was a placemarker.
            return;
        };
        let tokenizer = TokenSource::replay(self.context, self.scratch, &[result], location);
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::Rescan { argument: false },
            tokenizer,
        });
    }

    /// The tokens that the `##` operand `token` stands for: the argument as
    /// written when it names a parameter, which is empty for a placemarker,
    /// and otherwise the token itself.
    ///
    /// C99: §6.10.3.3 paragraph 2, p. 154; PDF p. 166.
    pub(in crate::translation_phases::preprocessing) fn paste_operand(
        &mut self,
        arguments: Option<MacroArguments<'x>>,
        token: PreprocessorToken,
    ) -> ArenaVec<'x, PreprocessorToken> {
        if let Some(argument) = arguments
            .and_then(|arguments| find_argument(arguments, token.identifier_id(self.context)))
        {
            return self.read_argument(argument, false);
        }
        let mut tokens = ArenaVec::new_in(self.scratch);
        tokens.push(token);
        tokens
    }

    /// Pastes the last token of `left` with the first of `right`, then
    /// appends the rest of `right`. An empty side is a placemarker, which
    /// leaves the other unchanged.
    ///
    /// C99: §6.10.3.3 paragraphs 2-3, p. 154; PDF p. 166.
    pub(in crate::translation_phases::preprocessing) fn paste_onto(
        &mut self,
        left: &mut ArenaVec<'x, PreprocessorToken>,
        mut right: ArenaVec<'x, PreprocessorToken>,
    ) {
        if !right.is_empty()
            && let Some(lhs) = left.pop()
        {
            let rhs = right.remove(0);
            if let Some(merged) = self.merge_tokens(lhs, rhs) {
                left.push(merged);
            }
        }
        left.extend(right);
    }
}
