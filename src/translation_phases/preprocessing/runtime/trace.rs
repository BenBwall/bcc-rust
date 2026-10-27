//! An opt-in record of what phase 4 did, in the order it did it, for the
//! explorer's account of preprocessing: which macro names were replaced,
//! where and with which arguments, what `#` and `##` produced, which tokens
//! each replacement left in the output, which conditional groups were
//! processed or skipped, and which headers were included. Recording never
//! changes what phase 4 does, and a preprocessor that does not record pays
//! one branch at each place below.
//!
//! For `#define TWICE(x) x + x` followed by `TWICE(2)`, the trace holds a
//! definition of `TWICE`, an invocation with `x` = `2`, and the three tokens
//! `2`, `+` and `2`, each tagged with the place of `TWICE(2)`.
//!
//! C99: translation phase 4, §5.1.1.2 paragraph 1 item 4, p. 10; PDF p. 22;
//! conditional inclusion, §6.10.1 paragraph 6, p. 149; PDF p. 161; source
//! file inclusion, §6.10.2, pp. 149-150; PDF pp. 161-162; macro
//! replacement, §6.10.3 paragraphs 9-10, p. 152; PDF p. 164; rescanning,
//! §6.10.3.4 paragraph 1, p. 155; PDF p. 167.

use super::{
    super::{
        Expander,
        Preprocessor,
    },
    TokenizerFrame,
    TokenizerFrameType,
};
use crate::{
    translation_phases::{
        Context,
        SourceVector,
        SourceVectors,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
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

/// One thing phase 4 did. Spellings are interned; places are physical
/// source segments, which outlive the provenance compaction between output
/// tokens.
#[derive(Debug, Clone)]
pub(crate) enum TraceEvent<'pp> {
    /// `#define` stored a macro definition.
    /// C99: §6.10.3 paragraphs 9-10, p. 152; PDF p. 164.
    Defined {
        name:          StringCacheId,
        /// The macro name in the directive.
        at:            SourceVector,
        function_like: bool,
    },
    /// `#undef` ended a macro definition.
    /// C99: §6.10.3.5 paragraph 2, p. 155; PDF p. 167.
    Undefined {
        name: StringCacheId,
        at:   SourceVector,
    },
    /// A macro name was replaced.
    /// C99: §6.10.3 paragraphs 9-10, p. 152; PDF p. 164; §6.10.3.4
    /// paragraph 1, p. 155; PDF p. 167.
    Invoked {
        name:      StringCacheId,
        /// The place of the outermost invocation this replacement belongs
        /// to, which identifies the expansion.
        expansion: SourceVector,
        /// Where that invocation ends: the replaced name, or the closing
        /// parenthesis of a call.
        end:       SourceVector,
        /// How many replacements were already being rescanned around this
        /// one: 0 for a name in the source.
        depth:     u32,
        /// For a function-like macro, each parameter and the argument it
        /// stands for, spelled as written with each run of whitespace as one
        /// space.
        arguments: Option<&'pp [(StringCacheId, StringCacheId)]>,
    },
    /// `#` spelled an argument as a character string literal.
    /// C99: §6.10.3.2 paragraph 2, p. 153; PDF p. 165.
    Stringified {
        expansion:  SourceVector,
        /// The macro whose replacement list holds the `#`.
        macro_name: StringCacheId,
        result:     StringCacheId,
    },
    /// `##` concatenated two tokens. An empty spelling is a placemarker.
    /// C99: §6.10.3.3 paragraphs 2-3, p. 154; PDF p. 166.
    Pasted {
        expansion:  SourceVector,
        macro_name: StringCacheId,
        left:       StringCacheId,
        right:      StringCacheId,
        /// `None` when two placemarkers gave a placemarker.
        result:     Option<StringCacheId>,
    },
    /// A token that a replacement produced left phase 4.
    /// C99: §6.10.3.4 paragraph 1, p. 155; PDF p. 167.
    Produced {
        expansion:    SourceVector,
        spelling:     StringCacheId,
        /// Where the token is spelled, if it has a place.
        spelled:      Option<SourceVector>,
        /// Whether whitespace precedes the token where it is spelled.
        white_before: bool,
    },
    /// A conditional directive decided whether its group is processed.
    /// C99: §6.10.1 paragraph 6, p. 149; PDF p. 161.
    Condition {
        directive: SourceVector,
        taken:     bool,
    },
    /// The physical lines of a group that was skipped, both inclusive.
    /// C99: §6.10.1 paragraph 6, p. 149; PDF p. 161.
    Skipped {
        file:       u32,
        first_line: u32,
        last_line:  u32,
    },
    /// `#include` named a header, which was found as `file` or not at all.
    /// C99: §6.10.2 paragraphs 2-4, pp. 149-150; PDF pp. 161-162.
    Included {
        directive: SourceVector,
        /// The header name in the directive, delimiters included, when it
        /// is written in one place.
        name:      Option<SourceVector>,
        header:    StringCacheId,
        angled:    bool,
        file:      Option<u32>,
    },
}

impl<'pp> Preprocessor<'_, 'pp> {
    /// Starts recording a trace; call it before preprocessing.
    pub(crate) fn record_trace(&mut self) {
        let state = &mut self.resting_mut().state;
        state.trace = Some(ArenaVec::new_in(state.arena));
    }

    /// What phase 4 did so far, if it was recorded.
    pub(crate) fn trace(&mut self) -> &[TraceEvent<'pp>] {
        self.resting_mut()
            .state
            .trace
            .as_deref()
            .unwrap_or_default()
    }
}

