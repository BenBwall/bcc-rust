//! The front of the compiler, phase by phase, from the compiler's own
//! output for each phase.
//!
//! - Phases 1-2: the source's logical lines, from the lexer's own [`splice`],
//!   and where its remaps say trigraphs, new-lines and line splices changed the
//!   text.
//! - Phase 3: the source's preprocessing tokens and white space, read from a
//!   [`TokenSource`] over the main file, as phase 4 reads them.
//! - Phase 4: one preprocessing run with the phase-4 trace on. Its output is
//!   written as `-E` writes it and listed as `--tokens` lists it, so the same
//!   run also gives the explorer's token stage; the trace becomes
//!   [`ExplorerPreprocessing`].
//!
//! C99: translation phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10; PDF
//! pp. 21-22.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "The explorer hands owned text and vectors across its interface, outside every arena."
)]

use std::{
    cell::Cell,
    fmt::Write as _,
    ops::Range,
    path::Path,
};

use super::{
    ExplorerArgument,
    ExplorerConditional,
    ExplorerDefinition,
    ExplorerDirective,
    ExplorerExpansion,
    ExplorerExpansionStep,
    ExplorerInclude,
    ExplorerLineRange,
    ExplorerLogicalLine,
    ExplorerOptions,
    ExplorerOutputLine,
    ExplorerPpToken,
    ExplorerPreprocessing,
    ExplorerSkippedLines,
    ExplorerSourceChange,
    compilation::{
        INPUT,
        header_search,
        new_context,
    },
};
use crate::{
    configuration::{
        CompilerConfiguration,
        Feature,
    },
    pipeline::{
        Pragmas,
        TextOptions,
        TextWriter,
        describe_token,
        with_preprocessor,
        would_paste,
    },
    translation_phases::{
        Context,
        SourcePosition,
        SourceVector,
        TranslationPhase,
        preprocessing::TraceEvent,
        preprocessor_tokenizer::{
            PreprocessorTokenType,
            RemapKind,
            TokenSource,
            splice,
        },
    },
    util::{
        bump::Bump,
        string_cache::StringCacheId,
    },
};

/// The explorer's front-end stages.
pub(super) struct FrontEnd {
    pub(super) logical_lines:      Vec<ExplorerLogicalLine>,
    pub(super) pp_tokens:          Vec<ExplorerPpToken>,
    pub(super) preprocessed:       String,
    pub(super) preprocessed_lines: Vec<ExplorerOutputLine>,
    pub(super) preprocessing:      ExplorerPreprocessing,
    pub(super) tokens:             String,
}

/// Runs phases 1 through 6 on `source` for each front-end view.
pub(super) fn front_end(
    source: &str,
    configuration: CompilerConfiguration,
    options: &ExplorerOptions,
) -> FrontEnd {
    let trigraphs = configuration.accepts(Feature::Trigraphs);
    let (preprocessed, preprocessed_lines, preprocessing, header_names) =
        phase_four(source, configuration, options);
    FrontEnd {
        logical_lines: logical_lines(source, trigraphs),
        pp_tokens: pp_tokens(source, configuration, options, &header_names),
        preprocessed,
        preprocessed_lines,
        preprocessing,
        tokens: tokens(source, configuration, options),
    }
}

