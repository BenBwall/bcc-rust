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
            TokenSource,
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
        /// Whether the name was read from an argument of an enclosing
        /// replacement, which is macro-replaced before it is substituted.
        /// C99: §6.10.3.1 paragraph 1, p. 153; PDF p. 165.
        prescan:   bool,
        /// Whether a directive read the name, as `#if` reads its condition.
        directive: bool,
    },
    /// A predefined macro such as `__LINE__` was replaced by its value.
    /// C99: §6.10.8 paragraph 1, p. 160; PDF p. 172.
    Builtin {
        name:      StringCacheId,
        at:        SourceVector,
        /// The outermost invocation around it, if it was read inside one.
        expansion: Option<SourceVector>,
        /// How many replacements were being read around it.
        depth:     u32,
        /// Whether it was read from an argument being prescanned.
        prescan:   bool,
        /// The token it became, if it became one token.
        result:    Option<StringCacheId>,
        directive: bool,
    },
    /// A directive was executed; the ones a skipped group holds are not.
    /// C99: §6.10 paragraph 2, pp. 146-147; PDF pp. 158-159.
    Directive {
        name: StringCacheId,
        at:   SourceVector,
    },
    /// `#` spelled an argument as a character string literal.
    /// C99: §6.10.3.2 paragraph 2, p. 153; PDF p. 165.
    Stringified {
        expansion:  SourceVector,
        /// How many replacements are being read: the `#` belongs to the
        /// innermost, one level up.
        depth:      u32,
        /// The macro whose replacement list holds the `#`.
        macro_name: StringCacheId,
        result:     StringCacheId,
    },
    /// `##` concatenated two tokens. An empty spelling is a placemarker.
    /// C99: §6.10.3.3 paragraphs 2-3, p. 154; PDF p. 166.
    Pasted {
        expansion:  SourceVector,
        /// As for [`TraceEvent::Stringified`].
        depth:      u32,
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
        /// Whether its condition was evaluated: the arms after a processed
        /// group are skipped without it.
        evaluated: bool,
    },
    /// The physical lines of a group that was skipped, both inclusive, in
    /// its physical file, which `#line` does not change.
    /// C99: §6.10.1 paragraph 6, p. 149; PDF p. 161.
    Skipped {
        file:       u32,
        first_line: u32,
        last_line:  u32,
    },
    /// The trace stopped here: it holds [`MAX_TRACE_EVENTS`] events.
    Truncated,
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

/// The most events a trace records. The trace lives in the preprocessing
/// arena, beside the files and macros, and a macro expansion that multiplies
/// its input can make any number of events, so the trace stops before it
/// could fill that arena (one fixed region in WebAssembly).
pub(crate) const MAX_TRACE_EVENTS: usize = 50_000;

/// The most bytes of one argument a trace spells; a longer argument is cut
/// and ends in `…`. Nested invocations each spell what is left of their
/// arguments, so without a cut the spellings of `F(F(F(...)))` grow with
/// the square of its depth.
const MAX_ARGUMENT_BYTES: usize = 160;

/// The most bytes of arguments a trace spells, all told; past it the trace
/// stops as it does at [`MAX_TRACE_EVENTS`]. The spellings are interned, so
/// they stay for the rest of the compilation.
const MAX_SPELLED_BYTES: usize = 1 << 20;

/// What phase 4 did, as far as it was recorded, and what recording it
/// spent. Recording stops (with [`TraceEvent::Truncated`]) before it could
/// fill an arena, so tracing never makes a program fail that compiles
/// without it.
pub(in crate::translation_phases::preprocessing) struct Trace<'pp> {
    events:  ArenaVec<'pp, TraceEvent<'pp>>,
    /// Bytes of arguments spelled so far.
    spelled: usize,
    /// Whether the trace stopped.
    full:    bool,
}

impl<'pp> Preprocessor<'_, 'pp> {
    /// Starts recording a trace; call it before preprocessing.
    pub(crate) fn record_trace(&mut self) {
        let state = &mut self.resting_mut().state;
        state.trace = Some(Trace {
            events:  ArenaVec::new_in(state.arena),
            spelled: 0,
            full:    false,
        });
    }

    /// What phase 4 did so far, if it was recorded.
    pub(crate) fn trace(&mut self) -> &[TraceEvent<'pp>] {
        self.resting_mut()
            .state
            .trace
            .as_ref()
            .map_or(&[], |trace| &trace.events)
    }
}

