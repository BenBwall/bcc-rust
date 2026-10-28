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

use std::alloc::Layout;

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
        preprocessing::macro_expansion::FunctionLikeMacroArgument,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// One thing phase 4 did. Names and token spellings are the interned
/// ones phase 4 already holds; what the trace spells itself (arguments, a
/// header name) is in the trace's own arena. Places are physical source
/// segments, which outlive the provenance compaction between output tokens.
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
        /// space; a spelling longer than 160 bytes is cut and ends in `…`.
        arguments: Option<&'pp [(StringCacheId, &'pp str)]>,
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
    /// A group that was skipped, in its physical file, which `#line` does
    /// not change: from byte `after` (or from the line after it unless
    /// `at_line_start`) through the line before byte `until`, where the
    /// directive ending the group starts, or through the end of the file.
    /// C99: §6.10.1 paragraph 6, p. 149; PDF p. 161.
    Skipped {
        file:          u32,
        after:         usize,
        at_line_start: bool,
        until:         Option<usize>,
    },
    /// `#include` named a header, which was found as `file` or not at all.
    /// C99: §6.10.2 paragraphs 2-4, pp. 149-150; PDF pp. 161-162.
    Included {
        directive: SourceVector,
        /// The header name in the directive, delimiters included, when it
        /// is written in one place.
        name:      Option<SourceVector>,
        header:    &'pp str,
        angled:    bool,
        file:      Option<u32>,
    },
}

/// Why and where a trace stopped short: it ran out of its budget of bytes
/// or reached [`MAX_TRACE_EVENTS`], and records nothing after.
#[derive(Debug, Clone)]
pub(crate) struct Truncated {
    /// The top-level replacement the trace holds only in part, if the trace
    /// stopped inside one or as one began.
    pub(crate) expansion: Option<Unfinished>,
}

/// A top-level replacement whose events the trace holds only in part.
#[derive(Debug, Clone)]
pub(crate) struct Unfinished {
    /// The replaced macro name.
    pub(crate) name:      StringCacheId,
    /// The invocation, as in [`TraceEvent::Invoked`].
    pub(crate) expansion: SourceVector,
    pub(crate) end:       SourceVector,
    pub(crate) directive: bool,
    /// Whether the trace holds its start, so the expansion is open: it
    /// stopped while the replacement was read rather than as it began.
    pub(crate) open:      bool,
}

/// The most events a trace records. A macro expansion that multiplies its
/// input can make any number of them, and each becomes part of the
/// explorer's account.
pub(crate) const MAX_TRACE_EVENTS: usize = 50_000;

/// The most bytes a trace takes from its arena, all told: the event vector
/// at each size it grows to, argument lists, and spellings. It is a quarter
/// of a WebAssembly region (`util::vm::REGION_BYTES`), so the trace's arena
/// never fills, and it is apart from every arena phase 4 itself uses.
const TRACE_BYTES: usize = 8 << 20;

/// The most bytes of one argument a trace spells; a longer argument is cut
/// and ends in `…`. Nested invocations each spell what is left of their
/// arguments, so without a cut the spellings of `F(F(F(...)))` grow with
/// the square of its depth.
const MAX_ARGUMENT_BYTES: usize = 160;

/// What phase 4 did, as far as it was recorded. Every byte the trace keeps
/// comes from its own arena and is charged to [`TRACE_BYTES`] before it is
/// allocated; when a charge or an allocation fails, the trace stops
/// ([`Truncated`]) and phase 4 goes on unrecorded, so tracing never makes a
/// program fail that compiles without it. Reading an argument to spell it
/// allocates nothing.
pub(in crate::translation_phases::preprocessing) struct Trace<'pp> {
    arena:     &'pp Bump,
    events:    ArenaVec<'pp, TraceEvent<'pp>>,
    /// Bytes the trace may still take from `arena`.
    budget:    usize,
    truncated: Option<Truncated>,
}

