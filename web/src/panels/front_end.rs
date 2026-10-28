//! What the front-end stages add to the pipeline view: the C99 rules each
//! one applies, a card on what it did to this program (what phases 1 and 2
//! changed, what phase 3 made, what phase 4 expanded, kept, skipped and
//! included), and the marks each one puts on the source.

use leptos::prelude::*;

use crate::{
    editor::Decoration,
    pipeline::{
        Citation,
        Compilation,
        Expansion,
        Include,
        SourceSpan,
        Stage,
    },
};

/// The rules of C99 the stage applies, as the abstract machine's narration
/// lists them.
#[component]
pub fn StageClauses(stage: RwSignal<Stage>) -> impl IntoView {
    view! {
        <ul class="clauses stage-clauses" aria-label="C standard rules applied" hidden=move || stage.get().clauses().is_empty()>
            {move || stage.get().clauses().iter().map(|clause: &Citation| view! {
                <li class="clause">
                    <span class="clause-id" title=clause.citation>{format!("C99 §{}", clause.id)}</span>
                    <span class="clause-gloss"><strong class="clause-title">{clause.title}.</strong>" "{clause.gloss}</span>
                </li>
            }).collect_view()}
        </ul>
    }
}

/// What the stage did to this program, for the stages before tokens.
#[component]
pub fn FrontEndCard(
    stage: RwSignal<Stage>,
    compilation: Memo<Option<Compilation>>,
    on_reveal: Callback<usize>,
) -> impl IntoView {
    move || {
        let stage = stage.get();
        compilation.with(|compilation| {
            let compilation = compilation.as_ref()?;
            compilation.output(stage)?;
            Some(match stage {
                | Stage::Lines => lines_card(compilation, on_reveal),
                | Stage::PpTokens => tokens_card(compilation),
                | Stage::Preprocessor => preprocessor_card(compilation, on_reveal),
                | _ => return None,
            })
        })
    }
}

/// A button that shows `line` in the source.
fn place(line: usize, on_reveal: Callback<usize>) -> impl IntoView {
    view! {
        <button type="button" class="phase-place" title="Show this line in the source" on:click=move |_| on_reveal.run(line)>
            {format!("line {line}")}
        </button>
    }
}

fn lines_card(compilation: &Compilation, on_reveal: Callback<usize>) -> AnyView {
    let physical = compilation
        .logical_lines
        .last()
        .map_or(0, |line| line.source.1);
    let logical = compilation.logical_lines.len();
    // One item per spliced logical line, then each other change.
    let mut items: Vec<(&'static str, usize, String)> = Vec::new();
    let mut splices = 0;
    for line in &compilation.logical_lines {
        let spliced = line
            .changes
            .iter()
            .filter(|change| change.kind == "splice")
            .count();
        splices += spliced;
        if spliced > 0 {
            let (first, last) = line.source;
            let text = if spliced == 1 {
                format!(
                    "Line {first} ends in a backslash, so phase 2 deleted it with the new-line \
                     and line {last} continues it."
                )
            } else {
                format!(
                    "Lines {first} to {} each end in a backslash, so phase 2 deleted them with \
                     their new-lines and lines {first}–{last} became one logical line.",
                    last - 1
                )
            };
            items.push(("splice", first, text));
        }
        for change in line.changes.iter().filter(|change| change.kind != "splice") {
            items.push((
                change_label(&change.kind),
                change.source.line,
                change.describe(),
            ));
        }
    }
    let summary = if splices == 0 {
        format!(
            "No line ends in a backslash, so each of the {logical} logical lines is one physical \
             line."
        )
    } else {
        format!(
            "{splices} line {} joined {physical} physical lines into {logical} logical lines.",
            if splices == 1 { "splice" } else { "splices" }
        )
    };
    view! {
        <section class="card-inset phase-card" aria-label="What phases 1 and 2 changed">
            <h4>"What phases 1 and 2 changed"</h4>
            <p class="phase-summary">{summary}</p>
            {(!items.is_empty()).then(|| view! {
                <ol class="phase-list">
                    {items.into_iter().map(|(label, line, text)| view! {
                        <li class="phase-item" data-kind=label>
                            <span class="phase-badge">{label}</span>
                            {place(line, on_reveal)}
                            <span class="phase-text">{text}</span>
                        </li>
                    }).collect_view()}
                </ol>
            })}
        </section>
    }
    .into_any()
}

fn change_label(kind: &str) -> &'static str {
    match kind {
        | "splice" => "splice",
        | "trigraph" => "trigraph",
        | "crlf" => "crlf",
        | _ => "UTF-8",
    }
}