/// Phases 1 and 2: the logical lines [`splice`] leaves, each with what its
/// remaps record, and the UTF-8 characters each line keeps as one source
/// character.
fn logical_lines(source: &str, trigraphs: bool) -> Vec<ExplorerLogicalLine> {
    let scratch = Bump::new();
    let (text, remaps) = splice(source, &scratch, trigraphs, true);
    let physical = Physical::new(source);
    // Each remap with the offset in `source` where its spelling starts.
    let mut delta = 0;
    let remapped: Vec<(usize, usize, RemapKind)> = remaps
        .iter()
        .map(|remap| {
            let at = remap.clean + delta;
            delta += growth(remap.kind);
            (remap.clean, at, remap.kind)
        })
        .collect();
    // Where the byte at `clean` is in `source`: after any splice deleted
    // just before it (`after_splices`), or where such a splice begins.
    let original = |clean: usize, after_splices: bool| {
        let mut count = remapped.partition_point(|(at, ..)| *at < clean);
        if after_splices {
            count += remapped[count..]
                .iter()
                .take_while(|(at, _, kind)| *at == clean && matches!(kind, RemapKind::Deleted(_)))
                .count();
        }
        count.checked_sub(1).map_or(clean, |last| {
            let (at, from, kind) = remapped[last];
            clean - at + from + growth(kind)
        })
    };
    let mut lines = Vec::new();
    let mut start = 0;
    let mut remaining = remapped.iter().peekable();
    while start < text.len() {
        let end = text[start..]
            .find('\n')
            .map_or(text.len(), |offset| start + offset);
        let line = &text[start..end];
        let first = physical.line(original(start, false));
        let last =
            physical.line(original(end.min(text.len()), true).min(source.len().saturating_sub(1)));
        // Columns count on from the last one asked for, which keeps a long
        // line with many changes linear; the two passes below each ask in
        // order.
        let counted = Cell::new((start, 1));
        let column = |clean: usize| {
            let (from, column) = match counted.get() {
                | (from, column) if from <= clean => (from, column),
                | _ => (start, 1),
            };
            let column = column + count(&text[from..clean]);
            counted.set((clean, column));
            column
        };
        let mut changes = Vec::new();
        while let Some(&&(at, from, kind)) = remaining.peek() {
            if at > end {
                break;
            }
            _ = remaining.next();
            let (kind, spelled, replacement) = match kind {
                | RemapKind::Deleted(length) => ("splice", &source[from..from + length], ""),
                | RemapKind::Trigraph => ("trigraph", &source[from..from + 3], &text[at..=at]),
                | RemapKind::CarriageReturnLineFeed => ("crlf", "\r\n", "\n"),
            };
            let (line, source_column) = physical.position(from);
            changes.push(ExplorerSourceChange {
                kind: kind.to_owned(),
                column: column(at),
                line,
                source_column,
                spelling: spelled.to_owned(),
                replacement: replacement.to_owned(),
            });
        }
        // Runs of characters outside ASCII, which phase 1 reads from UTF-8
        // as one source character each.
        let mut offset = 0;
        while let Some(run) = line[offset..].find(|c: char| !c.is_ascii()) {
            let run_start = offset + run;
            let run_end = line[run_start..]
                .find(|c: char| c.is_ascii())
                .map_or(line.len(), |length| run_start + length);
            let (source_line, source_column) = physical.position(original(start + run_start, true));
            changes.push(ExplorerSourceChange {
                kind: "extended".to_owned(),
                column: column(start + run_start),
                line: source_line,
                source_column,
                spelling: line[run_start..run_end].to_owned(),
                replacement: line[run_start..run_end].to_owned(),
            });
            offset = run_end;
        }
        changes.sort_by_key(|change| change.column);
        lines.push(ExplorerLogicalLine {
            text: line.to_owned(),
            source: ExplorerLineRange {
                start: first,
                end:   last.max(first),
            },
            changes,
        });
        start = end + 1;
    }
    lines
}

/// How many bytes longer the source is than the text a remap leaves.
fn growth(kind: RemapKind) -> usize {
    match kind {
        | RemapKind::Deleted(length) => length,
        | RemapKind::Trigraph => 2,
        | RemapKind::CarriageReturnLineFeed => 1,
    }
}

