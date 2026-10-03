mod batch;
mod replay;
#[cfg(test)]
mod tests;
mod token_source;
pub(crate) mod ucn;

use std::fmt::Display;

pub(crate) use token_source::{
    LexingStrategy,
    TokenSource,
};

use super::{
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceFileIndex,
    GetSourceVectors,
    SetPosition,
    SetSourceFileIndex,
    SourcePosition,
    SourceVector,
    SourceVectors,
    TranslationPhase,
    initial_processing::InitialProcessor,
};
use crate::{
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        quote_spelling,
    },
    util::{
        byte_scan,
        shared::SharedString,
        string_cache::StringCacheId,
    },
};

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone)]
pub(crate) struct PreprocessorTokenizer {
    initial_processor:   InitialProcessor,
    current_token_start: SourcePosition,
}

impl GetPosition for PreprocessorTokenizer {
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        self.initial_processor.position(context)
    }
}

impl SetPosition for PreprocessorTokenizer {
    #[inline(always)]
    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        self.initial_processor.set_position(context, position);
    }
}

impl GetSourceFileIndex for PreprocessorTokenizer {
    #[inline(always)]
    fn source_file_index(&self) -> u32 {
        self.initial_processor.source_file_index()
    }
}

impl SetSourceFileIndex for PreprocessorTokenizer {
    fn set_source_file_index(&mut self, context: &mut Context, source_file_index: u32) {
        self.initial_processor
            .set_source_file_index(context, source_file_index);
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum PreprocessorTokenizerErrorType {
    UnknownToken,
    UnterminatedBlockComment,
    UnterminatedCharacter,
    UnterminatedString,
    UnterminatedIncludeString,
    NewlineInCharacter,
    NewlineInString,
    NewlineInIncludeString,
}

impl PreprocessorTokenizerErrorType {
    /// Describes the error; `spelling` is the source text it points at.
    pub(crate) fn explain(self, spelling: Option<&str>) -> Explanation {
        match self {
            | Self::UnknownToken => {
                let character = spelling.and_then(|spelling| spelling.chars().next());
                let quoted = match character {
                    | Some('`') => "'`'".to_owned(),
                    | Some(c) if c.is_control() => format!("U+{:04X}", u32::from(c)),
                    | Some(c) => format!("`{c}`"),
                    | None => "character".to_owned(),
                };
                Explanation::new(format!("unexpected character {quoted} in source"))
                    .label("no C token starts with this character")
                    .note(
                        "C99 §6.4: a preprocessing token that survives replacement must be \
                         convertible to a C token",
                    )
            },
            | Self::UnterminatedBlockComment => Explanation::new("unterminated block comment")
                .label("the file ends before the closing `*/`")
                .note("C99 §5.1.1.2p3: a source file shall not end in a partial comment")
                .help("close the comment with `*/`"),
            | Self::UnterminatedCharacter => Explanation::new("unterminated character constant")
                .label("the file ends before the closing `'`"),
            | Self::UnterminatedString => Explanation::new("unterminated string literal")
                .label("the file ends before the closing `\"`"),
            | Self::UnterminatedIncludeString => Explanation::new("unterminated header name")
                .label("the file ends before the closing delimiter"),
            | Self::NewlineInCharacter => Explanation::new("unterminated character constant")
                .label("the line ends before the closing `'`")
                .note("C99 §6.4.4.4: a character constant cannot span lines")
                .help("write `\\n` for a newline character"),
            | Self::NewlineInString => Explanation::new("unterminated string literal")
                .label("the line ends before the closing `\"`")
                .note("C99 §6.4.5: a string literal cannot span lines")
                .help(
                    "close the literal on this line and start another on the next; adjacent \
                     literals are concatenated",
                ),
            | Self::NewlineInIncludeString => Explanation::new("unterminated header name")
                .label("the line ends before the closing delimiter")
                .note("C99 §6.4.7: a header name cannot span lines"),
        }
    }
}

impl Display for PreprocessorTokenizerErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.explain(None).message)
    }
}