fn tokens_card(compilation: &Compilation) -> AnyView {
    let counts = compilation.pp_counts.clone();
    let comments = compilation.pp_comments.len();
    let tokens: usize = counts
        .iter()
        .filter(|(kind, _)| !matches!(kind.as_str(), "whitespace" | "newline" | "comment"))
        .map(|(_, count)| count)
        .sum();
    let summary = format!(
        "{tokens} preprocessing tokens. {}",
        match comments {
            | 0 => "There are no comments.".to_owned(),
            | 1 => "The one comment became a single space.".to_owned(),
            | _ => format!("Each of the {comments} comments became a single space."),
        }
    );
    view! {
        <section class="card-inset phase-card" aria-label="What phase 3 made">
            <h4>"What phase 3 made"</h4>
            <p class="phase-summary">{summary}</p>
            <ul class="kind-counts">
                {counts.into_iter().map(|(kind, count)| {
                    let name = kind.clone();
                    view! {
                        <li class="kind-count" data-kind=kind>
                            <span class="kind-swatch" aria-hidden="true"></span>
                            <span class="kind-name">{name}</span>
                            <span class="kind-number">{count}</span>
                        </li>
                    }
                }).collect_view()}
            </ul>
        </section>
    }
    .into_any()
}

fn preprocessor_card(compilation: &Compilation, on_reveal: Callback<usize>) -> AnyView {
    let trace = compilation.preprocessing.clone();
    let quiet = trace.expansions.is_empty() && trace.directives.is_empty();
    // Directives the other lists do not cover, such as #pragma and #line.
    let others: Vec<_> = trace
        .directives
        .iter()
        .filter(|directive| {
            !matches!(
                directive.name.as_str(),
                "define"
                    | "undef"
                    | "include"
                    | "include_next"
                    | "if"
                    | "ifdef"
                    | "ifndef"
                    | "elif"
                    | "elifdef"
                    | "elifndef"
                    | "else"
                    | "endif"
            )
        })
        .cloned()
        .collect();
    let omitted = trace.omitted_expansions;
    let defined: Vec<String> = trace
        .definitions
        .iter()
        .map(|definition| match definition.kind.as_str() {
            | "undef" => format!("#undef {}", definition.name),
            | "function" => {
                // The name and its parameters, as the directive spells them.
                let directive = &definition.directive;
                let start = directive.find(&*definition.name).unwrap_or(0);
                let end = directive[start..]
                    .find(')')
                    .map_or(directive.len(), |end| start + end + 1);
                directive[start..end].to_owned()
            },
            | _ => definition.name.clone(),
        })
        .collect();
    view! {
        <section class="card-inset phase-card" aria-label="What the preprocessor did">
            <h4>"What the preprocessor did"</h4>
            {quiet.then(|| view! {
                <p class="phase-summary">
                    "This program has no directives and uses no macros, so the preprocessor passes its tokens on unchanged."
                </p>
            })}
            {(!others.is_empty()).then(|| view! {
                <h5>"Other directives"</h5>
                <ol class="phase-list">
                    {others.into_iter().map(|directive| view! {
                        <li class="phase-item" data-kind="directive">
                            <span class="phase-badge">{format!("#{}", directive.name)}</span>
                            {place(directive.line, on_reveal)}
                            <span class="phase-text">
                                <code>{directive.directive.clone()}</code>
                                {format!(" {}", directive_meaning(&directive.name))}
                                {presumed(directive.presumed.as_deref())}
                            </span>
                        </li>
                    }).collect_view()}
                </ol>
            })}
            {(!trace.includes.is_empty()).then(|| view! {
                <h5>"Headers"</h5>
                <ol class="phase-list">
                    {trace.includes.iter().map(|include| include_item(include, on_reveal)).collect_view()}
                </ol>
            })}
            {(!defined.is_empty()).then(|| view! {
                <h5>"Macros defined"</h5>
                <p class="macro-chips">
                    {defined.into_iter().map(|name| view! { <code class="macro-chip">{name}</code> }).collect_view()}
                </p>
            })}
            {(!trace.expansions.is_empty()).then(|| view! {
                <h5>{format!("Macro expansions ({})", trace.expansions.len())}</h5>
                <ol class="expansion-list">
                    {trace.expansions.iter().map(|expansion| expansion_item(expansion, on_reveal)).collect_view()}
                </ol>
                {(omitted > 0).then(|| view! {
                    <p class="phase-summary">{format!("… and {omitted} more expansions, not listed here.")}</p>
                })}
            })}
            {(!trace.conditionals.is_empty()).then(|| {
                let skipped = trace.skipped.clone();
                view! {
                    <h5>"Conditional inclusion"</h5>
                    <ol class="phase-list">
                        {trace.conditionals.iter().map(|conditional| {
                            let line = conditional.line;
                            // The group this directive opens is the lines
                            // after it, up to the next directive.
                            let dropped = skipped.iter().find(|(first, _)| *first == line + 1).copied();
                            let lines = match dropped {
                                | Some((first, last)) if first == last => format!("line {first} is skipped"),
                                | Some((first, last)) => format!("lines {first}–{last} are skipped"),
                                | None => "its group is skipped".to_owned(),
                            };
                            let outcome = match (conditional.taken, conditional.evaluated) {
                                | (true, _) => "its group is kept".to_owned(),
                                | (false, true) => lines,
                                | (false, false) => format!("not evaluated, since an earlier group was kept: {lines}"),
                            };
                            view! {
                                <li class="phase-item" data-kind=if conditional.taken { "kept" } else { "skipped" }>
                                    <span class="phase-badge">{if conditional.taken { "kept" } else { "skipped" }}</span>
                                    {place(line, on_reveal)}
                                    <span class="phase-text">
                                        <code>{conditional.directive.clone()}</code>
                                        {format!(": {outcome}.")}
                                        {presumed(conditional.presumed.as_deref())}
                                    </span>
                                </li>
                            }
                        }).collect_view()}
                    </ol>
                }
            })}
        </section>
    }
    .into_any()
}