/// Phase 3 over the main file: every entry the lexer saved, read as phase 4
/// reads it. Tokens inside `header_names` (byte ranges of the source) were
/// read by `#include` as one header name.
fn pp_tokens(
    source: &str,
    configuration: CompilerConfiguration,
    options: &ExplorerOptions,
    header_names: &[Range<usize>],
) -> Vec<ExplorerPpToken> {
    let tu = Bump::new();
    let mut context = new_context(&tu, configuration, options);
    let file = context.intern_source_file(Path::new(INPUT));
    context.record_source_text(file, source);
    let pp = Bump::new();
    let mut reader = TokenSource::new(&mut context, &pp, file, source);
    let mut entries: Vec<ExplorerPpToken> = Vec::new();
    let mut line_start = true;
    let mut leading_space = false;
    while let Some(token) = reader.next_item(&mut context) {
        let segments = context.get_source_vectors(token.source_vectors);
        let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
            continue;
        };
        let (start, end) = (first.index as usize, last.end());
        // Counting from the last segment's own column keeps a long line
        // linear; only a block comment's segment can span lines.
        let spelled = source.get(last.index as usize..end).unwrap_or_default();
        let (end_line, end_column) = if spelled.contains(['\n', '\r']) {
            context
                .physical_position(file, end)
                .unwrap_or((last.line, last.column))
        } else {
            (last.line, last.column + count(spelled))
        };
        let kind = pp_token_kind(token.kind);
        let spelling = context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0');
        let canonical =
            matches!(token.kind, PreprocessorTokenType::UniversalIdentifier).then(|| {
                context
                    .string_cache
                    .at(token.identifier_id(&context))
                    .trim_end_matches('\0')
                    .to_owned()
            });
        let whitespace = matches!(
            token.kind,
            PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
        );
        entries.push(ExplorerPpToken {
            kind: kind.to_owned(),
            spelling: if whitespace && kind == "whitespace" {
                " ".to_owned()
            } else {
                spelling.to_owned()
            },
            line: first.line,
            column: first.column,
            end_line,
            end_column,
            comment: kind == "whitespace"
                && source
                    .get(start..end)
                    .is_some_and(|text| text.contains("/*") || text.contains("//")),
            line_start: line_start && !whitespace,
            leading_space: leading_space && !whitespace,
            header_name: !whitespace
                && header_names
                    .iter()
                    .any(|name| name.start <= start && end <= name.end),
            canonical,
        });
        match token.kind {
            | PreprocessorTokenType::Newline => {
                line_start = true;
                leading_space = false;
            },
            | PreprocessorTokenType::Whitespace => leading_space = true,
            | _ => {
                line_start = false;
                leading_space = false;
            },
        }
    }
    entries
}

/// The C99 name of a phase-3 entry's lexical category.
/// C99: §6.4 paragraphs 1-3, p. 49; PDF p. 61.
fn pp_token_kind(kind: PreprocessorTokenType) -> &'static str {
    match kind {
        | PreprocessorTokenType::Identifier
        | PreprocessorTokenType::UniversalIdentifier
        | PreprocessorTokenType::UnavailableIdentifier
        | PreprocessorTokenType::UnavailableUniversalIdentifier
        | PreprocessorTokenType::Defined => "identifier",
        | PreprocessorTokenType::Number => "pp-number",
        | PreprocessorTokenType::String
        | PreprocessorTokenType::GeneratedString
        | PreprocessorTokenType::WideGeneratedString => "string-literal",
        | PreprocessorTokenType::Character => "character-constant",
        | PreprocessorTokenType::Other | PreprocessorTokenType::Placeholder => "other",
        | PreprocessorTokenType::Whitespace => "whitespace",
        | PreprocessorTokenType::Newline => "newline",
        | _ => "punctuator",
    }
}

/// Phase 4 alone, with the trace on: the text phase 4 leaves (adjacent
/// strings still apart, each token spelled as written) as `-E` lays it
/// out, where each of its lines came from, what phase 4 did, and the byte
/// ranges of the main file's header names.
fn phase_four(
    source: &str,
    configuration: CompilerConfiguration,
    options: &ExplorerOptions,
) -> (
    String,
    Vec<ExplorerOutputLine>,
    ExplorerPreprocessing,
    Vec<Range<usize>>,
) {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let mut context = new_context(&tu, configuration, options);
    // The preprocessor opens the source under this name, as this index.
    let main_file = context.intern_source_file(Path::new(INPUT));
    let text_arena = Bump::new();
    let text_options = TextOptions {
        as_written: true,
        ..TextOptions::default()
    };
    let mut writer = TextWriter::new(&text_arena, text_options, main_file);
    let mut pragmas = Pragmas::default();
    let mut lines = OutputLines::default();
    // Each token is written as it is read, with its provenance still live,
    // so a large source keeps no token array.
    let (preprocessing, header_names) = with_preprocessor(
        &mut context,
        Path::new(INPUT),
        source,
        header_search(&tu, options),
        |mut preprocessor, context, _pp| {
            preprocessor.record_trace();
            preprocessor.keep_strings_apart();
            let mut ordinal = 0;
            preprocessor.for_each_iterator_item_read_at(context, |context, token, at| {
                pragmas.write_before(context, ordinal, &mut writer);
                ordinal += 1;
                let root = at.as_ref().and_then(|at| at.root);
                let place = writer.token(context, token, at);
                lines.written(&writer.text, context, main_file, place, root);
            });
            Trace::new(context).read(preprocessor.trace())
        },
    );
    pragmas.write_before(&context, usize::MAX, &mut writer);
    writer.finish();
    lines.written(&writer.text, &context, main_file, None, None);
    (
        writer.text.as_str().to_owned(),
        lines.lines,
        preprocessing,
        header_names,
    )
}

