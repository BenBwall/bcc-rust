//! The compiler pipeline: a strip of stages from the logical lines of
//! translation phases 1-2 to machine code, an explanation of the selected
//! stage with the C99 rules it applies, and bcc's real output for it in a
//! read-only editor, compiled live in the browser at `-O0` or `-O2`. The
//! front-end stages add what they did to this program (`front_end.rs`).
//! Hovering a source line, or moving the cursor to it, highlights the output
//! lines that came from it (and the reverse; see `app/links.rs`). When an error stops bcc, the view says which stage stopped
//! and why, and the diagnostics list every message.

use leptos::prelude::*;

use super::front_end::{
    FrontEndCard,
    StageClauses,
};
use crate::{
    app::links::{
        Pointing,
        Side,
    },
    demos,
    editor::{
        CodeEditor,
        Decoration,
        Layer,
    },
    pipeline::{
        Compilation,
        CompileDiagnostic,
        Links,
        Stage,
        displayed,
        gutter,
        is_folded,
        links,
        optimizer_changes,
    },
};

/// Where the repository's source files are shown.
const SOURCE_URL: &str = "https://github.com/BenBwall/bcc-rust/blob/main/";

#[component]
#[expect(clippy::too_many_arguments, reason = "The page state the view reads.")]
#[expect(clippy::too_many_lines, reason = "One block per part of the view.")]
pub fn PipelineView(
    stage: RwSignal<Stage>,
    /// Whether to show `-O2` output rather than `-O0`.
    optimize: RwSignal<bool>,
    /// The source's compilation at the selected level, which places the
    /// hover links and lists the diagnostics and the optimizer's changes.
    compilation: Memo<Option<Compilation>>,
    /// The output on screen: `compilation`, or the last output while the
    /// next one compiles. The stage strip and stop box describe it.
    shown: Memo<Option<Compilation>>,
    /// Its compilation at the other level, for the optimizer comparison.
    other: Memo<Option<Compilation>>,
    /// The in-browser compiler's progress on `source` at the selected level.
    live: Signal<LiveCompile>,
    theme: Signal<&'static str>,
    /// The lines linked between the source and the output.
    pointing: Pointing,
    /// Whether the preprocessed text shows the headers' text, not a folded
    /// line for each.
    headers: RwSignal<bool>,
    filename: ReadSignal<String>,
    /// The source the compilation came from.
    source: Signal<String>,
    /// Show a source line, from a diagnostic.
    on_reveal: Callback<usize>,
    /// Leave the last stage for the abstract machine, to watch the program
    /// run.
    on_run: Callback<()>,
) -> impl IntoView {
    let output = Memo::new(move |_| {
        let stage = stage.get();
        let headers = headers.get();
        shown.with(|compilation| {
            let compilation = compilation.as_ref()?;
            displayed(stage, compilation, headers).map(|(text, _)| text)
        })
    });
    let text = Signal::derive(move || output.get().unwrap_or_default());
    let lesson = Memo::new(move |_| source.with(|source| demos::matching(source).is_some()));
    let language = Signal::derive(move || stage.get().language());
    let line_numbers = Signal::derive(move || {
        let stage = stage.get();
        let headers = headers.get();
        shown.with(|compilation| {
            compilation
                .as_ref()
                .map_or_else(Vec::new, |compilation| gutter(stage, compilation, headers))
        })
    });
    let links_of = move |compilation: Memo<Option<Compilation>>| {
        Memo::new(move |_| {
            compilation.with(|compilation| {
                compilation
                    .as_ref()
                    .map(|compilation| {
                        source.with(|source| links(stage.get(), compilation, source, headers.get()))
                    })
                    .unwrap_or_default()
            })
        })
    };
    let links = links_of(compilation);
    // The note on linking describes the output on screen, so it stays while
    // the next output compiles.
    let shown_links = links_of(shown);
    let layers = Signal::derive(move || -> Vec<Layer> {
        let hovered = pointing.line(Side::Source);
        let linked = if hovered > 0 {
            links.with(|links| links.outputs_for(hovered))
        } else {
            Vec::new()
        };
        vec![
            (
                "links",
                linked
                    .into_iter()
                    .map(|line| Decoration::line(line, "link-line"))
                    .collect(),
            ),
            ("stage", output_marks(stage.get(), shown, text)),
        ]
    });
    let reveal = Signal::derive(move || {
        let hovered = pointing.line(Side::Source);
        (hovered > 0)
            .then(|| links.with(|links| links.first_output_for(hovered)))
            .flatten()
    });
    // The whole-path lesson's note on what this stage did to it, which
    // describes its -O0 output.
    let path_note = move || {
        if optimize.get() {
            return None;
        }
        let index = Stage::ALL.iter().position(|&each| each == stage.get())?;
        source.with(|source| {
            demos::matching(source).and_then(|demo| demo.path.get(index).copied())
        })
    };
    let on_hover = Callback::new(move |line| pointing.hover(Side::Output, line));
    let on_cursor = Callback::new(move |line| pointing.cursor(Side::Output, line));
    view! {
        <section class="pipeline" aria-labelledby="pipeline-title">
            <h2 id="pipeline-title" class="sr-only">"Compiler pipeline"</h2>
            <StageStrip stage compilation=shown />
            <article class="card stage-card" id="stage-output" data-phase=move || stage.get().phase()>
                <header class="stage-header">
                    <div>
                        <p class="stage-kicker">
                            {move || stage.get().phase()}
                            {move || stage.get().translation_phases().map(|phases| view! {
                                <span class="stage-kicker-phases">{format!("Translation {phases}")}</span>
                            })}
                        </p>
                        <h3>{move || stage.get().label()}</h3>
                    </div>
                    <div class="stage-controls">
                        <LiveStatus live lesson stage />
                        <OptimizationToggle optimize />
                    </div>
                </header>
                {move || {
                    let stage = stage.get();
                    match (stage.command(&filename.get(), optimize.get()), stage.internal()) {
                        | (Some(command), _) => view! { <code class="stage-command">{format!("$ {command}")}</code> }.into_any(),
                        | (None, Some(path)) => view! {
                            <p class="stage-command is-internal">
                                "No command prints this stage: it happens inside bcc, in "
                                <a href=format!("{SOURCE_URL}{path}")><code>{path}</code></a>
                            </p>
                        }.into_any(),
                        | (None, None) => ().into_any(),
                    }
                }}
                <p class="stage-handoff">
                    <span>{move || stage.get().handoff()}</span>
                    {move || match stage.get().next() {
                        | Some(next) => view! {
                            <button type="button" class="stage-next" on:click=move |_| stage.set(next)>
                                "Next: "{next.label()}<span aria-hidden="true">" →"</span>
                            </button>
                        }.into_any(),
                        | None => view! {
                            <button type="button" class="stage-next" on:click=move |_| on_run.run(())>
                                "Next: watch it run in the abstract machine"<span aria-hidden="true">" →"</span>
                            </button>
                        }.into_any(),
                    }}
                </p>
                <p class="stage-path-note" hidden=move || path_note().is_none()>
                    <strong>"In this program: "</strong>
                    {move || path_note().map(super::rich_text)}
                </p>
                <p class="stage-explanation">{move || stage.get().explanation()}</p>
                <StageClauses stage />
                <StopBanner compilation=shown />
                <FrontEndCard stage compilation on_reveal />
                <OptimizerNote stage optimize compilation other shown=Signal::derive(move || output.with(Option::is_some)) />
                <div class="stage-toolbar">
                    <p class="stage-note is-link" hidden=move || shown_links.with(Links::is_empty)>
                        {move || if shown_links.with(Links::is_exact) {
                            "Hover a source line, or move the cursor to it, to find what it became here; do the \
                             same here to find its source."
                        } else {
                            "Hover a source line, or move the cursor to it, to find its function here; do the same \
                             here to find the source. These lines keep no source locations, so they link per \
                             function, not per line."
                        }}
                    </p>
                    <button
                        type="button"
                        class="toggle-chip"
                        hidden=move || stage.get() != Stage::Preprocessor || output.with(Option::is_none)
                        aria-pressed=move || headers.get().to_string()
                        title="Show the text each #include brought in, or fold it into one line"
                        on:click=move |_| headers.update(|headers| *headers = !*headers)
                    >
                        <span class="toggle-chip-dot" aria-hidden="true"></span>
                        "Show headers"
                    </button>
                </div>
                <div class="stage-output" hidden=move || output.with(Option::is_none)>
                    <CodeEditor
                        text
                        language
                        theme
                        label="Compiler output (read only)"
                        read_only=true
                        decorations=layers
                        on_hover_line=on_hover
                        on_cursor_line=on_cursor
                        reveal
                        line_numbers
                    />
                </div>
                <div class="stage-missing" hidden=move || output.with(Option::is_some)>
                    <p>
                        {move || shown.with(|compilation| {
                            missing(stage.get(), lesson.get(), compilation.as_ref(), &live.get())
                        })}
                    </p>
                    <div class="stage-missing-actions" hidden=move || stage.get() != Stage::Llvm || lesson.get()>
                        <button type="button" class="button small" on:click=move |_| show_ir(stage)>
                            "Show bcc IR, compiled live"
                        </button>
                        <code class="stage-command">{move || Stage::Llvm.command(&filename.get(), optimize.get()).map(|command| format!("$ {command}"))}</code>
                    </div>
                </div>
            </article>
            <Diagnostics compilation live on_reveal />
        </section>
    }
}