impl PreprocessorTokenType {
    /// Names the token kind for use in a message, such as "identifier" or
    /// "`(`".
    pub(crate) fn description(self) -> &'static str {
        match self {
            | Self::Identifier
            | Self::UnavailableIdentifier
            | Self::UnavailableUniversalIdentifier
            | Self::UniversalIdentifier => "identifier",
            | Self::Number => "number",
            | Self::String | Self::GeneratedString => "string literal",
            | Self::WideGeneratedString => "wide string literal",
            | Self::Character => "character constant",
            | Self::Other => "character",
            | Self::Placeholder => "empty macro argument",
            | Self::AngleBracketString | Self::IncludeString => "header name",
            | Self::Newline => "end of line",
            | Self::Whitespace => "whitespace",
            | Self::Defined => "`defined`",
            | Self::OpeningSquareBracket => "`[`",
            | Self::ClosingSquareBracket => "`]`",
            | Self::OpeningParenthesis => "`(`",
            | Self::ClosingParenthesis => "`)`",
            | Self::OpeningCurlyBrace => "`{`",
            | Self::ClosingCurlyBrace => "`}`",
            | Self::Period => "`.`",
            | Self::Arrow => "`->`",
            | Self::PlusPlus => "`++`",
            | Self::MinusMinus => "`--`",
            | Self::Ampersand => "`&`",
            | Self::Asterisk => "`*`",
            | Self::Plus => "`+`",
            | Self::Minus => "`-`",
            | Self::Tilde => "`~`",
            | Self::ExclamationMark => "`!`",
            | Self::ForwardSlash => "`/`",
            | Self::Percent => "`%`",
            | Self::LessThanLessThan => "`<<`",
            | Self::GreaterThanGreaterThan => "`>>`",
            | Self::LessThan => "`<`",
            | Self::GreaterThan => "`>`",
            | Self::LessThanEquals => "`<=`",
            | Self::GreaterThanEquals => "`>=`",
            | Self::EqualsEquals => "`==`",
            | Self::ExclamationMarkEquals => "`!=`",
            | Self::Caret => "`^`",
            | Self::Pipe => "`|`",
            | Self::AmpersandAmpersand => "`&&`",
            | Self::PipePipe => "`||`",
            | Self::QuestionMark => "`?`",
            | Self::Colon => "`:`",
            | Self::SemiColon => "`;`",
            | Self::Ellipsis => "`...`",
            | Self::Equals => "`=`",
            | Self::AsteriskEquals => "`*=`",
            | Self::ForwardSlashEquals => "`/=`",
            | Self::PercentEquals => "`%=`",
            | Self::PlusEquals => "`+=`",
            | Self::MinusEquals => "`-=`",
            | Self::LessThanLessThanEquals => "`<<=`",
            | Self::GreaterThanGreaterThanEquals => "`>>=`",
            | Self::AmpersandEquals => "`&=`",
            | Self::CaretEquals => "`^=`",
            | Self::PipeEquals => "`|=`",
            | Self::Comma => "`,`",
            | Self::Hash => "`#`",
            | Self::HashHash => "`##`",
        }
    }

    pub(crate) fn is_identifier(self) -> bool {
        matches!(
            self,
            Self::Identifier
                | Self::UniversalIdentifier
                | Self::UnavailableIdentifier
                | Self::UnavailableUniversalIdentifier
        )
    }

    /// Describes a found token, quoting its spelling when the kind alone
    /// does not say what was written.
    pub(crate) fn found(self, spelling: Option<&str>) -> String {
        let spelled = matches!(
            self,
            Self::Identifier
                | Self::Number
                | Self::String
                | Self::GeneratedString
                | Self::WideGeneratedString
                | Self::Character
                | Self::Other
                | Self::AngleBracketString
                | Self::IncludeString
        );
        match spelling {
            | Some(spelling) if spelled && !spelling.is_empty() =>
                format!("{} {}", self.description(), quote_spelling(spelling)),
            | _ => self.description().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreprocessorTokenizerError {
    source_vector: SourceVector,
    error_type:    PreprocessorTokenizerErrorType,
    /// For an unknown token, the character after phases 1 and 2 that no
    /// token starts with; its raw spelling may be a trigraph.
    character:     Option<char>,
}

impl PreprocessorTokenizerError {
    /// Reports a phase-3 character that survived preprocessing but cannot
    /// become a C token in phase 7.
    pub(crate) fn unknown_character(source_vector: SourceVector, character: char) -> Self {
        Self {
            source_vector,
            error_type: PreprocessorTokenizerErrorType::UnknownToken,
            character: Some(character),
        }
    }
}

impl std::error::Error for PreprocessorTokenizerError {}

impl GetPosition for PreprocessorTokenizerError {
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        self.source_vector.position(context)
    }
}

impl GetSourceVectors for PreprocessorTokenizerError {
    #[inline(always)]
    fn source_vectors(&self, context: &mut Context) -> SourceVectors {
        context.create_source_vectors(
            self.source_vector.position(context),
            self.source_vector.source_file_index,
            self.source_vector.length as usize,
        )
    }
}

impl GetSeverity for PreprocessorTokenizerError {
    fn severity(&self) -> ErrorSeverity {
        match self.error_type {
            | PreprocessorTokenizerErrorType::UnknownToken
            | PreprocessorTokenizerErrorType::UnterminatedBlockComment
            | PreprocessorTokenizerErrorType::UnterminatedCharacter
            | PreprocessorTokenizerErrorType::UnterminatedString
            | PreprocessorTokenizerErrorType::UnterminatedIncludeString
            | PreprocessorTokenizerErrorType::NewlineInCharacter
            | PreprocessorTokenizerErrorType::NewlineInString
            | PreprocessorTokenizerErrorType::NewlineInIncludeString => ErrorSeverity::Error,
        }
    }
}

impl Display for PreprocessorTokenizerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error_type)
    }
}