impl<'pp> Expander<'_, '_, 'pp, '_> {
    /// Whether phase 4 is being traced. The recorders below check it at
    /// their call sites, so an untraced run only tests it.
    #[inline(always)]
    pub(in crate::translation_phases::preprocessing) fn tracing(&self) -> bool {
        // Once truncated, the trace records nothing more, so nothing more is
        // spelled or allocated for it.
        self.state.trace.as_ref().is_some_and(|trace| !trace.full)
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
                let mut bytes = 0;
                for argument in *arguments {
                    // An argument read from its invocation ends with the
                    // call's closing parenthesis.
                    let (spelling, length) = spell_argument(
                        self.context,
                        self.scratch,
                        argument.tokenizer.clone(),
                        !argument.substituted,
                    );
                    bytes += length;
                    spelled.push((argument.name, spelling));
                }
                if let Some(trace) = &mut self.state.trace {
                    trace.spelled += bytes;
                    if trace.spelled > MAX_SPELLED_BYTES {
                        trace.stop();
                        return;
                    }
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
        // A nested replacement belongs to the outermost one being read,
        // whose frame is lowest on the stack; a frame records the nearest
        // invocation around it only up to an argument's boundary.
        let expansion = self
            .replacement()
            .map_or(expansion, |(outermost, _)| outermost);
        let prescan = self.reading_argument();
        let directive = self.state.in_directive;
        self.push_trace(TraceEvent::Invoked {
            name,
            expansion,
            end,
            depth,
            arguments,
            prescan,
            directive,
        });
    }

    /// Records that the predefined macro `token` names became `result`.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_builtin(
        &mut self,
        token: PreprocessorToken,
        result: Option<PreprocessorToken>,
    ) {
        let at = self
            .context
            .first_source_vector_or_default(token.source_vectors);
        let name = token.identifier_id(self.context);
        let expansion = self.replacement().map(|(outermost, _)| outermost);
        let directive = self.state.in_directive;
        let depth = self.replacement_depth();
        let prescan = self.reading_argument();
        self.push_trace(TraceEvent::Builtin {
            name,
            at,
            expansion,
            depth,
            prescan,
            result: result.map(|token| token.contents),
            directive,
        });
    }

    /// Records the directive whose name is `name`.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_directive(
        &mut self,
        name: PreprocessorToken,
    ) {
        let at = self
            .context
            .first_source_vector_or_default(name.source_vectors);
        self.push_trace(TraceEvent::Directive {
            name: name.contents,
            at,
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
                depth: self.replacement_depth(),
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
                depth: self.replacement_depth(),
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
        evaluated: bool,
    ) {
        let directive = self.context.first_source_vector_or_default(directive);
        self.push_trace(TraceEvent::Condition {
            directive,
            taken,
            evaluated,
        });
    }

    /// Records the lines of `file` (a source file index, which `#line` may
    /// have renamed) that a skip passed over: from byte `after`, or from
    /// the line after it unless `at_line_start`, through the line before the
    /// byte `until` that starts the directive ending the group, or through
    /// the end of the file. Lines are physical, as an editor counts them.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_skipped(
        &mut self,
        file: u32,
        (after, at_line_start): (usize, bool),
        until: Option<usize>,
    ) {
        let physical = self.context.physical_source_file(file);
        let line = |offset: usize| self.context.physical_position(physical, offset);
        let Some((start, _)) = line(after) else {
            return;
        };
        let first_line = if at_line_start { start } else { start + 1 };
        let last_line = match until {
            | Some(until) => line(until).map(|(line, _)| line.saturating_sub(1)),
            | None => {
                let length = self.context.source_text(physical).map_or(0, str::len);
                line(length).map(|(line, column)| if column == 1 { line - 1 } else { line })
            },
        };
        if let Some(last_line) = last_line.filter(|&last| last >= first_line) {
            self.push_trace(TraceEvent::Skipped {
                file: physical,
                first_line,
                last_line,
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

    /// Whether the token being read comes from an argument of the innermost
    /// replacement, rather than from its replacement list.
    fn reading_argument(&self) -> bool {
        self.tokenizer_stack
            .iter()
            .rev()
            .take_while(|frame| invocation(frame).is_none())
            .any(|frame| {
                matches!(
                    frame.frame_type,
                    TokenizerFrameType::FunctionLikeMacroArgument { .. }
                )
            })
    }

    /// How many macro replacements are being read.
    fn replacement_depth(&self) -> u32 {
        let depth = self.tokenizer_stack.iter().filter_map(invocation).count();
        u32::try_from(depth).unwrap_or(u32::MAX)
    }

    /// Records `event`, unless the trace already holds
    /// [`MAX_TRACE_EVENTS`]; the first event past that is recorded as
    /// [`TraceEvent::Truncated`].
    fn push_trace(&mut self, event: TraceEvent<'pp>) {
        if let Some(trace) = self.state.trace.as_mut().filter(|trace| !trace.full) {
            if trace.events.len() < MAX_TRACE_EVENTS {
                trace.events.push(event);
            } else {
                trace.stop();
            }
        }
    }
}

impl Trace<'_> {
    /// Ends the trace with [`TraceEvent::Truncated`]; nothing more is
    /// recorded, spelled or allocated for it.
    fn stop(&mut self) {
        if !self.full {
            self.events.push(TraceEvent::Truncated);
            self.full = true;
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

/// The argument `tokens` replays, spelled with each run of whitespace as one
/// space and none at either end, as `#` would spell it without the quotes,
/// and how many bytes that took. With `closing`, the replay ends with the
/// call's closing parenthesis, which is not part of the argument. Spelling
/// stops after [`MAX_ARGUMENT_BYTES`], so it reads no more of a long
/// argument than it spells.
fn spell_argument(
    context: &mut Context<'_>,
    scratch: &Bump,
    mut tokens: TokenSource<'_>,
    closing: bool,
) -> (StringCacheId, usize) {
    let mut text = ArenaString::new_in(scratch);
    let mut space = false;
    let mut append = |context: &Context<'_>,
                      text: &mut ArenaString<'_>,
                      token: PreprocessorToken| match token.kind {
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
    };
    // Each token is spelled once the next is read, so the last one, the
    // closing parenthesis, can be left out.
    let mut pending = None;
    while let Some(token) = tokens.next_item(context) {
        if let Some(previous) = pending.replace(token) {
            append(context, &mut text, previous);
        }
        if text.len() >= MAX_ARGUMENT_BYTES {
            // One long token, such as a string literal, is cut too.
            let mut cut = MAX_ARGUMENT_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            let mut shown = ArenaString::new_in(scratch);
            shown.push_str(&text[..cut]);
            shown.push_str(" …");
            let length = shown.len();
            return (context.string_cache.intern(&*shown), length);
        }
    }
    if let Some(last) = pending
        && !(closing && last.kind == PreprocessorTokenType::ClosingParenthesis)
    {
        append(context, &mut text, last);
    }
    let length = text.len();
    (context.string_cache.intern(&*text), length)
}