/// Switches to the bcc IR stage and moves the focus to its tab, since the
/// button that asked for it disappears with the empty state.
fn show_ir(stage: RwSignal<Stage>) {
    stage.set(Stage::Ir);
    #[cfg(feature = "hydrate")]
    if let Some(tab) = document()
        .get_element_by_id(&format!("stage-tab-{}", Stage::Ir.key()))
        .and_then(|tab| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(tab).ok())
    {
        _ = tab.focus();
    }
}

/// Which stage an error stopped bcc in, and why.
#[component]
fn StopBanner(compilation: Memo<Option<Compilation>>) -> impl IntoView {
    let stop = Memo::new(move |_| {
        compilation.with(|compilation| compilation.as_ref().and_then(Compilation::stop))
    });
    view! {
        <div class="stop-banner" role="status" hidden=move || stop.with(Option::is_none)>
            <p class="stop-title">
                <span class="stop-icon" aria-hidden="true">"✕"</span>
                {move || stop.with(|stop| stop.as_ref().map(|stop| format!("Compilation stopped in the {}", stop.phase.to_lowercase())))}
            </p>
            <p>{move || stop.with(|stop| stop.as_ref().map(|stop| stop.explanation.clone()))}</p>
        </div>
    }
}

/// The stages as tabs, in two rows: the six of the front end, then the five
/// of the middle and back ends, each row under the headings of its parts. Each
/// front-end stage names the C99 translation phases it shows. A stage an error
/// kept from running is marked.
#[component]
fn StageStrip(stage: RwSignal<Stage>, compilation: Memo<Option<Compilation>>) -> impl IntoView {
    let stopped = move |entry: Stage| {
        compilation.with(|compilation| {
            compilation
                .as_ref()
                .is_some_and(|compilation| compilation.stopped_before(entry))
        })
    };
    let item = move |(index, entry): (usize, Stage)| {
        view! {
            <div class="stage-item" role="presentation">
                <button
                    type="button"
                    role="tab"
                    class="stage-button"
                    class:is-stopped=move || stopped(entry)
                    data-phase=entry.phase()
                    aria-selected=move || (stage.get() == entry).to_string()
                    aria-controls="stage-output"
                    id=format!("stage-tab-{}", entry.key())
                    on:click=move |_| stage.set(entry)
                >
                    <span class="stage-index">{move || if stopped(entry) { "✕".to_owned() } else { (index + 1).to_string() }}</span>
                    <span class="stage-text">
                        <span class="stage-name">{entry.label()}</span>
                        {entry.translation_phases().map(|phases| view! { <span class="stage-phase">{phases}</span> })}
                    </span>
                    <span class="sr-only">
                        {move || format!(" ({}{})", entry.phase(), if stopped(entry) { ", did not run" } else { "" })}
                    </span>
                </button>
            </div>
        }
    };
    let row = move |range: std::ops::Range<usize>| {
        Stage::ALL[range.clone()]
            .iter()
            .copied()
            .enumerate()
            .map(move |(offset, entry)| (range.start + offset, entry))
            .map(item)
            .collect_view()
    };
    view! {
        <div class="stage-strip" role="tablist" aria-label="Compiler stages">
            <div class="stage-row" role="presentation">
                <span class="stage-group-label" data-phase="Front end" aria-hidden="true">"Front end"</span>
                {row(0..6)}
            </div>
            <div class="stage-row" role="presentation">
                <span class="stage-group-label" data-phase="Middle end" aria-hidden="true">"Middle end"</span>
                <span class="stage-group-label" data-phase="Back end" aria-hidden="true">"Back end"</span>
                {row(6..11)}
            </div>
        </div>
    }
}