impl ToDiagnostic for PreprocessorTokenizerError {
    fn to_diagnostic(&self, context: &Context, source: SourceVectors) -> Diagnostic {
        let mut buffer = [0; 4];
        let spelling = match self.character {
            | Some(character) => Some(&*character.encode_utf8(&mut buffer)),
            | None => context.source_spelling(source),
        };
        self.error_type
            .explain(spelling)
            .at(self.severity(), source)
    }
}

impl TranslationPhase for PreprocessorTokenizer {
    type Item = PreprocessorToken;

    fn next_item(&mut self, context: &mut Context) -> Option<PreprocessorToken> {
        loop {
            context.string_cache.undo_str();
            self.current_token_start = self.position(context);
            let Some(input) = self.initial_processor.next_item(context) else {
                break None;
            };
            context.string_cache.push(input);
            break Some(match input {
                | '\n' => self.generate_token(context, PreprocessorTokenType::Newline),
                | '0'..='9' => self.tokenize_number(context),
                | '{' => self.generate_token(context, PreprocessorTokenType::OpeningCurlyBrace),
                | '}' => self.generate_token(context, PreprocessorTokenType::ClosingCurlyBrace),
                | '(' => self.generate_token(context, PreprocessorTokenType::OpeningParenthesis),
                | ')' => self.generate_token(context, PreprocessorTokenType::ClosingParenthesis),
                | '[' => self.generate_token(context, PreprocessorTokenType::OpeningSquareBracket),
                | ']' => self.generate_token(context, PreprocessorTokenType::ClosingSquareBracket),
                | 'L' => self.tokenize_wide_string_or_identifier(context),
                | '\\' if self.consume_ucn(context, true, false) =>
                    self.tokenize_identifier(context, true),
                | c if c.is_alphabetic() || c == '_' =>
                    self.tokenize_keyword_or_identifier(context),
                | '.' => self.tokenize_period_or_number(context),
                | '"' =>
                    if context.is_tokenizing_include_string() {
                        self.tokenize_include_string(context)
                    } else {
                        self.tokenize_string(context)
                    },
                | '\'' => self.tokenize_char(context),
                | '#' => self.tokenize_hash(context),
                | ' ' | '\t' | '\x0b' | '\x0c' => self.tokenize_whitespace(context),
                | '/' => self.tokenize_forward_slash(context),
                | '%' => self.tokenize_percent(context),
                | '<' =>
                    if context.is_tokenizing_include_string() {
                        match self.tokenize_angle_bracket_string(context) {
                            | Some(token) => token,
                            | None => {
                                self.set_position(context, self.current_token_start);
                                context.set_is_tokenizing_include_string(false);
                                context.string_cache.undo_str();
                                continue;
                            },
                        }
                    } else {
                        self.tokenize_left_angle_bracket(context)
                    },
                | '>' => self.tokenize_right_angle_bracket(context),
                | ',' => self.generate_token(context, PreprocessorTokenType::Comma),
                | ';' => self.generate_token(context, PreprocessorTokenType::SemiColon),
                | '?' => self.generate_token(context, PreprocessorTokenType::QuestionMark),
                | '~' => self.generate_token(context, PreprocessorTokenType::Tilde),
                | ':' => self.tokenize_colon(context),
                | '+' => self.tokenize_plus(context),
                | '-' => self.tokenize_minus(context),
                | '*' => self.tokenize_asterisk(context),
                | '^' => self.tokenize_caret(context),
                | '&' => self.tokenize_ampersand(context),
                | '|' => self.tokenize_pipe(context),
                | '!' => self.tokenize_exclamation_mark(context),
                | '=' => self.tokenize_equals(context),
                | unknown => self.generate_other_token(context, unknown),
            });
        }
    }
}