/// Where each line of the preprocessed text came from, as the writer
/// writes it.
#[derive(Default)]
struct OutputLines {
    lines:   Vec<ExplorerOutputLine>,
    /// Where the line being written starts.
    start:   usize,
    /// How much of the text was searched for new-lines.
    scanned: usize,
    /// Where that line's first token came from.
    current: Option<ExplorerOutputLine>,
}

impl OutputLines {
    /// Accounts for what the writer wrote since the last call, which ended
    /// with a token written for `place` (a source file index and byte
    /// offset) while the outermost file was read at `root`; at the end of
    /// the text, `place` is `None`. A line takes the place of its first
    /// token. A line between two tokens' lines takes the next token's file:
    /// a blank one counts back from that token's line, and a line marker or
    /// pragma has no line.
    ///
    /// Files are told apart by their physical identity, which `#line` does
    /// not change, and only the text written since the last call is
    /// searched, so a long line costs one pass.
    fn written(
        &mut self,
        text: &str,
        context: &Context<'_>,
        main_file: u32,
        place: Option<(u32, usize)>,
        root: Option<(u32, SourcePosition)>,
    ) {
        let main_file = context.physical_source_file(main_file);
        let token = place.and_then(|(file, offset)| {
            let line = Positions::line(context, file, offset)?;
            let physical = context.physical_source_file(file);
            let include = root
                .filter(|(root, _)| {
                    physical != main_file && context.physical_source_file(*root) == main_file
                })
                // The main file's reader stands at the end of the
                // `#include` line, before its new-line.
                .and_then(|(root, position)| Positions::line(context, root, position.index));
            Some(ExplorerOutputLine {
                file: context.get_source_file(physical).display().to_string(),
                line: Some(line),
                include,
            })
        });
        let ends: Vec<usize> = text[self.scanned..]
            .match_indices('\n')
            .map(|(index, _)| self.scanned + index)
            .collect();
        self.scanned = text.len();
        if ends.is_empty() {
            if self.current.is_none() {
                self.current = token;
            }
            return;
        }
        // The line a token was written on ends first; until one was, every
        // line is between tokens.
        let mut start = self.start;
        let mut between = &ends[..];
        if let Some(current) = self.current.take() {
            self.lines.push(current);
            start = ends[0] + 1;
            between = &ends[1..];
        }
        for (index, &end) in between.iter().enumerate() {
            let blank = text[start..end].trim().is_empty();
            let back = u32::try_from(between.len() - index).unwrap_or(u32::MAX);
            self.lines
                .push(token.as_ref().map_or_else(Default::default, |token| {
                    ExplorerOutputLine {
                        line: token
                            .line
                            .and_then(|line| line.checked_sub(back))
                            .filter(|&line| blank && line > 0),
                        ..token.clone()
                    }
                }));
            start = end + 1;
        }
        self.start = start;
        self.current = token;
    }
}

/// The `--tokens` listing of phases 5-7, without the diagnostics it
/// interleaves. Each token is described as it is read, so a large source
/// keeps no token array: in WebAssembly a region holds one.
fn tokens(source: &str, configuration: CompilerConfiguration, options: &ExplorerOptions) -> String {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let mut context = new_context(&tu, configuration, options);
    let mut text = String::new();
    let mut scratch = Bump::new();
    with_preprocessor(
        &mut context,
        Path::new(INPUT),
        source,
        header_search(&tu, options),
        |mut preprocessor, context, _pp| {
            preprocessor.for_each_iterator_item(context, |context, token| {
                scratch.reset();
                _ = writeln!(text, "{}", describe_token(token, context, &scratch));
            });
        },
    );
    text
}

