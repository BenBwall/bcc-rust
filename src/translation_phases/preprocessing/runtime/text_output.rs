//! Support for printing the preprocessed translation unit as text (`-E`):
//! each phase-6 token with the place the reader has reached in its source
//! file, which line markers follow, and the `#define` lines of the macros in
//! effect where preprocessing stopped, sorted by name, as `-E -dM` prints
//! them for GCC and Clang.
//!
//! C99: translation phases 4-6, §5.1.1.2 paragraph 1 items 4-6, p. 10;
//! PDF p. 22; macro definitions, §6.10.3 paragraphs 9-10, p. 152; PDF p. 164;
//! predefined macro names, §6.10.8, pp. 160-161; PDF pp. 172-173. Printing
//! them is a command-line extension; it does not change replacement.

use std::{
    fmt::Write as _,
    ops::ControlFlow,
};

use super::{
    super::{
        Expander,
        MacroDefinition,
        Preprocessor,
        Token,
    },
    TokenizerFrameType,
};
use crate::{
    configuration::Feature,
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SourcePosition,
        TranslationPhase,
        preprocessor_tokenizer::PreprocessorTokenType,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

impl<'tu> Preprocessor<'tu, '_> {
    /// Calls `visit` with each phase-6 token, as
    /// [`Preprocessor::for_each_iterator_item`] does, and with the source
    /// file the reader is in and its position there once the token was
    /// read. A token a macro expands to is spelled in the macro's
    /// definition, but it belongs on the line of the invocation; the
    /// reader's position gives that line.
    pub(crate) fn for_each_iterator_item_read_at(
        &mut self,
        context: &mut Context<'tu>,
        mut visit: impl FnMut(&mut Context<'tu>, Token, Option<(u32, SourcePosition)>),
    ) {
        self.run(context, |preprocessor| {
            match preprocessor.next_iterator_item() {
                | Some(token) => {
                    let position = preprocessor.reading_position();
                    visit(preprocessor.context, token, position);
                    ControlFlow::Continue(())
                },
                | None => ControlFlow::Break(()),
            }
        });
    }

    /// The `#define` lines of every macro defined after the whole input was
    /// read, sorted by name, in `arena`. A replacement list keeps its tokens'
    /// spellings with each run of whitespace written as one space. Of the
    /// built-in macros, those with a constant value are listed with it, as
    /// GCC and Clang list them; `__FILE__`, `__LINE__`, `__DATE__`,
    /// `__TIME__`, `__COUNTER__`, `_Pragma` and the `__has_*` queries are not.
    pub(crate) fn macro_listing_in<'a>(
        &mut self,
        context: &mut Context<'_>,
        arena: &'a Bump,
    ) -> &'a str {
        let state = &self.resting_mut().state;
        let mut lines = ArenaVec::with_capacity_in(state.macro_definitions.len(), arena);
        for (&name, definition) in &state.macro_definitions {
            let mut line = ArenaString::new_in(arena);
            let spelling = context.string_cache.at(name).trim_end_matches('\0');
            _ = write!(line, "#define {spelling}");
            let tokenizer = match definition {
                | MacroDefinition::BuiltIn => {
                    let Some(value) = builtin_value(context, spelling) else {
                        continue;
                    };
                    _ = write!(line, " {value}");
                    lines.push(line.into_str());
                    continue;
                },
                | MacroDefinition::ObjectLike { tokenizer } => tokenizer,
                | MacroDefinition::FunctionLike {
                    argument_names,
                    tokenizer,
                    is_variadic,
                    variadic_alias,
                } => {
                    line.push('(');
                    for (index, &argument) in argument_names.iter().enumerate() {
                        if index != 0 {
                            line.push(',');
                        }
                        line.push_str(context.string_cache.at(argument).trim_end_matches('\0'));
                    }
                    if *is_variadic {
                        if let Some(alias) = variadic_alias {
                            line.push_str(context.string_cache.at(*alias).trim_end_matches('\0'));
                        } else if !argument_names.is_empty() {
                            line.push(',');
                        }
                        line.push_str("...");
                    }
                    line.push(')');
                    tokenizer
                },
            };
            let mut replacement = tokenizer.clone();
            let mut space = true;
            while let Some(token) = replacement.next_item(context) {
                match token.kind {
                    | PreprocessorTokenType::Newline => break,
                    | PreprocessorTokenType::Whitespace => space = true,
                    | _ => {
                        if space {
                            line.push(' ');
                            space = false;
                        }
                        line.push_str(
                            context
                                .string_cache
                                .at(token.contents)
                                .trim_end_matches('\0'),
                        );
                    },
                }
            }
            lines.push(line.into_str());
        }
        lines.sort_unstable();
        let mut listing = ArenaString::new_in(arena);
        for line in lines {
            listing.push_str(line);
            listing.push('\n');
        }
        listing.into_str()
    }
}

/// The value a built-in macro with a constant expansion lists, or `None`
/// for one whose expansion depends on where it is used or what it is asked.
/// C99: §6.10.8 paragraph 1, p. 160; PDF p. 172.
fn builtin_value(context: &Context<'_>, name: &str) -> Option<&'static str> {
    let configuration = context.configuration;
    Some(match name {
        | "__STDC__" | "__STRICT_ANSI__" | "__STDC_MB_MIGHT_NEQ_WC__" => "1",
        | "__STDC_HOSTED__" =>
            if configuration.hosted() {
                "1"
            } else {
                "0"
            },
        | "__STDC_VERSION__" => configuration
            .standard()
            .version_macro()?
            .trim_end_matches('\0'),
        | "__STDC_EMBED_NOT_FOUND__" if configuration.accepts(Feature::Embed) => "0",
        | "__STDC_EMBED_FOUND__" if configuration.accepts(Feature::Embed) => "1",
        | "__STDC_EMBED_EMPTY__" if configuration.accepts(Feature::Embed) => "2",
        | _ => return None,
    })
}

impl Expander<'_, '_, '_, '_> {
    /// The innermost source file being read, and the reader's position in
    /// it: the live cursor when no macro frame is above the file, otherwise
    /// the cursor the file's frame saved when the macro was pushed.
    fn reading_position(&self) -> Option<(u32, SourcePosition)> {
        let last = self.tokenizer_stack.len().checked_sub(1)?;
        self.tokenizer_stack
            .iter()
            .enumerate()
            .rev()
            .find(|(_, frame)| matches!(frame.frame_type, TokenizerFrameType::SourceFile { .. }))
            .map(|(index, frame)| {
                let cursor = if index == last {
                    &self.tokenizer
                } else {
                    &frame.tokenizer
                };
                (cursor.source_file_index(), cursor.position(self.context))
            })
    }
}