impl PreprocessorTokenizer {
    #[inline(never)]
    #[cold]
    fn generate_error(
        &mut self,
        context: &mut Context,
        error_type: PreprocessorTokenizerErrorType,
    ) {
        let position = self.current_token_start;

        context.preprocessor_tokenizer_error(PreprocessorTokenizerError {
            source_vector: SourceVector::new(
                position,
                self.source_file_index(),
                self.index(context) - position.index,
            ),
            error_type,
            character: None,
        });
    }

    /// Preserves `character` at its own location rather than at the token
    /// start, which precedes any line splice deleted before it.
    #[inline(never)]
    #[cold]
    fn generate_other_token(
        &mut self,
        context: &mut Context,
        character: char,
    ) -> PreprocessorToken {
        let (position, length) = self.initial_processor.last_char_start(character);
        let vector = context.push_source_vector(position, self.source_file_index(), length);
        PreprocessorToken {
            kind:           PreprocessorTokenType::Other,
            contents:       context.string_cache.end_str(),
            source_vectors: SourceVectors::new(vector, vector + 1),
        }
    }

    fn generate_token(
        &mut self,
        context: &mut Context,
        token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        let source_vector = context.push_source_vector(
            self.current_token_start,
            self.source_file_index(),
            self.index(context) - self.current_token_start.index,
        );
        let contents = context.string_cache.end_str();
        PreprocessorToken {
            source_vectors: SourceVectors::new(source_vector, source_vector + 1),
            kind: token_type,
            contents,
        }
    }

    /// Continues a whitespace token after its first character or comment.
    /// Comments become part of the surrounding whitespace (C99
    /// §5.1.1.2p1, phase 3), so the token's spelling is one space.
    fn tokenize_whitespace(&mut self, context: &mut Context) -> PreprocessorToken {
        loop {
            let run = byte_scan::horizontal_space_run(self.initial_processor.raw_remaining());
            _ = self.initial_processor.consume_verbatim(run);
            let last_position = self.position(context);
            match self.initial_processor.next_item(context) {
                | Some('/') => {
                    let (comment_start, _) = self.initial_processor.last_char_start('/');
                    match self.initial_processor.next_item(context) {
                        | Some('/') => self.skip_line_comment(context),
                        | Some('*') => self.skip_block_comment(context, comment_start),
                        | _ => {
                            self.set_position(context, last_position);
                            break;
                        },
                    }
                },
                | Some(c) if c.is_whitespace() && c != '\n' => {},
                | _ => {
                    self.set_position(context, last_position);
                    break;
                },
            }
        }
        context.string_cache.undo_str();
        context.string_cache.push(' ');
        self.generate_token(context, PreprocessorTokenType::Whitespace)
    }

    /// Skips the body of a `//` comment, leaving the line ending unread.
    fn skip_line_comment(&mut self, context: &mut Context) {
        loop {
            let run = byte_scan::raw_verbatim_run(self.initial_processor.raw_remaining());
            _ = self.initial_processor.consume_verbatim(run);
            let last_position = self.position(context);
            match self.initial_processor.next_item(context) {
                | Some('\n') | None => {
                    self.set_position(context, last_position);
                    return;
                },
                | Some(_) => {},
            }
        }
    }

