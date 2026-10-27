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
    clippy::disallowed_methods,
    reason = "The explorer hands owned text and vectors across its interface, outside every arena."
)]

use std::{
    fmt::Write as _,
    ops::Range,
    path::Path,
};

use super::{
    ExplorerArgument,
    ExplorerConditional,
    ExplorerDefinition,
    ExplorerExpansion,
    ExplorerExpansionStep,
    ExplorerInclude,
    ExplorerLineRange,
    ExplorerLogicalLine,
    ExplorerOptions,
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
    pub(super) logical_lines: Vec<ExplorerLogicalLine>,
    pub(super) pp_tokens:     Vec<ExplorerPpToken>,
    pub(super) preprocessed:  String,
    pub(super) preprocessing: ExplorerPreprocessing,
    pub(super) tokens:        String,
}

/// Runs phases 1 through 6 on `source` for each front-end view.
pub(super) fn front_end(
    source: &str,
    configuration: CompilerConfiguration,
    options: &ExplorerOptions,
) -> FrontEnd {
    let trigraphs = configuration.accepts(Feature::Trigraphs);
    let (preprocessed, preprocessing, tokens, header_names) =
        preprocess(source, configuration, options);
    FrontEnd {
        logical_lines: logical_lines(source, trigraphs),
        pp_tokens: pp_tokens(source, configuration, options, &header_names),
        preprocessed,
        preprocessing,
        tokens,
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
        remapped
            .iter()
            .take_while(|(at, _, kind)| {
                *at < clean
                    || (after_splices && *at == clean && matches!(kind, RemapKind::Deleted(_)))
            })
            .last()
            .map_or(clean, |&(at, from, kind)| clean - at + from + growth(kind))
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
        let column = |clean: usize| count(&text[start..clean]) + 1;
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
        let (end_line, end_column) = context
            .physical_position(file, end)
            .unwrap_or((last.line, last.column + last.length));
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

/// Phases 4-6 in one run with the trace on: the `-E` text, what phase 4
/// did, the `--tokens` listing, and the byte ranges of the main file's
/// header names.
fn preprocess(
    source: &str,
    configuration: CompilerConfiguration,
    options: &ExplorerOptions,
) -> (String, ExplorerPreprocessing, String, Vec<Range<usize>>) {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let mut context = new_context(&tu, configuration, options);
    let (items, (preprocessing, header_names)) = with_preprocessor(
        &mut context,
        Path::new(INPUT),
        source,
        header_search(&tu, options),
        |mut preprocessor, context, _pp| {
            preprocessor.record_trace();
            let mut items = Vec::new();
            preprocessor.for_each_iterator_item_read_at(context, |context, mut token, at| {
                token.source_vectors = context.retain_token_source(token.source_vectors);
                items.push((token, at));
            });
            (items, Trace::new(context).read(preprocessor.trace()))
        },
    );
    let text_arena = Bump::new();
    let main_file = context.intern_source_file(Path::new(INPUT));
    let mut writer = TextWriter::new(&text_arena, TextOptions::default(), main_file);
    let mut pragmas = Pragmas::default();
    let mut tokens = String::new();
    let mut scratch = Bump::new();
    for (ordinal, (token, at)) in items.into_iter().enumerate() {
        pragmas.write_before(&context, ordinal, &mut writer);
        writer.token(&context, token, at);
        scratch.reset();
        _ = writeln!(tokens, "{}", describe_token(token, &context, &scratch));
    }
    pragmas.write_before(&context, usize::MAX, &mut writer);
    writer.finish();
    (
        writer.text.as_str().to_owned(),
        preprocessing,
        tokens,
        header_names,
    )
}

/// Turns the phase-4 trace into the explorer's account of it.
struct Trace<'c, 'tu> {
    context:      &'c Context<'tu>,
    output:       ExplorerPreprocessing,
    /// Each expansion's invocation place and how its result ends.
    keys:         Vec<((u32, u32), ResultEnd)>,
    header_names: Vec<Range<usize>>,
}

impl<'c, 'tu> Trace<'c, 'tu> {
    fn new(context: &'c Context<'tu>) -> Self {
        Self {
            context,
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
                ..
            } => {
                if *depth == 0 {
                    let (file, line, column) = self.place(expansion);
                    let (end_line, end_column) = self
                        .context
                        .physical_position(end.source_file_index, end.end())
                        .filter(|_| end.source_file_index == expansion.source_file_index)
                        .unwrap_or((line, column));
                    self.output.expansions.push(ExplorerExpansion {
                        file,
                        line,
                        column,
                        end_line,
                        end_column,
                        invocation: self.invocation(expansion, end, *name),
                        result: String::new(),
                        steps: Vec::new(),
                    });
                    self.keys.push((key(expansion), ResultEnd::default()));
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
                    macro_name: self.spell(*name).to_owned(),
                    depth: *depth,
                    arguments,
                    operands: Vec::new(),
                    result: String::new(),
                };
                self.step(expansion, step);
            },
            | TraceEvent::Stringified {
                expansion,
                macro_name,
                result,
            } => {
                let step = ExplorerExpansionStep {
                    kind:       "stringify".to_owned(),
                    macro_name: self.spell(*macro_name).to_owned(),
                    depth:      0,
                    arguments:  Vec::new(),
                    operands:   Vec::new(),
                    result:     self.spell(*result).to_owned(),
                };
                self.step(expansion, step);
            },
            | TraceEvent::Pasted {
                expansion,
                macro_name,
                left,
                right,
                result,
            } => {
                let step = ExplorerExpansionStep {
                    kind:       "paste".to_owned(),
                    macro_name: self.spell(*macro_name).to_owned(),
                    depth:      0,
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
            | TraceEvent::Condition { directive, taken } => {
                let (file, line, _) = self.place(directive);
                self.output.conditionals.push(ExplorerConditional {
                    file,
                    line,
                    directive: self.line_text(directive),
                    taken: *taken,
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
    fn step(&mut self, expansion: &SourceVector, step: ExplorerExpansionStep) {
        if let Some(index) = self.expansion(expansion) {
            self.output.expansions[index].steps.push(step);
        }
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
        let ResultEnd { end, last } = &mut self.keys[index].1;
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
        let text = (start.source_file_index == end.source_file_index)
            .then(|| self.context.source_text(start.source_file_index))
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

    /// The physical file name, line and column of `vector`.
    fn place(&self, vector: &SourceVector) -> (String, u32, u32) {
        let (line, column) = self
            .context
            .physical_position(vector.source_file_index, vector.index as usize)
            .unwrap_or((vector.line, vector.column));
        (self.file_name(vector.source_file_index), line, column)
    }

    fn file_name(&self, file: u32) -> String {
        let physical = self.context.physical_source_file(file);
        self.context.get_source_file(physical).display().to_string()
    }

    /// The physical line that holds `vector`, without its new-line.
    fn line_text(&self, vector: &SourceVector) -> String {
        let Some(text) = self.context.source_text(vector.source_file_index) else {
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
    end:  Option<(u32, usize, u32)>,
    /// The first and last bytes of its spelling.
    last: (Option<u8>, Option<u8>),
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
        Self { source, starts }
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
        (
            line,
            count(self.source.get(start..offset).unwrap_or_default()) + 1,
        )
    }
}

fn count(text: &str) -> u32 {
    u32::try_from(text.chars().count()).unwrap_or(u32::MAX)
}