impl<'pp> Preprocessor<'_, 'pp> {
    /// Starts recording a trace in `arena`, which holds nothing else; call
    /// it before preprocessing.
    pub(crate) fn record_trace(&mut self, arena: &'pp Bump) {
        self.resting_mut().state.trace = Some(Trace {
            arena,
            events: ArenaVec::new_in(arena),
            budget: TRACE_BYTES,
            truncated: None,
        });
    }

    /// What phase 4 did so far, if it was recorded, and where the trace
    /// stopped short, if it did.
    pub(crate) fn trace(&mut self) -> (&[TraceEvent<'pp>], Option<&Truncated>) {
        self.resting_mut()
            .state
            .trace
            .as_ref()
            .map_or((&[], None), |trace| {
                (&trace.events, trace.truncated.as_ref())
            })
    }
}

impl<'pp> Expander<'_, '_, 'pp, '_> {
    /// Whether phase 4 is being traced. The recorders below check it at
    /// their call sites, so an untraced run only tests it.
    #[inline(always)]
    pub(in crate::translation_phases::preprocessing) fn tracing(&self) -> bool {
        // Once truncated, the trace records nothing more, so nothing more is
        // spelled or allocated for it.
        self.state
            .trace
            .as_ref()
            .is_some_and(|trace| trace.truncated.is_none())
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
        self.push_trace(
            TraceEvent::Defined {
                name,
                at,
                function_like,
            },
            None,
        );
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
        self.push_trace(TraceEvent::Undefined { name, at }, None);
    }

    /// Records the replacement `frame` is about to start, before it is
    /// pushed: the frames already on the stack are the ones around it.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_invocation(
        &mut self,
        frame: &TokenizerFrame<'_>,
    ) {
        let (name, invocation, end, arguments) = match &frame.frame_type {
            | TokenizerFrameType::ObjectLikeMacroInvocation {
                name,
                invocation,
                invocation_end,
                ..
            } => (*name, invocation, invocation_end, None),
            | TokenizerFrameType::FunctionLikeMacroInvocation {
                name,
                invocation,
                invocation_end,
                arguments,
                ..
            } => (*name, invocation, invocation_end, Some(*arguments)),
            | _ => return,
        };
        let starting = Unfinished {
            name,
            expansion: invocation.clone(),
            end: end.clone(),
            directive: self.state.in_directive,
            open: false,
        };
        let arguments = match arguments.map(|arguments| self.spell_arguments(arguments)) {
            | None => None,
            | Some(Some(spelled)) => Some(spelled),
            | Some(None) => {
                self.stop_trace(Some(starting));
                return;
            },
        };
        let depth = self.replacement_depth();
        // A nested replacement belongs to the outermost one being read,
        // whose frame is lowest on the stack; a frame records the nearest
        // invocation around it only up to an argument's boundary.
        let expansion = self
            .replacement()
            .map_or_else(|| invocation.clone(), |(outermost, _)| outermost);
        let prescan = self.reading_argument();
        let directive = self.state.in_directive;
        self.push_trace(
            TraceEvent::Invoked {
                name,
                expansion,
                end: end.clone(),
                depth,
                arguments,
                prescan,
                directive,
            },
            Some(starting),
        );
    }

    /// Each argument under its parameter's name, spelled into the trace's
    /// arena, or `None` once the trace's budget is spent.
    fn spell_arguments(
        &mut self,
        arguments: &[FunctionLikeMacroArgument<'_>],
    ) -> Option<&'pp [(StringCacheId, &'pp str)]> {
        let context = &*self.context;
        let trace = self.state.trace.as_mut()?;
        let mut spelled = trace.vec_with_capacity(arguments.len())?;
        for argument in arguments {
            // An argument read from its invocation ends with the call's
            // closing parenthesis.
            let spelling = Spelling::of(context, &argument.tokenizer, !argument.substituted);
            spelled.push((argument.name, trace.copy_str(spelling.as_str())?));
        }
        Some(spelled.leak())
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
        // Outside a replacement, the builtin is an expansion of its own.
        let starting = expansion.is_none().then(|| Unfinished {
            name,
            expansion: at.clone(),
            end: at.clone(),
            directive,
            open: false,
        });
        self.push_trace(
            TraceEvent::Builtin {
                name,
                at,
                expansion,
                depth,
                prescan,
                result: result.map(|token| token.contents),
                directive,
            },
            starting,
        );
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
        self.push_trace(
            TraceEvent::Directive {
                name: name.contents,
                at,
            },
            None,
        );
    }

    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_stringified(
        &mut self,
        result: PreprocessorToken,
    ) {
        if let Some((expansion, macro_name)) = self.replacement() {
            let depth = self.replacement_depth();
            self.push_trace(
                TraceEvent::Stringified {
                    expansion,
                    depth,
                    macro_name,
                    result: result.contents,
                },
                None,
            );
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
            let depth = self.replacement_depth();
            self.push_trace(
                TraceEvent::Pasted {
                    expansion,
                    depth,
                    macro_name,
                    left: left.contents,
                    right: right.contents,
                    result: result.map(|token| token.contents),
                },
                None,
            );
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
        self.push_trace(
            TraceEvent::Produced {
                expansion,
                spelling: token.contents,
                spelled,
                white_before,
            },
            None,
        );
    }

    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_condition(
        &mut self,
        directive: SourceVectors,
        taken: bool,
        evaluated: bool,
    ) {
        let directive = self.context.first_source_vector_or_default(directive);
        self.push_trace(
            TraceEvent::Condition {
                directive,
                taken,
                evaluated,
            },
            None,
        );
    }

    /// Records that a skip passed over `file` (a source file index, which
    /// `#line` may have renamed) from byte `after`, or from the line after
    /// it unless `at_line_start`, up to the byte `until` that starts the
    /// directive ending the group, or to the end of the file.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn trace_skipped(
        &mut self,
        file: u32,
        (after, at_line_start): (usize, bool),
        until: Option<usize>,
    ) {
        let file = self.context.physical_source_file(file);
        self.push_trace(
            TraceEvent::Skipped {
                file,
                after,
                at_line_start,
                until,
            },
            None,
        );
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
        let Some(header) = self
            .state
            .trace
            .as_mut()
            .and_then(|trace| trace.copy_str(header))
        else {
            self.stop_trace(None);
            return;
        };
        self.push_trace(
            TraceEvent::Included {
                directive,
                name,
                header,
                angled,
                file,
            },
            None,
        );
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

    /// Records `event`, or stops the trace if it has no room for it. The
    /// replacement `event` starts, if any, is `starting`.
    fn push_trace(&mut self, event: TraceEvent<'pp>, starting: Option<Unfinished>) {
        if let Some(trace) = self.state.trace.as_mut()
            && trace.truncated.is_none()
            && !trace.record(event)
        {
            self.stop_trace(starting);
        }
    }

    /// Stops the trace, which then records, spells and allocates nothing
    /// more. The replacement it holds only in part is the outermost one
    /// being read, or else `starting`, the one that was about to begin.
    #[cold]
    #[inline(never)]
    fn stop_trace(&mut self, starting: Option<Unfinished>) {
        let open = self
            .tokenizer_stack
            .iter()
            .find_map(|frame| match &frame.frame_type {
                | TokenizerFrameType::ObjectLikeMacroInvocation {
                    name,
                    invocation,
                    invocation_end,
                    ..
                }
                | TokenizerFrameType::FunctionLikeMacroInvocation {
                    name,
                    invocation,
                    invocation_end,
                    ..
                } => Some(Unfinished {
                    name:      *name,
                    expansion: invocation.clone(),
                    end:       invocation_end.clone(),
                    directive: self.state.in_directive,
                    open:      true,
                }),
                | _ => None,
            });
        if let Some(trace) = self
            .state
            .trace
            .as_mut()
            .filter(|trace| trace.truncated.is_none())
        {
            trace.truncated = Some(Truncated {
                expansion: open.or(starting),
            });
        }
    }
}