/// What a directive the other lists do not cover does, in a few words.
fn directive_meaning(name: &str) -> &'static str {
    match name {
        | "pragma" => "gives bcc an implementation-defined instruction (C99 §6.10.6p1).",
        | "line" => "renumbers the lines after it, and may rename the file (C99 §6.10.4p3).",
        | "error" => "stops translation with a message (C99 §6.10.5p1).",
        | "warning" => "prints a warning, as GCC and C23 allow.",
        | "embed" => "puts a file's bytes in its place as integer constants (C23).",
        | _ => "is executed and deleted, as every directive is (C99 §6.10p2).",
    }
}

/// Where `#line` says a place is, when it differs from the physical one.
fn presumed(place: Option<&str>) -> Option<impl IntoView + use<>> {
    let place = place?.to_owned();
    Some(view! {
        <span class="presumed" title="The place a #line directive gives it">{format!(" (#line: {place})")}</span>
    })
}

fn include_item(include: &Include, on_reveal: Callback<usize>) -> impl IntoView {
    let name = if include.angled {
        format!("<{}>", include.header)
    } else {
        format!("\"{}\"", include.header)
    };
    let text = if include.found {
        format!(
            "is one of bcc's own headers, {} lines long. The preprocessor ran phases 1 to 4 on it \
             in place of the #include; the view folds what it left.",
            include.lines
        )
    } else {
        "was not found. In the browser bcc has only its own freestanding headers, such as \
         <limits.h>, <stdint.h>, <stddef.h> and <stdbool.h>, and no C library headers like \
         <stdio.h>."
            .to_owned()
    };
    view! {
        <li class="phase-item" data-kind=if include.found { "kept" } else { "skipped" }>
            <span class="phase-badge">"#include"</span>
            {place(include.line, on_reveal)}
            <span class="phase-text"><code>{name}</code>{format!(" {text}")}</span>
        </li>
    }
}