/// The marks the output of `stage` carries: where phase 2 spliced each
/// logical line, and the folded headers of the preprocessed text.
fn output_marks(
    stage: Stage,
    compilation: Memo<Option<Compilation>>,
    text: Signal<String>,
) -> Vec<Decoration> {
    match stage {
        | Stage::Lines => compilation.with(|compilation| {
            compilation
                .iter()
                .flat_map(|compilation| compilation.logical_lines.iter().enumerate())
                .flat_map(|(index, line)| {
                    line.changes.iter().filter_map(move |change| {
                        let (before, class) = match change.kind.as_str() {
                            | "splice" => ("↩", "splice-marker"),
                            | "trigraph" => ("", "trigraph-marker"),
                            | _ => return None,
                        };
                        let end = if before.is_empty() {
                            change.column + 1
                        } else {
                            change.column
                        };
                        Some(Decoration {
                            start_line: index + 1,
                            start_column: change.column,
                            end_line: index + 1,
                            end_column: end,
                            inline_class: before.is_empty().then(|| class.to_owned()),
                            before: (!before.is_empty()).then(|| before.to_owned()),
                            before_class: (!before.is_empty()).then(|| class.to_owned()),
                            hover: Some(change.describe()),
                            ..Decoration::default()
                        })
                    })
                })
                .collect()
        }),
        | Stage::Preprocessor => text.with(|text| {
            text.lines()
                .enumerate()
                .filter(|(_, line)| is_folded(line))
                .map(|(index, _)| Decoration::line(index + 1, "folded-line"))
                .collect()
        }),
        | _ => Vec::new(),
    }
}