/// Turns the phase-4 trace into the explorer's account of it.
struct Trace<'c, 'tu> {
    context:      &'c Context<'tu>,
    positions:    Positions,
    output:       ExplorerPreprocessing,
    /// Each expansion's invocation place and how its result ends.
    keys:         Vec<((u32, u32), ResultEnd)>,
    header_names: Vec<Range<usize>>,
}

impl<'c, 'tu> Trace<'c, 'tu> {
    fn new(context: &'c Context<'tu>) -> Self {
        Self {
            context,
            positions: Positions::default(),
            output: ExplorerPreprocessing::default(),
            keys: Vec::new(),
            header_names: Vec::new(),
        }
    }

    fn read(mut self, trace: &[TraceEvent<'_>]) -> (ExplorerPreprocessing, Vec<Range<usize>>) {
        for event in trace {
            self.event(event);
        }
        (self.output, self.header_names)
    }

    fn event(&mut self, event: &TraceEvent<'_>) {
        match event {
            | TraceEvent::Defined {
                name,
                at,
                function_like,
            } => self.definition(
                *name,
                at,
                if *function_like { "function" } else { "object" },
            ),
            | TraceEvent::Undefined { name, at } => self.definition(*name, at, "undef"),
            | TraceEvent::Invoked {
                name,
                expansion,
                end,
                depth,
                arguments,
                prescan,
                directive,
            } => {
                if *depth == 0 {
                    let invocation = self.invocation(expansion, end, *name);
                    self.open_expansion(expansion, end, invocation, *directive);
                }
                let arguments = arguments
                    .iter()
                    .flat_map(|arguments| arguments.iter())
                    .map(|&(parameter, argument)| ExplorerArgument {
                        parameter: self.spell(parameter).to_owned(),
                        argument:  self.spell(argument).to_owned(),
                    })
                    .collect();
                let step = ExplorerExpansionStep {
                    kind: "replace".to_owned(),
                    prescan: *prescan,
                    macro_name: self.spell(*name).to_owned(),
                    depth: *depth,
                    parent: None,
                    arguments,
                    operands: Vec::new(),
                    result: String::new(),
                };
                self.step(expansion, step);
            },
            | TraceEvent::Builtin {
                name,
                at,
                expansion,
                depth,
                prescan,
                result,
                directive,
            } => {
                let result = result
                    .map(|result| self.spell(result).to_owned())
                    .unwrap_or_default();
                let step = ExplorerExpansionStep {
                    kind:       "builtin".to_owned(),
                    prescan:    *prescan,
                    macro_name: self.spell(*name).to_owned(),
                    depth:      *depth,
                    parent:     None,
                    arguments:  Vec::new(),
                    operands:   Vec::new(),
                    result:     result.clone(),
                };
                let outermost = expansion.as_ref().unwrap_or(at);
                if expansion.is_none() {
                    self.open_expansion(at, at, self.spell(*name).to_owned(), *directive);
                    if let Some(index) = self.expansion(at)
                        && !*directive
                    {
                        self.output.expansions[index].result = result;
                    }
                }
                self.step(outermost, step);
            },
            | TraceEvent::Directive { name, at } => {
                let (file, line, presumed) = self.place(at);
                self.output.directives.push(ExplorerDirective {
                    file,
                    line,
                    presumed,
                    name: self.spell(*name).to_owned(),
                    directive: self.line_text(at),
                });
            },
            | TraceEvent::Stringified {
                expansion,
                depth,
                macro_name,
                result,
            } => {
                let step = ExplorerExpansionStep {
                    kind:       "stringify".to_owned(),
                    prescan:    false,
                    macro_name: self.spell(*macro_name).to_owned(),
                    depth:      *depth,
                    parent:     None,
                    arguments:  Vec::new(),
                    operands:   Vec::new(),
                    result:     self.spell(*result).to_owned(),
                };
                self.step(expansion, step);
            },
            | TraceEvent::Pasted {
                expansion,
                depth,
                macro_name,
                left,
                right,
                result,
            } => {
                let step = ExplorerExpansionStep {
                    kind:       "paste".to_owned(),
                    prescan:    false,
                    macro_name: self.spell(*macro_name).to_owned(),
                    depth:      *depth,
                    parent:     None,
                    arguments:  Vec::new(),
                    operands:   [self.spell(*left).to_owned(), self.spell(*right).to_owned()]
                        .into(),
                    result:     result
                        .map(|result| self.spell(result).to_owned())
                        .unwrap_or_default(),
                };
                self.step(expansion, step);
            },
            | TraceEvent::Produced {
                expansion,
                spelling,
                spelled,
                white_before,
            } => self.produced(expansion, *spelling, spelled.as_ref(), *white_before),
            | TraceEvent::Condition {
                directive,
                taken,
                evaluated,
            } => {
                let (file, line, presumed) = self.place(directive);
                self.output.conditionals.push(ExplorerConditional {
                    file,
                    line,
                    presumed,
                    directive: self.line_text(directive),
                    taken: *taken,
                    evaluated: *evaluated,
                });
            },
            | TraceEvent::Skipped {
                file,
                first_line,
                last_line,
            } => self.output.skipped.push(ExplorerSkippedLines {
                file:  self.file_name(*file),
                lines: ExplorerLineRange {
                    start: *first_line,
                    end:   *last_line,
                },
            }),
            | TraceEvent::Included {
                directive,
                name,
                header,
                angled,
                file,
            } => {
                let (including, line, _) = self.place(directive);
                if let Some(name) = name.as_ref().filter(|_| including == INPUT) {
                    self.header_names.push(name.index as usize..name.end());
                }
                self.output.includes.push(ExplorerInclude {
                    file: including,
                    line,
                    header: self.spell(*header).to_owned(),
                    angled: *angled,
                    found: file.map(|file| self.file_name(file)),
                    system: file.is_some_and(|file| self.context.has_system_header_part(file)),
                    lines: file
                        .and_then(|file| self.context.source_text(file))
                        .map_or(0, |text| {
                            u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
                        }),
                });
            },
        }
    }

    /// Starts the expansion of an invocation in the source, from
    /// `expansion` through `end`.
    fn open_expansion(
        &mut self,
        expansion: &SourceVector,
        end: &SourceVector,
        invocation: String,
        directive: bool,
    ) {
        let (file, line, presumed) = self.place(expansion);
        let column = self.column(expansion);
        let (end_line, end_column) = self
            .positions
            .position(self.context, end.source_file_index, end.end())
            .filter(|_| end.source_file_index == expansion.source_file_index)
            .unwrap_or((line, column));
        self.output.expansions.push(ExplorerExpansion {
            file,
            line,
            column,
            end_line,
            end_column,
            presumed,
            invocation,
            directive,
            result: String::new(),
            steps: Vec::new(),
        });
        self.keys.push((key(expansion), ResultEnd::default()));
    }

    fn definition(&mut self, name: StringCacheId, at: &SourceVector, kind: &str) {
        let (file, line, _) = self.place(at);
        self.output.definitions.push(ExplorerDefinition {
            file,
            line,
            name: self.spell(name).to_owned(),
            kind: kind.to_owned(),
            directive: self.line_text(at),
        });
    }

    /// Adds `step` to the expansion at `expansion`, the latest one there.
    ///
    /// Its parent is the replacement it happened in: the latest step one
    /// level up. Replacement frames form a stack, so the frame being read
    /// at each level is the one pushed there last.
    fn step(&mut self, expansion: &SourceVector, mut step: ExplorerExpansionStep) {
        let Some(index) = self.expansion(expansion) else {
            return;
        };
        let steps = &mut self.output.expansions[index].steps;
        let levels = &mut self.keys[index].1.levels;
        let depth = step.depth as usize;
        step.parent = depth
            .checked_sub(1)
            .and_then(|up| levels.get(up))
            .and_then(|&parent| u32::try_from(parent).ok());
        levels.truncate(depth);
        levels.push(steps.len());
        steps.push(step);
    }

    /// Appends a token to the expansion's result, spaced as `-E` spaces it.
    fn produced(
        &mut self,
        expansion: &SourceVector,
        spelling: StringCacheId,
        spelled: Option<&SourceVector>,
        white_before: bool,
    ) {
        let Some(index) = self.expansion(expansion) else {
            return;
        };
        let spelling = self.spell(spelling);
        let first = spelling.bytes().next();
        let ResultEnd { end, last, .. } = &mut self.keys[index].1;
        let result = &mut self.output.expansions[index].result;
        if !result.is_empty() {
            let space = match (*end, spelled) {
                | (Some((file, end, line)), Some(vector)) if file == vector.source_file_index => {
                    let start = vector.index as usize;
                    if start == end {
                        false
                    } else if start > end && vector.line == line {
                        true
                    } else {
                        white_before || would_paste(*last, first)
                    }
                },
                | _ => white_before || would_paste(*last, first),
            };
            if space {
                result.push(' ');
            }
        }
        result.push_str(spelling);
        *end = spelled.map(|vector| (vector.source_file_index, vector.end(), vector.line));
        *last = (first, spelling.bytes().last());
    }

    /// The latest expansion whose invocation is at `expansion`.
    fn expansion(&self, expansion: &SourceVector) -> Option<usize> {
        let wanted = key(expansion);
        self.keys.iter().rposition(|(key, _)| *key == wanted)
    }

    /// The invocation from its name through `end`, as written, with each
    /// run of white space as one space.
    fn invocation(&self, start: &SourceVector, end: &SourceVector, name: StringCacheId) -> String {
        let physical = self.context.physical_source_file(start.source_file_index);
        let text = (start.source_file_index == end.source_file_index)
            .then(|| self.context.source_text(physical))
            .flatten()
            .and_then(|text| text.get(start.index as usize..end.end().max(start.end())));
        match text {
            | Some(text) => text
                .split(|c: char| c.is_whitespace() || c == '\\')
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            | None => self.spell(name).to_owned(),
        }
    }

    /// The physical file name and line of `vector`, and the place `#line`
    /// gives it as `file:line` when that differs.
    fn place(&self, vector: &SourceVector) -> (String, u32, Option<String>) {
        let line = Positions::line(
            self.context,
            vector.source_file_index,
            vector.index as usize,
        )
        .unwrap_or(vector.line);
        let file = self.file_name(vector.source_file_index);
        let named = self
            .context
            .get_source_file(vector.source_file_index)
            .display()
            .to_string();
        let presumed =
            (named != file || vector.line != line).then(|| format!("{named}:{}", vector.line));
        (file, line, presumed)
    }

    /// The physical character column of `vector`.
    fn column(&self, vector: &SourceVector) -> u32 {
        self.positions
            .position(
                self.context,
                vector.source_file_index,
                vector.index as usize,
            )
            .map_or(vector.column, |(_, column)| column)
    }

    fn file_name(&self, file: u32) -> String {
        let physical = self.context.physical_source_file(file);
        self.context.get_source_file(physical).display().to_string()
    }

    /// The physical line that holds `vector`, without its new-line.
    fn line_text(&self, vector: &SourceVector) -> String {
        let physical = self.context.physical_source_file(vector.source_file_index);
        let Some(text) = self.context.source_text(physical) else {
            return String::new();
        };
        let at = (vector.index as usize).min(text.len());
        let start = text[..at].rfind(['\n', '\r']).map_or(0, |index| index + 1);
        let end = text[at..]
            .find(['\n', '\r'])
            .map_or(text.len(), |index| at + index);
        text[start..end].trim().to_owned()
    }

    fn spell(&self, id: StringCacheId) -> &'c str {
        self.context.string_cache.at(id).trim_end_matches('\0')
    }
}