fn expansion_item(expansion: &Expansion, on_reveal: Callback<usize>) -> impl IntoView {
    let line = expansion.range.line;
    let result = match (expansion.result.is_empty(), expansion.directive) {
        | (false, _) => expansion.result.clone(),
        | (true, true) => "nothing in the output: the directive on this line read it".to_owned(),
        | (true, false) => "nothing: its replacement list is empty".to_owned(),
    };
    let steps = expansion.steps.clone();
    let detailed = steps.len() > 1
        || steps
            .first()
            .is_some_and(|step| !step.arguments.is_empty() || step.kind != "replace");
    view! {
        <li class="expansion">
            <div class="expansion-head">
                {place(line, on_reveal)}
                <code class="expansion-from">{expansion.invocation.clone()}</code>
                <span class="expansion-arrow" aria-label="becomes">"→"</span>
                <code class="expansion-to" class:is-empty=expansion.result.is_empty()>{result}</code>
                {presumed(expansion.presumed.as_deref())}
            </div>
            {detailed.then(|| view! {
                <ol class="expansion-steps">
                    {steps.into_iter().map(|step| {
                        let depth = step.depth.min(4).to_string();
                        view! {
                            <li class="expansion-step" data-kind=step.kind.clone() data-depth=depth>
                                <span class="step-kind">{match step.kind.as_str() {
                                    | "stringify" => "#",
                                    | "paste" => "##",
                                    | "builtin" => "predefined",
                                    | _ if step.prescan => "prescan",
                                    | _ => "replace",
                                }}</span>
                                <span class="step-text">{step.describe()}</span>
                            </li>
                        }
                    }).collect_view()}
                </ol>
            })}
        </li>
    }
}

/// The marks `stage` puts on the source: the splices, trigraphs and UTF-8
/// characters of phases 1-2, the comments phase 3 replaced, and for phase
/// 4 each macro invocation (its expansion on hover), each skipped line, and
/// each conditional directive's outcome.
pub fn source_marks(stage: Stage, compilation: &Compilation) -> Vec<Decoration> {
    let span = |range: &SourceSpan, class: &str, hover: String| Decoration {
        start_line: range.line,
        start_column: range.column,
        end_line: range.end_line,
        end_column: range.end_column,
        inline_class: Some(class.to_owned()),
        hover: Some(hover),
        ..Decoration::default()
    };
    match stage {
        | Stage::Lines => compilation
            .logical_lines
            .iter()
            .flat_map(|line| &line.changes)
            .map(|change| {
                let class = match change.kind.as_str() {
                    | "splice" => "src-splice",
                    | "trigraph" => "src-trigraph",
                    | "crlf" => "src-crlf",
                    | _ => "src-extended",
                };
                span(&change.source, class, change.describe())
            })
            .filter(|mark| mark.inline_class.as_deref() != Some("src-crlf"))
            .collect(),
        | Stage::PpTokens => compilation
            .pp_comments
            .iter()
            .map(|range| {
                span(
                    range,
                    "src-comment-space",
                    "Phase 3 replaces this comment, and the white space around it, by one space."
                        .to_owned(),
                )
            })
            .collect(),
        | Stage::Preprocessor => {
            let trace = &compilation.preprocessing;
            let mut marks: Vec<Decoration> = trace
                .expansions
                .iter()
                .map(|expansion| {
                    let hover = match (expansion.result.is_empty(), expansion.directive) {
                        | (false, _) =>
                            format!("`{}` becomes `{}`", expansion.invocation, expansion.result),
                        | (true, true) => format!(
                            "`{}` is expanded for the directive on this line.",
                            expansion.invocation
                        ),
                        | (true, false) => format!(
                            "`{}` becomes nothing: its replacement list is empty.",
                            expansion.invocation
                        ),
                    };
                    span(&expansion.range, "src-macro", hover)
                })
                .collect();
            for &(first, last) in &trace.skipped {
                for line in first..=last {
                    marks.push(Decoration::line(line, "src-skipped"));
                }
            }
            for conditional in &trace.conditionals {
                marks.push(Decoration {
                    glyph_class: Some(
                        if conditional.taken {
                            "pp-glyph-kept"
                        } else {
                            "pp-glyph-skipped"
                        }
                        .to_owned(),
                    ),
                    hover: Some(format!(
                        "`{}`: the group after it is {}.",
                        conditional.directive,
                        if conditional.taken { "kept" } else { "skipped" }
                    )),
                    ..Decoration::line(conditional.line, "src-conditional")
                });
            }
            marks
        },
        | _ => Vec::new(),
    }
}