/// Where the selected stage's output comes from: compiled in this browser, or
/// (LLVM IR) precomputed by the native compiler.
#[component]
fn LiveStatus(
    live: Signal<LiveCompile>,
    lesson: Memo<bool>,
    stage: RwSignal<Stage>,
) -> impl IntoView {
    let status = move || {
        if stage.get() == Stage::Llvm {
            return if lesson.get() {
                ("precomputed", "Precomputed by native bcc")
            } else {
                ("idle", "Bundled lessons only")
            };
        }
        match live.get() {
            | LiveCompile::Loading => ("busy", "Loading the compiler…"),
            | LiveCompile::Compiling => ("busy", "Compiling…"),
            | LiveCompile::Failed(error) if error.starts_with(crate::pipeline::TOO_LARGE) =>
                ("failed", "Too large to compile here"),
            | LiveCompile::Failed(_) => ("failed", "Compiler stopped"),
            | LiveCompile::Compiled => ("live", "Compiled live in your browser"),
            | LiveCompile::Idle => ("idle", "Not compiled yet"),
        }
    };
    view! {
        <span class="live-status" data-status=move || status().0 aria-live="polite">
            <i aria-hidden="true"></i>
            {move || status().1}
        </span>
    }
}

/// The `-O0` / `-O2` switch.
#[component]
fn OptimizationToggle(optimize: RwSignal<bool>) -> impl IntoView {
    let option = move |level: bool, label: &'static str, title: &'static str| {
        view! {
            <button
                type="button"
                class="opt-option"
                title=title
                aria-pressed=move || (optimize.get() == level).to_string()
                on:click=move |_| optimize.set(level)
            >
                {label}
            </button>
        }
    };
    view! {
        <div class="opt-toggle" role="group" aria-label="Optimization level">
            {option(false, "-O0", "No optimization: each operation translated as written")}
            {option(true, "-O2", "Optimize: run bcc's optimizer before code generation")}
        </div>
    }
}