/// Where an expansion's result so far ends, to space its next token as
/// `-E` would.
#[derive(Default)]
struct ResultEnd {
    /// The last token's file, the byte offset just past it, and its line.
    end:    Option<(u32, usize, u32)>,
    /// The first and last bytes of its spelling.
    last:   (Option<u8>, Option<u8>),
    /// The index of the latest step at each depth, for the next step's
    /// parent.
    levels: Vec<usize>,
}

/// The identity of an invocation place.
fn key(vector: &SourceVector) -> (u32, u32) {
    (vector.source_file_index, vector.index)
}

/// Character counts and positions in the physical source, whose lines end
/// at LF, CRLF or CR, as the compiler counts them.
struct Physical<'a> {
    source: &'a str,
    starts: Vec<usize>,
    /// The last place found: its line start, byte offset and column, to
    /// count on from on the same line.
    last:   Cell<(usize, usize, u32)>,
}

impl<'a> Physical<'a> {
    fn new(source: &'a str) -> Self {
        let bytes = source.as_bytes();
        let starts = std::iter::once(0)
            .chain(
                bytes
                    .iter()
                    .enumerate()
                    .filter(|&(index, &byte)| {
                        byte == b'\n' || (byte == b'\r' && bytes.get(index + 1) != Some(&b'\n'))
                    })
                    .map(|(index, _)| index + 1),
            )
            .collect();
        Self {
            source,
            starts,
            last: Cell::new((0, 0, 1)),
        }
    }