impl<'pp> Trace<'pp> {
    /// Records `event`, unless the trace holds [`MAX_TRACE_EVENTS`] or
    /// cannot afford a larger vector.
    fn record(&mut self, event: TraceEvent<'pp>) -> bool {
        let length = self.events.len();
        if length == self.events.capacity() {
            let capacity = (length * 2).clamp(64, MAX_TRACE_EVENTS);
            if length == MAX_TRACE_EVENTS
                || !Layout::array::<TraceEvent<'_>>(capacity)
                    .is_ok_and(|layout| self.charge(layout))
                || self.events.try_reserve_exact(capacity - length).is_err()
            {
                return false;
            }
        }
        self.events.push(event);
        true
    }

    /// An empty vector with room for `capacity` values, charged to the
    /// budget, or `None` if the budget or the arena cannot hold it.
    fn vec_with_capacity<T>(&mut self, capacity: usize) -> Option<ArenaVec<'pp, T>> {
        let mut values = ArenaVec::new_in(self.arena);
        if capacity != 0 {
            if !self.charge(Layout::array::<T>(capacity).ok()?) {
                return None;
            }
            values.try_reserve_exact(capacity).ok()?;
        }
        Some(values)
    }

    /// `text` copied into the trace's arena, as for
    /// [`Trace::vec_with_capacity`].
    fn copy_str(&mut self, text: &str) -> Option<&'pp str> {
        let mut bytes = self.vec_with_capacity(text.len())?;
        bytes.extend_from_slice(text.as_bytes());
        std::str::from_utf8(bytes.leak()).ok()
    }

    /// Takes room for a block of `layout` from the budget, with the most
    /// padding aligning it can add, if the budget has that much left.
    fn charge(&mut self, layout: Layout) -> bool {
        let cost = layout.size().saturating_add(layout.align());
        self.budget
            .checked_sub(cost)
            .map(|left| self.budget = left)
            .is_some()
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

/// Appended to an argument's spelling where it is cut.
const CUT: &str = " …";

/// An argument spelled with each run of whitespace as one space and none at
/// either end, as `#` would spell it without the quotes, cut after
/// [`MAX_ARGUMENT_BYTES`]. It is spelled in place, so spelling allocates
/// nothing.
struct Spelling {
    bytes: [u8; MAX_ARGUMENT_BYTES + CUT.len()],
    len:   usize,
    cut:   bool,
}

impl Spelling {
    /// Spells the argument left in `tokens`, without reading them: nothing
    /// is reported and no provenance is recorded. With `closing`, the
    /// tokens end with the call's closing parenthesis, which is not part of
    /// the argument. Spelling stops where it is cut, so it reads no more of
    /// a long argument than it spells.
    fn of(context: &Context<'_>, tokens: &TokenSource<'_>, closing: bool) -> Self {
        let mut spelling = Self {
            bytes: [0; MAX_ARGUMENT_BYTES + CUT.len()],
            len:   0,
            cut:   false,
        };
        let mut space = false;
        // Each token is spelled once the next is read, so the last one, the
        // closing parenthesis, can be left out.
        let mut pending = None;
        for token in tokens.spellings() {
            if let Some(previous) = pending.replace(token) {
                spelling.append(context, previous, &mut space);
                if spelling.cut {
                    return spelling;
                }
            }
        }
        if let Some(last) = pending
            && !(closing && last.0 == PreprocessorTokenType::ClosingParenthesis)
        {
            spelling.append(context, last, &mut space);
        }
        spelling
    }

    fn append(
        &mut self,
        context: &Context<'_>,
        (kind, contents): (PreprocessorTokenType, StringCacheId),
        space: &mut bool,
    ) {
        match kind {
            | PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline => {
                *space = self.len != 0;
            },
            | PreprocessorTokenType::Placeholder => {},
            | _ => {
                if std::mem::take(space) {
                    self.push(" ");
                }
                self.push(context.string_cache.at(contents).trim_end_matches('\0'));
            },
        }
    }

    /// Appends `text`, or as much of it as fits, cut at a character
    /// boundary and followed by [`CUT`].
    fn push(&mut self, text: &str) {
        if self.cut {
            return;
        }
        let mut length = text.len();
        let room = MAX_ARGUMENT_BYTES - self.len;
        if length > room {
            length = room;
            while !text.is_char_boundary(length) {
                length -= 1;
            }
            self.cut = true;
        }
        self.bytes[self.len..self.len + length].copy_from_slice(&text.as_bytes()[..length]);
        self.len += length;
        if self.cut {
            while self.len > 0 && self.bytes[self.len - 1] == b' ' {
                self.len -= 1;
            }
            self.bytes[self.len..self.len + CUT.len()].copy_from_slice(CUT.as_bytes());
            self.len += CUT.len();
        }
    }

    fn as_str(&self) -> &str {
        // Only whole characters are copied.
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }
}