/// What the optimizer does at this stage and, once both levels are compiled,
/// what it changed in this program.
#[component]
fn OptimizerNote(
    stage: RwSignal<Stage>,
    optimize: RwSignal<bool>,
    compilation: Memo<Option<Compilation>>,
    other: Memo<Option<Compilation>>,
    /// Whether the stage has output to show; the note is about that output.
    shown: Signal<bool>,
) -> impl IntoView {
    let changes = Memo::new(move |_| {
        let stage = stage.get();
        compilation.with(|current| {
            other.with(|other| match (current, other) {
                | (Some(current), Some(other)) if optimize.get() =>
                    optimizer_changes(stage, other, current),
                | (Some(current), Some(other)) => optimizer_changes(stage, current, other),
                | _ => Vec::new(),
            })
        })
    });
    let summary = move || {
        if optimize.get() {
            stage.get().optimizer_note().to_owned()
        } else {
            "This is -O0: bcc translates each operation as written, without optimizing. Switch to \
             -O2 to watch its optimizer rewrite the program."
                .to_owned()
        }
    };
    view! {
        <aside class="optimizer-note" data-level=move || if optimize.get() { "O2" } else { "O0" } hidden=move || !shown.get()>
            <p>{summary}</p>
            <ul hidden=move || changes.with(Vec::is_empty)>
                {move || changes.get().into_iter().map(|change| view! { <li>{change}</li> }).collect_view()}
            </ul>
        </aside>
    }
}

/// bcc's errors, warnings and notes for the program, each placed in the
/// source.
#[component]
fn Diagnostics(
    compilation: Memo<Option<Compilation>>,
    live: Signal<LiveCompile>,
    on_reveal: Callback<usize>,
) -> impl IntoView {
    let diagnostics = Memo::new(move |_| {
        compilation.with(|compilation| {
            compilation
                .as_ref()
                .filter(|compilation| {
                    compilation.tokens.is_some() || !compilation.diagnostics.is_empty()
                })
                .map(|compilation| compilation.diagnostics.clone())
        })
    });
    view! {
            <article class="card diagnostics-card" aria-labelledby="diagnostics-title">
                <h3 id="diagnostics-title">"Diagnostics"</h3>
                {move || match diagnostics.get() {
                    | Some(list) if list.is_empty() => view! {
                        <p class="diagnostics-clean">
                            <span class="check" aria-hidden="true">"✓"</span>
                            "bcc compiled this program without warnings. Undefined behavior is usually not \
                             detectable at compile time: the lessons that stop with UB compile cleanly."
                        </p>
                    }.into_any(),
                    | Some(list) => view! {
                        <ol class="diagnostics-list">
                            {list.into_iter().map(|diagnostic| view! { <DiagnosticItem diagnostic on_reveal /> }).collect_view()}
                        </ol>
                    }.into_any(),
                    | None if live.get() == LiveCompile::Compiling || live.get() == LiveCompile::Loading =>
                        view! { <p class="muted">"Compiling…"</p> }.into_any(),
                    | None => view! { <p class="muted">"No compilation for this source yet."</p> }.into_any(),
                }}
            </article>
    }
}

#[component]
fn DiagnosticItem(diagnostic: CompileDiagnostic, on_reveal: Callback<usize>) -> impl IntoView {
    let line = diagnostic.range.map(|range| range.line);
    let place = diagnostic.range.map_or_else(
        || "no source location".to_owned(),
        |range| format!("line {}, column {}", range.line, range.column),
    );
    let stage = match diagnostic.stage.as_str() {
        | "middle-end" => "middle end",
        | "back-end" => "back end",
        | _ => "front end",
    };
    view! {
        <li class="diagnostic" data-severity=diagnostic.severity.clone()>
            <div class="diagnostic-head">
                <span class="diagnostic-severity">{diagnostic.severity.clone()}</span>
                <button
                    type="button"
                    class="diagnostic-place"
                    disabled=line.is_none()
                    title="Show this line in the source"
                    on:click=move |_| if let Some(line) = line { on_reveal.run(line) }
                >
                    {place}
                </button>
                <span class="diagnostic-stage">{stage}</span>
            </div>
            <p class="diagnostic-message">{diagnostic.message.clone()}</p>
            <details>
                <summary>"Full message"</summary>
                <pre>{diagnostic.rendered.clone()}</pre>
            </details>
        </li>
    }
}