    /// Skips the body of a `/*` comment through its closing `*/`, or to the
    /// end of the input.
    fn skip_block_comment(&mut self, context: &mut Context, comment_start: SourcePosition) {
        let mut after_asterisk = false;
        loop {
            if !after_asterisk {
                let run = byte_scan::raw_block_comment_run(self.initial_processor.raw_remaining());
                _ = self.initial_processor.consume_verbatim(run);
            }
            match self.initial_processor.next_item(context) {
                | Some('/') if after_asterisk => return,
                | Some(c) => after_asterisk = c == '*',
                | None => {
                    context.preprocessor_tokenizer_error(PreprocessorTokenizerError {
                        source_vector: SourceVector::new(
                            comment_start,
                            self.source_file_index(),
                            self.index(context) - comment_start.index,
                        ),
                        error_type:    PreprocessorTokenizerErrorType::UnterminatedBlockComment,
                        character:     None,
                    });
                    return;
                },
            }
        }
    }

    fn tokenize_forward_slash(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);
        let (comment_start, _) = self.initial_processor.last_char_start('/');
        let current = self.initial_processor.next_item(context);
        match current {
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::ForwardSlashEquals)
            },
            | Some('/') => {
                self.skip_line_comment(context);
                self.tokenize_whitespace(context)
            },
            | Some('*') => {
                self.skip_block_comment(context, comment_start);
                self.tokenize_whitespace(context)
            },
            | Some(_) | None => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::ForwardSlash)
            },
        }
    }

    /// See [`InitialProcessor::final_newline_withheld`].
    pub(crate) fn final_newline_withheld(&self) -> bool {
        self.initial_processor.final_newline_withheld()
    }

    pub(crate) fn set_final_newline_withheld(&mut self, withheld: bool) {
        self.initial_processor.set_final_newline_withheld(withheld);
    }

    pub(crate) fn new(source_file_index: u32, source: SharedString) -> Self {
        Self {
            initial_processor:   InitialProcessor::new(source_file_index, source),
            current_token_start: SourcePosition::default(),
        }
    }

    fn tokenize_hash(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);
        let current = self.initial_processor.next_item(context);
        match current {
            | Some('#') => {
                context.string_cache.push('#');
                self.generate_token(context, PreprocessorTokenType::HashHash)
            },
            | _ => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::Hash)
            },
        }
    }

    /// Called just after a backslash; only commit a complete valid UCN.
    fn consume_ucn(&mut self, context: &mut Context, first: bool, append_backslash: bool) -> bool {
        let mut cursor = self.initial_processor.clone();
        let ignored = context.ignore_tokenizer_errors();
        context.set_ignore_tokenizer_errors(true);
        let spelling = (|| {
            let mut spelling = String::from("\\");
            let count = match cursor.next_item(context) {
                | Some('u') => {
                    spelling.push('u');
                    4
                },
                | Some('U') => {
                    spelling.push('U');
                    8
                },
                | _ => return None,
            };
            for _ in 0..count {
                match cursor.next_item(context) {
                    | Some(c) if c.is_ascii_hexdigit() => spelling.push(c),
                    | _ => return None,
                }
            }
            let _ = ucn::decode(&spelling, first)?;
            Some(spelling)
        })();
        context.set_ignore_tokenizer_errors(ignored);
        let Some(spelling) = spelling else {
            return false;
        };
        self.initial_processor = cursor;
        context
            .string_cache
            .push_str(&spelling[usize::from(!append_backslash)..]);
        true
    }

    fn tokenize_identifier(
        &mut self,
        context: &mut Context,
        mut universal: bool,
    ) -> PreprocessorToken {
        loop {
            let run = byte_scan::identifier_run(self.initial_processor.raw_remaining());
            context
                .string_cache
                .push_str(self.initial_processor.consume_verbatim(run));
            let last_position = self.position(context);
            let current = self.initial_processor.next_item(context);
            match current {
                | Some('\\') if self.consume_ucn(context, false, true) => universal = true,
                | Some(c) if c.is_alphanumeric() || c == '_' => context.string_cache.push(c),
                | _ => {
                    self.set_position(context, last_position);
                    let mut token = self.generate_token(context, PreprocessorTokenType::Identifier);
                    if universal {
                        (token.kind, token.contents) = ucn::identifier(context, token.contents);
                    }
                    break token;
                },
            }
        }
    }

    fn tokenize_keyword_or_identifier(&mut self, context: &mut Context) -> PreprocessorToken {
        let mut res = self.tokenize_identifier(context, false);
        if context.string_cache.get(res.contents) == Some("defined") {
            res.kind = PreprocessorTokenType::Defined;
        }
        res
    }

    fn tokenize_wide_string_or_identifier(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);
        let current = self.initial_processor.next_item(context);
        match current {
            // The spelling keeps its opening quote so stringification and
            // literal evaluation see exactly `L"..."` or `L'...'`.
            | Some(quote @ '"') => {
                context.string_cache.push(quote);
                self.tokenize_string(context)
            },
            | Some(quote @ '\'') => {
                context.string_cache.push(quote);
                self.tokenize_char(context)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.tokenize_keyword_or_identifier(context)
            },
        }
    }

    #[inline(always)]
    fn tokenize_string_like(
        &mut self,
        context: &mut Context,
        end_char: char,
        ignore_escapes: bool,
        return_token_type: PreprocessorTokenType,
        unterminated_error_type: PreprocessorTokenizerErrorType,
        newline_error_type: PreprocessorTokenizerErrorType,
    ) -> PreprocessorToken {
        loop {
            let run = byte_scan::raw_literal_run(self.initial_processor.raw_remaining());
            context
                .string_cache
                .push_str(self.initial_processor.consume_verbatim(run));
            let last_position = self.position(context);
            let Some(current) = self.initial_processor.next_item(context) else {
                self.generate_error(context, unterminated_error_type);
                break;
            };
            if !ignore_escapes && current == '\\' {
                context.string_cache.push(current);
                let escape_position = self.position(context);
                match self.initial_processor.next_item(context) {
                    | Some('\n') | None => self.set_position(context, escape_position),
                    | Some(escaped) => context.string_cache.push(escaped),
                }
                continue;
            }
            if current == end_char {
                context.string_cache.push(end_char);
                break;
            }
            if current == '\n' {
                self.set_position(context, last_position);
                context.string_cache.push(end_char);
                self.generate_error(context, newline_error_type);
                break;
            }
            context.string_cache.push(current);
        }

        self.generate_token(context, return_token_type)
    }

    fn tokenize_string(&mut self, context: &mut Context) -> PreprocessorToken {
        self.tokenize_string_like(
            context,
            '"',
            false,
            PreprocessorTokenType::String,
            PreprocessorTokenizerErrorType::UnterminatedString,
            PreprocessorTokenizerErrorType::NewlineInString,
        )
    }

    fn tokenize_char(&mut self, context: &mut Context) -> PreprocessorToken {
        self.tokenize_string_like(
            context,
            '\'',
            false,
            PreprocessorTokenType::Character,
            PreprocessorTokenizerErrorType::UnterminatedCharacter,
            PreprocessorTokenizerErrorType::NewlineInCharacter,
        )
    }

    fn tokenize_angle_bracket_string(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        loop {
            let current = self.initial_processor.next_item(context)?;
            context.string_cache.push(current);
            if current == '>' {
                break;
            }
            if current == '\n' {
                return None;
            }
        }
        Some(self.generate_token(context, PreprocessorTokenType::AngleBracketString))
    }

    fn tokenize_include_string(&mut self, context: &mut Context) -> PreprocessorToken {
        self.tokenize_string_like(
            context,
            '"',
            true,
            PreprocessorTokenType::IncludeString,
            PreprocessorTokenizerErrorType::UnterminatedIncludeString,
            PreprocessorTokenizerErrorType::NewlineInIncludeString,
        )
    }

    fn tokenize_period_or_number(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);
        match self.initial_processor.next_item(context) {
            | Some('.') => self.tokenize_ellipsis(context, last_position),
            | Some(v) if v.is_ascii_digit() => {
                context.string_cache.push(v);
                self.tokenize_number(context)
            },
            | _ => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::Period)
            },
        }
    }

    #[inline(never)]
    #[cold]
    fn tokenize_ellipsis(
        &mut self,
        context: &mut Context,
        last_position: SourcePosition,
    ) -> PreprocessorToken {
        match self.initial_processor.next_item(context) {
            | Some('.') => {
                context.string_cache.push_str("..");
                self.generate_token(context, PreprocessorTokenType::Ellipsis)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::Period)
            },
        }
    }

    fn tokenize_number(&mut self, context: &mut Context) -> PreprocessorToken {
        loop {
            let run = byte_scan::number_run(self.initial_processor.raw_remaining());
            let text = self.initial_processor.consume_verbatim(run);
            context.string_cache.push_str(text);
            if matches!(text.as_bytes().last(), Some(b'e' | b'E' | b'p' | b'P')) {
                let sign_position = self.position(context);
                match self.initial_processor.next_item(context) {
                    | Some(sign @ ('+' | '-')) => {
                        context.string_cache.push(sign);
                        continue;
                    },
                    | None | Some(_) => self.set_position(context, sign_position),
                }
            }
            let last_position = self.position(context);
            let Some(current) = self.initial_processor.next_item(context) else {
                break;
            };
            // A preprocessing number is a spelling, not yet a floating-point
            // value. Keep optional signs after e/E/p/P. Those letters may
            // also be digits in a hexadecimal integer.
            if current == '\\' && self.consume_ucn(context, false, true) {
                continue;
            }
            if current.is_alphanumeric() || matches!(current, '.' | '_') {
                context.string_cache.push(current);
                if matches!(current, 'e' | 'E' | 'p' | 'P') {
                    let sign_position = self.position(context);
                    match self.initial_processor.next_item(context) {
                        | Some(sign @ ('+' | '-')) => context.string_cache.push(sign),
                        | None | Some(_) => self.set_position(context, sign_position),
                    }
                }
                continue;
            }
            self.set_position(context, last_position);
            break;
        }
        // Push trailing null byte so we can we call libc for float parsing.
        context.string_cache.push('\0');
        self.generate_token(context, PreprocessorTokenType::Number)
    }

    fn tokenize_percent(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);
        match self.initial_processor.next_item(context) {
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::PercentEquals)
            },
            // Digraph.
            | Some('>') => {
                context.string_cache.push('>');
                self.generate_token(context, PreprocessorTokenType::ClosingCurlyBrace)
            },
            | Some(':') => {
                context.string_cache.push(':');
                self.tokenize_hash_digraph(context)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::Percent)
            },
        }
    }

    #[inline(never)]
    #[cold]
    fn tokenize_hash_digraph(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);
        match self.initial_processor.next_item(context) {
            | Some('%') => self.tokenize_double_hash_digraph(context, last_position),
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::Hash)
            },
        }
    }

    fn tokenize_double_hash_digraph(
        &mut self,
        context: &mut Context,
        last_position: SourcePosition,
    ) -> PreprocessorToken {
        match self.initial_processor.next_item(context) {
            | Some(':') => {
                context.string_cache.push_str("%:");
                self.generate_token(context, PreprocessorTokenType::HashHash)
            },
            | _ => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::Hash)
            },
        }
    }

    fn tokenize_left_angle_bracket(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some(':') => {
                context.string_cache.push(':');
                self.generate_token(context, PreprocessorTokenType::OpeningSquareBracket)
            },
            | Some('%') => {
                context.string_cache.push('%');
                self.generate_token(context, PreprocessorTokenType::OpeningCurlyBrace)
            },
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::LessThanEquals)
            },
            | Some('<') => {
                context.string_cache.push('<');
                self.tokenize_left_shift(context)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::LessThan)
            },
        }
    }

    fn tokenize_left_shift(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::LessThanLessThanEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::LessThanLessThan)
            },
        }
    }

    fn tokenize_right_angle_bracket(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('>') => {
                context.string_cache.push('>');
                self.tokenize_right_shift(context)
            },
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::GreaterThanEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::GreaterThan)
            },
        }
    }

    fn tokenize_right_shift(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::GreaterThanGreaterThanEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::GreaterThanGreaterThan)
            },
        }
    }

    fn tokenize_colon(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('>') => {
                context.string_cache.push('>');
                self.generate_token(context, PreprocessorTokenType::ClosingSquareBracket)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::Colon)
            },
        }
    }

    fn tokenize_plus(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('+') => {
                context.string_cache.push('+');
                self.generate_token(context, PreprocessorTokenType::PlusPlus)
            },
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::PlusEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::Plus)
            },
        }
    }

    fn tokenize_minus(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('-') => {
                context.string_cache.push('-');
                self.generate_token(context, PreprocessorTokenType::MinusMinus)
            },
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::MinusEquals)
            },
            | Some('>') => {
                context.string_cache.push('>');
                self.generate_token(context, PreprocessorTokenType::Arrow)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::Minus)
            },
        }
    }

    fn tokenize_asterisk(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::AsteriskEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);

                self.generate_token(context, PreprocessorTokenType::Asterisk)
            },
        }
    }

    fn tokenize_caret(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::CaretEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);

                self.generate_token(context, PreprocessorTokenType::Caret)
            },
        }
    }

    fn tokenize_ampersand(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('&') => {
                context.string_cache.push('&');
                self.generate_token(context, PreprocessorTokenType::AmpersandAmpersand)
            },
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::AmpersandEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);

                self.generate_token(context, PreprocessorTokenType::Ampersand)
            },
        }
    }

    fn tokenize_pipe(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('|') => {
                context.string_cache.push('|');
                self.generate_token(context, PreprocessorTokenType::PipePipe)
            },
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::PipeEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);

                self.generate_token(context, PreprocessorTokenType::Pipe)
            },
        }
    }

    fn tokenize_exclamation_mark(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::ExclamationMarkEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);
                self.generate_token(context, PreprocessorTokenType::ExclamationMark)
            },
        }
    }

    fn tokenize_equals(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_position = self.position(context);

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                context.string_cache.push('=');
                self.generate_token(context, PreprocessorTokenType::EqualsEquals)
            },
            | None | Some(_) => {
                self.set_position(context, last_position);

                self.generate_token(context, PreprocessorTokenType::Equals)
            },
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PreprocessorTokenType {
    // Literals
    Identifier,
    /// Suppressed during rescan; remains unavailable in later rescans
    /// (6.10.3.4p2).
    UnavailableIdentifier,
    /// Canonical identity is in contents; this payload preserves source
    /// spelling.
    UniversalIdentifier,
    UnavailableUniversalIdentifier,
    Number,
    String,
    Character,

    /// A non-whitespace character outside the other pp-token categories
    /// (C99 §6.4p3). It can be discarded or stringified in phase 4.
    Other,

    // Expanded from hash operator
    GeneratedString,
    WideGeneratedString,

    // Generated when a macro argument generated no tokens
    Placeholder,

    // Include tokens.
    AngleBracketString,
    IncludeString,

    // Whitespace
    Newline,
    Whitespace,

    // Keywords
    Defined,

    // Punctuation
    // In order of appearance in the C99 standard.
    OpeningSquareBracket,
    ClosingSquareBracket,
    OpeningParenthesis,
    ClosingParenthesis,
    OpeningCurlyBrace,
    ClosingCurlyBrace,
    Period,
    Arrow,
    PlusPlus,
    MinusMinus,
    Ampersand,
    Asterisk,
    Plus,
    Minus,
    Tilde,
    ExclamationMark,
    ForwardSlash,
    Percent,
    LessThanLessThan,
    GreaterThanGreaterThan,
    LessThan,
    GreaterThan,
    LessThanEquals,
    GreaterThanEquals,
    EqualsEquals,
    ExclamationMarkEquals,
    Caret,
    Pipe,
    AmpersandAmpersand,
    PipePipe,
    QuestionMark,
    Colon,
    SemiColon,
    Ellipsis,
    Equals,
    AsteriskEquals,
    ForwardSlashEquals,
    PercentEquals,
    PlusEquals,
    MinusEquals,
    LessThanLessThanEquals,
    GreaterThanGreaterThanEquals,
    AmpersandEquals,
    CaretEquals,
    PipeEquals,
    Comma,
    Hash,
    HashHash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PreprocessorToken {
    pub(crate) kind:           PreprocessorTokenType,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) contents:       StringCacheId,
}

impl PreprocessorToken {
    pub(crate) fn identifier_id(self, context: &Context) -> StringCacheId {
        match self.kind {
            | PreprocessorTokenType::UniversalIdentifier
            | PreprocessorTokenType::UnavailableUniversalIdentifier =>
                context.canonical_identifiers[&self.contents],
            | _ => self.contents,
        }
    }
}

impl Default for PreprocessorToken {
    fn default() -> Self {
        Self {
            kind:           PreprocessorTokenType::WideGeneratedString,
            source_vectors: SourceVectors::empty(),
            contents:       StringCacheId::from_u32(u32::MAX),
        }
    }
}