    /// The 1-based line of byte `offset`.
    fn line(&self, offset: usize) -> u32 {
        u32::try_from(self.starts.partition_point(|&start| start <= offset)).unwrap_or(u32::MAX)
    }

    /// The 1-based line and character column of byte `offset`.
    fn position(&self, offset: usize) -> (u32, u32) {
        let line = self.line(offset);
        let start = self.starts[line as usize - 1];
        let offset = offset.min(self.source.len());
        let (from, column) = match self.last.get() {
            | (last_start, last_offset, column) if last_start == start && last_offset <= offset =>
                (last_offset, column),
            | _ => (start, 1),
        };
        let column = column + count(self.source.get(from..offset).unwrap_or_default());
        self.last.set((start, offset, column));
        (line, column)
    }
}

/// Physical lines and columns of byte offsets in time proportional to the
/// text read: a line by binary search over its file's line starts, and a
/// column by counting on from the last one found on the same line, since
/// the explorer asks for places in reading order. A long line costs one
/// pass, not one pass per token.
#[derive(Default)]
struct Positions {
    /// The last column found: its physical file, line start, byte offset
    /// and column.
    last: Cell<Option<(u32, usize, usize, u32)>>,
}

impl Positions {
    /// The physical file behind `file`, its text, and the 1-based line of
    /// `offset` with where that line starts.
    fn locate<'c>(
        context: &'c Context<'_>,
        file: u32,
        offset: usize,
    ) -> Option<(u32, &'c str, usize, usize, u32)> {
        let physical = context.physical_source_file(file);
        let text = context.source_text(physical)?;
        let starts = context.source_line_starts(physical)?;
        let offset = offset.min(text.len());
        let line = starts.partition_point(|&start| start <= offset).max(1);
        Some((
            physical,
            text,
            starts[line - 1],
            offset,
            u32::try_from(line).unwrap_or(u32::MAX),
        ))
    }

    /// The 1-based physical line of byte `offset` of `file`, which `#line`
    /// may have renamed.
    fn line(context: &Context<'_>, file: u32, offset: usize) -> Option<u32> {
        Self::locate(context, file, offset).map(|(.., line)| line)
    }

    /// The 1-based physical line and character column of byte `offset`.
    fn position(&self, context: &Context<'_>, file: u32, offset: usize) -> Option<(u32, u32)> {
        let (physical, text, start, offset, line) = Self::locate(context, file, offset)?;
        let (from, column) = match self.last.get() {
            | Some((last_file, last_start, last_offset, column))
                if last_file == physical && last_start == start && last_offset <= offset =>
                (last_offset, column),
            | _ => (start, 1),
        };
        let column = column + count(text.get(from..offset).unwrap_or_default());
        self.last.set(Some((physical, start, offset, column)));
        Some((line, column))
    }
}

fn count(text: &str) -> u32 {
    u32::try_from(text.chars().count()).unwrap_or(u32::MAX)
}