impl<'pp> Expander<'_, '_, 'pp, '_> {
    /// Whether phase 4 is being traced. The recorders below check it at
    /// their call sites, so an untraced run only tests it.
    #[inline(always)]
    pub(in crate::translation_phases::preprocessing) fn tracing(&self) -> bool {
        self.state.trace.is_some()
    }

    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_defined(
        &mut self,
        name: PreprocessorToken,
        function_like: bool,
    ) {
        let at = self
            .context
            .first_source_vector_or_default(name.source_vectors);
        let name = name.identifier_id(self.context);
        self.push_trace(TraceEvent::Defined {
            name,
            at,
            function_like,
        });
    }

    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_undefined(
        &mut self,
        name: PreprocessorToken,
    ) {
        let at = self
            .context
            .first_source_vector_or_default(name.source_vectors);
        let name = name.identifier_id(self.context);
        self.push_trace(TraceEvent::Undefined { name, at });
    }

    /// Records the replacement `frame` is about to start, before it is
    /// pushed: the frames already on the stack are the ones around it.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_invocation(
        &mut self,
        frame: &TokenizerFrame<'_>,
    ) {
        let (name, expansion, end, arguments) = match &frame.frame_type {
            | TokenizerFrameType::ObjectLikeMacroInvocation {
                name,
                invocation,
                invocation_end,
                ..
            } => (*name, invocation.clone(), invocation_end.clone(), None),
            | TokenizerFrameType::FunctionLikeMacroInvocation {
                name,
                invocation,
                invocation_end,
                arguments,
                ..
            } => {
                let arena = self.state.arena;
                let mut spelled = ArenaVec::with_capacity_in(arguments.len(), arena);
                for argument in *arguments {
                    let mut tokens = argument.tokenizer.clone();
                    let mut read = ArenaVec::new_in(self.scratch);
                    while let Some(token) = tokens.next_item(self.context) {
                        read.push(token);
                    }
                    // An argument read from its invocation ends with the
                    // call's closing parenthesis.
                    if !argument.substituted
                        && read.last().is_some_and(|token| {
                            token.kind == PreprocessorTokenType::ClosingParenthesis
                        })
                    {
                        _ = read.pop();
                    }
                    spelled.push((argument.name, spell(self.context, self.scratch, &read)));
                }
                (
                    *name,
                    invocation.clone(),
                    invocation_end.clone(),
                    Some(&*spelled.leak()),
                )
            },
            | _ => return,
        };
        let depth = self.replacement_depth();
        self.push_trace(TraceEvent::Invoked {
            name,
            expansion,
            end,
            depth,
            arguments,
        });
    }

    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_stringified(
        &mut self,
        result: PreprocessorToken,
    ) {
        if let Some((expansion, macro_name)) = self.replacement() {
            self.push_trace(TraceEvent::Stringified {
                expansion,
                macro_name,
                result: result.contents,
            });
        }
    }

    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_pasted(
        &mut self,
        left: PreprocessorToken,
        right: PreprocessorToken,
        result: Option<PreprocessorToken>,
    ) {
        if let Some((expansion, macro_name)) = self.replacement() {
            self.push_trace(TraceEvent::Pasted {
                expansion,
                macro_name,
                left: left.contents,
                right: right.contents,
                result: result.map(|token| token.contents),
            });
        }
    }

    /// Records `token` as output of the replacement being read, if any.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_produced(
        &mut self,
        token: PreprocessorToken,
    ) {
        let Some((expansion, _)) = self.replacement() else {
            return;
        };
        let spelled = self
            .context
            .get_source_vectors(token.source_vectors)
            .first()
            .cloned();
        let white_before = spelled.as_ref().is_some_and(|vector| {
            self.context
                .source_text(vector.source_file_index)
                .and_then(|text| text.as_bytes().get((vector.index as usize).checked_sub(1)?))
                .is_some_and(u8::is_ascii_whitespace)
        });
        self.push_trace(TraceEvent::Produced {
            expansion,
            spelling: token.contents,
            spelled,
            white_before,
        });
    }

    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_condition(
        &mut self,
        directive: SourceVectors,
        taken: bool,
    ) {
        let directive = self.context.first_source_vector_or_default(directive);
        self.push_trace(TraceEvent::Condition { directive, taken });
    }

    /// Records that the lines of `file` after `after_line` and before
    /// `until_line` were skipped, if there are any.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_skipped(
        &mut self,
        file: u32,
        after_line: u32,
        until_line: u32,
    ) {
        if until_line > after_line + 1 {
            self.push_trace(TraceEvent::Skipped {
                file,
                first_line: after_line + 1,
                last_line: until_line - 1,
            });
        }
    }

    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_included(
        &mut self,
        directive: SourceVectors,
        name: (SourceVectors, &str, bool),
        file: Option<u32>,
    ) {
        let (vectors, header, angled) = name;
        let directive = self.context.first_source_vector_or_default(directive);
        let segments = self.context.get_source_vectors(vectors);
        let name = match (segments.first(), segments.last()) {
            | (Some(first), Some(last))
                if first.source_file_index == last.source_file_index
                    && last.end() >= first.index as usize =>
                Some(SourceVector {
                    length: u32::try_from(last.end() - first.index as usize)
                        .expect("a header name is shorter than its file"),
                    ..first.clone()
                }),
            | _ => None,
        };
        let header = self.context.string_cache.intern(header);
        self.push_trace(TraceEvent::Included {
            directive,
            name,
            header,
            angled,
            file,
        });
    }

    /// The outermost replacement being read, by the place of its
    /// invocation, and the name of the innermost macro being replaced.
    fn replacement(&self) -> Option<(SourceVector, StringCacheId)> {
        let mut invocations = self.tokenizer_stack.iter().filter_map(invocation);
        let (outermost, innermost) = (invocations.next()?, invocations.next_back());
        Some((outermost.0.clone(), innermost.unwrap_or(outermost).1))
    }

    /// How many macro replacements are being read.
    fn replacement_depth(&self) -> u32 {
        let depth = self.tokenizer_stack.iter().filter_map(invocation).count();
        u32::try_from(depth).unwrap_or(u32::MAX)
    }

    fn push_trace(&mut self, event: TraceEvent<'pp>) {
        if let Some(trace) = &mut self.state.trace {
            trace.push(event);
        }
    }
}

/// The invocation place and macro name of a replacement frame.
fn invocation<'f>(frame: &'f TokenizerFrame<'_>) -> Option<(&'f SourceVector, StringCacheId)> {
    match &frame.frame_type {
        | TokenizerFrameType::ObjectLikeMacroInvocation {
            invocation, name, ..
        }
        | TokenizerFrameType::FunctionLikeMacroInvocation {
            invocation, name, ..
        } => Some((invocation, *name)),
        | _ => None,
    }
}

/// `tokens` spelled with each run of whitespace as one space and none at
/// either end, as `#` would spell them without the quotes.
fn spell(context: &mut Context<'_>, scratch: &Bump, tokens: &[PreprocessorToken]) -> StringCacheId {
    let mut text = ArenaString::new_in(scratch);
    let mut space = false;
    for token in tokens {
        match token.kind {
            | PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline => {
                space = !text.is_empty();
            },
            | PreprocessorTokenType::Placeholder => {},
            | _ => {
                if std::mem::take(&mut space) {
                    text.push(' ');
                }
                text.push_str(
                    context
                        .string_cache
                        .at(token.contents)
                        .trim_end_matches('\0'),
                );
            },
        }
    }
    context.string_cache.intern(&*text)
}