/// What the page knows about compiling the editor's text in the browser at
/// the selected level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveCompile {
    /// No compilation of this text has been requested.
    Idle,
    /// The compiler Worker is starting.
    Loading,
    Compiling,
    Compiled,
    /// The compiler Worker could not compile this text: it failed to start,
    /// ran too long, or crashed.
    Failed(String),
}

/// Why `stage` has no output to show for a bundled lesson (`lesson`) or other
/// source, given its compilation, if any, and the compiler's progress.
fn missing(
    stage: Stage,
    lesson: bool,
    compilation: Option<&Compilation>,
    live: &LiveCompile,
) -> String {
    if stage == Stage::Llvm && !lesson {
        return "LLVM IR comes from bcc's native build, which links LLVM. The copy of bcc running \
                in your browser has no LLVM, so this stage is shown for the bundled lessons \
                only; every other stage is compiled live as you type. bcc IR, two stages back, \
                has the same shape: values in SSA form and explicit basic blocks. For this \
                program's LLVM IR, run bcc on your machine:"
            .to_owned();
    }
    match live {
        | LiveCompile::Loading =>
            return "Loading bcc's compiler into your browser. It is a separate download, so the \
                    abstract machine never waits for it."
                .to_owned(),
        | LiveCompile::Compiling => return "Compiling this program in your browser…".to_owned(),
        // Refused before compiling: nothing stopped.
        | LiveCompile::Failed(error) if error.starts_with(crate::pipeline::TOO_LARGE) =>
            return error.clone(),
        | LiveCompile::Failed(error) =>
            return format!("{error} The page starts a fresh compiler for your next edit."),
        | LiveCompile::Compiled | LiveCompile::Idle => {},
    }
    match compilation {
        | Some(compilation) if compilation.stopped_before(stage) => format!(
            "This stage did not run: bcc stopped in the {}. The red box above explains why.",
            compilation
                .stop()
                .map_or("front end", |stop| stop.phase)
                .to_lowercase()
        ),
        | Some(compilation) if compilation.tokens.is_some() => {
            let notes: Vec<&str> = compilation
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == "note")
                .map(|diagnostic| diagnostic.message.as_str())
                .collect();
            if notes.is_empty() {
                "bcc produced no output for this stage.".to_owned()
            } else {
                format!("bcc produced no output for this stage: {}", notes.join(" "))
            }
        },
        | _ => "This version of the program has not been compiled yet.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LiveCompile,
        Stage,
        missing,
    };
    use crate::pipeline::{
        Compilation,
        CompileDiagnostic,
    };

    #[test]
    fn missing_output_explains_the_compilers_progress() {
        let failed = missing(
            Stage::Ir,
            false,
            None,
            &LiveCompile::Failed("The compiler stopped.".to_owned()),
        );
        assert!(failed.starts_with("The compiler stopped."), "{failed}");
        assert!(failed.contains("fresh compiler"), "{failed}");
        assert!(missing(Stage::Ir, true, None, &LiveCompile::Compiling).starts_with("Compiling"));
        assert!(missing(Stage::Ir, false, None, &LiveCompile::Loading).contains("Loading"));
        // Edited source has no LLVM IR, compiled or not.
        assert!(missing(Stage::Llvm, false, None, &LiveCompile::Compiled).contains("no LLVM"));
        // A program the page refused was never compiled, so nothing stopped.
        let refusal = crate::pipeline::too_large(&"x".repeat(70 * 1024)).unwrap();
        let refused = missing(
            Stage::Ir,
            false,
            None,
            &LiveCompile::Failed(refusal.clone()),
        );
        assert_eq!(refused, refusal);
        let memory = missing(
            Stage::Ir,
            false,
            None,
            &LiveCompile::Failed(crate::engine::OUT_OF_MEMORY.to_owned()),
        );
        assert!(memory.starts_with("bcc ran out of memory"), "{memory}");
        assert!(memory.contains("fresh compiler"), "{memory}");
    }

    #[test]
    fn a_stage_an_error_skipped_points_at_the_stop() {
        let stopped = Compilation {
            tokens: Some("<input>:1:1: identifier `x`".into()),
            diagnostics: vec![CompileDiagnostic {
                severity: "error".into(),
                stage: "front-end".into(),
                message: "expected ';'".into(),
                ..CompileDiagnostic::default()
            }],
            ..Compilation::default()
        };
        let text = missing(Stage::Asm, false, Some(&stopped), &LiveCompile::Compiled);
        assert!(text.contains("stopped in the front end"), "{text}");
        let clean = Compilation {
            tokens: Some("<input>:1:1: identifier `x`".into()),
            ..Compilation::default()
        };
        assert_eq!(
            missing(Stage::Asm, false, Some(&clean), &LiveCompile::Compiled),
            "bcc produced no output for this stage."
        );
    }

    /// The pipeline view at `-O2` (when `optimize`), rendered to HTML, given
    /// the source's compilation at that level, the output on screen, and the
    /// source's compilation at the other level.
    #[cfg(feature = "ssr")]
    fn render(
        optimize: bool,
        current: Option<Compilation>,
        held: Option<Compilation>,
        other: Option<Compilation>,
    ) -> String {
        use leptos::prelude::*;

        use super::{
            PipelineView,
            Pointing,
        };

        Owner::new().with(|| {
            let compilation = Memo::new(move |_| current.clone());
            let shown = Memo::new(move |_| held.clone());
            let other = Memo::new(move |_| other.clone());
            view! {
                <PipelineView
                    stage=RwSignal::new(Stage::Ir)
                    optimize=RwSignal::new(optimize)
                    compilation
                    shown
                    other
                    live=Signal::stored(LiveCompile::Compiling)
                    theme=Signal::stored("dark")
                    pointing=Pointing::new()
                    headers=RwSignal::new(false)
                    filename=RwSignal::new("program.c".to_owned()).read_only()
                    source=Signal::stored("int main(void) { return 0; }\n".to_owned())
                    on_reveal=Callback::new(|_: usize| {})
                    on_run=Callback::new(|()| {})
                />
            }
            .to_html()
        })
    }

    /// An `-O0` compilation of an earlier text with one placed error.
    #[cfg(feature = "ssr")]
    fn earlier() -> Compilation {
        Compilation {
            tokens: Some(String::new()),
            ir: Some("function @earlier_output() -> i32 external {\n".into()),
            diagnostics: vec![CompileDiagnostic {
                severity: "warning".into(),
                stage: "front-end".into(),
                message: "an earlier warning".into(),
                range: Some(crate::pipeline::SourceSpan {
                    line:       3,
                    column:     1,
                    end_line:   3,
                    end_column: 2,
                }),
                ..CompileDiagnostic::default()
            }],
            ..Compilation::default()
        }
    }

    #[test]
    #[cfg(feature = "ssr")]
    fn a_held_output_lists_none_of_its_diagnostics() {
        let held = render(false, None, Some(earlier()), None);
        assert!(held.contains("earlier_output"), "the output stays: {held}");
        assert!(!held.contains("an earlier warning"), "{held}");
        assert!(!held.contains("diagnostic-place"), "{held}");
        let current = render(false, Some(earlier()), Some(earlier()), None);
        assert!(current.contains("an earlier warning"), "{current}");
        assert!(current.contains("diagnostic-place"), "{current}");
    }

    #[test]
    #[cfg(feature = "ssr")]
    fn the_optimizer_note_waits_for_both_levels() {
        const SAME: &str = "nothing to improve";
        // Switched to an uncached -O2 while -O0 is still on screen.
        let switched = render(true, None, Some(earlier()), Some(earlier()));
        assert!(!switched.contains(SAME), "{switched}");
        // Both levels compiled to the same IR.
        let compiled = render(true, Some(earlier()), Some(earlier()), Some(earlier()));
        assert!(compiled.contains(SAME), "{compiled}");
    }
}
