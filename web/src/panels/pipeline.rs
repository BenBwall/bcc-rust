//! The compiler pipeline: a strip of stages from the logical lines of
//! translation phases 1-2 to machine code, an explanation of the selected
//! stage with the C99 rules it applies, and bcc's real output for it in a
//! read-only editor, compiled live in the browser at `-O0` or `-O2`. The
//! front-end stages add what they did to this program (`front_end.rs`).
//! Hovering a source line, or moving the cursor to it, highlights the output
//! lines that came from it (and the reverse; see `app/links.rs`). When an error
//! stops bcc, the view says which stage stopped and why, and the diagnostics
//! list every message.

use leptos::prelude::*;

use super::{
    front_end::{
        FrontEndCard,
        StageClauses,
    },
    notation::StageNotation,
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
        Brief,
        Compilation,
        CompileDiagnostic,
        Links,
        Place,
        Stage,
        Unfolded,
        displayed,
        frames,
        gutter,
        is_folded,
        linked_terms,
        optimizer_changes,
        optimizer_measures,
        target,
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
    /// The links between the source and `compilation`'s output.
    links: Memo<Links>,
    /// What the line the reader points at links to, in a sentence.
    reading: Signal<Option<String>>,
    /// The longest sentences `reading` can be in this view, which its box
    /// keeps room for.
    reading_room: Signal<Vec<String>>,
    /// Its compilation at the other level, for the optimizer comparison.
    other: Memo<Option<Compilation>>,
    /// Whether `-O0` and `-O2` are shown side by side.
    compare: RwSignal<bool>,
    /// The links between the source and `other`'s output, while they are.
    other_links: Memo<Links>,
    /// The in-browser compiler's progress on `source` at the selected level.
    live: Signal<LiveCompile>,
    theme: Signal<&'static str>,
    /// The lines linked between the source and the output.
    pointing: Pointing,
    /// Whether the preprocessed text shows the headers' text, not a folded
    /// line for each, and the IRs their target lines, not one line for them.
    unfolded: RwSignal<Unfolded>,
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
        let unfolded = unfolded.get();
        shown.with(|compilation| {
            let compilation = compilation.as_ref()?;
            displayed(stage, compilation, unfolded).map(|(text, _)| text)
        })
    });
    let text = Signal::derive(move || output.get().unwrap_or_default());
    let lesson = Memo::new(move |_| source.with(|source| demos::matching(source).is_some()));
    let language = Signal::derive(move || stage.get().language());
    let line_numbers = Signal::derive(move || {
        let stage = stage.get();
        let unfolded = unfolded.get();
        shown.with(|compilation| {
            compilation
                .as_ref()
                .map_or_else(Vec::new, |compilation| gutter(stage, compilation, unfolded))
        })
    });
    let links_of = move |compilation: Memo<Option<Compilation>>| {
        Memo::new(move |_| {
            compilation.with(|compilation| {
                compilation.as_ref().map_or_default(|compilation| {
                    source.with(|source| {
                        crate::pipeline::links(stage.get(), compilation, source, unfolded.get())
                    })
                })
            })
        })
    };
    // The note on linking describes the output on screen, so it stays while
    // the next output compiles.
    let shown_links = links_of(shown);
    let layers = Signal::derive(move || -> Vec<Layer> {
        let hovered = pointing.line(Side::Source);
        let linked = if hovered > 0 {
            links.with(|links| {
                links
                    .outputs_for(hovered)
                    .into_iter()
                    .map(|line| {
                        // A copy inlined from elsewhere reads differently from
                        // what this line became in place.
                        let class = if links.origin(line).call == 0 {
                            "link-line"
                        } else {
                            "link-inlined"
                        };
                        Decoration::line(line, class)
                    })
                    .collect()
            })
        } else {
            Vec::new()
        };
        vec![
            ("links", linked),
            ("stage", output_marks(stage.get(), shown, text)),
            (
                "unclaimed",
                shown_links.with(|links| {
                    links
                        .unclaimed()
                        .map(|line| Decoration {
                            start_line: line,
                            end_line: line,
                            margin_class: Some("link-unclaimed".to_owned()),
                            ..Decoration::default()
                        })
                        .collect()
                }),
            ),
        ]
    });
    let reveal = Signal::derive(move || {
        let hovered = pointing.line(Side::Source);
        (hovered > 0)
            .then(|| links.with(|links| links.run_for(hovered)))
            .flatten()
    });
    // Where the reader is in the code, which the outputs keep as the level,
    // the stage or the program changes under them.
    let place = StoredValue::new(None::<Place>);
    let keeper = PlaceKeeper::new(place, shown_links, shown);
    // Whether the stage's output opens with target lines to fold.
    let has_target = Memo::new(move |_| {
        let stage = stage.get();
        shown.with(|compilation| {
            compilation
                .as_ref()
                .and_then(|compilation| compilation.output(stage))
                .and_then(|output| target::opening(stage, output))
                .is_some()
        })
    });
    // Whether the stage's output has call-frame directives to fold.
    let has_frames = Memo::new(move |_| {
        stage.get() == Stage::Asm
            && shown.with(|compilation| {
                compilation
                    .as_ref()
                    .and_then(|compilation| compilation.output(Stage::Asm))
                    .is_some_and(|output| frames::count(output) > 0)
            })
    });
    // Whether the source is the lesson that follows one program through
    // every stage.
    let on_path = move || {
        source.with(|source| demos::matching(source).is_some_and(|demo| !demo.path.is_empty()))
    };
    // What this stage did to the program: the whole-path lesson's own note,
    // which describes its -O0 output, or else an example read from the
    // output on screen.
    let path_note = move || -> Option<String> {
        let stage = stage.get();
        let index = Stage::ALL.iter().position(|&each| each == stage)?;
        let written = source
            .with(|source| demos::matching(source).and_then(|demo| demo.path.get(index).copied()));
        match written {
            | Some(note) if !optimize.get() => Some(note.to_owned()),
            | _ => shown.with(|compilation| {
                source.with(|source| crate::pipeline::example(stage, compilation.as_ref()?, source))
            }),
        }
    };
    let on_hover = Callback::new(move |line| {
        pointing.hover(Side::Output, line);
        keeper.pointer(line);
    });
    let on_cursor = Callback::new(move |line| pointing.cursor(Side::Output, line));
    let side_by_side = Memo::new(move |_| compare.get() && stage.get().differs_by_level());
    // Whether the stage has output at the other level: shown beside a level
    // that failed, it is still worth comparing.
    let other_output = Memo::new(move |_| {
        let stage = stage.get();
        let unfolded = unfolded.get();
        other.with(|compilation| {
            compilation
                .as_ref()
                .is_some_and(|compilation| displayed(stage, compilation, unfolded).is_some())
        })
    });
    let outputs_hidden = Memo::new(move |_| {
        output.with(Option::is_none) && !(side_by_side.get() && other_output.get())
    });
    let missing_text = move || {
        shown.with(|compilation| {
            missing(stage.get(), lesson.get(), compilation.as_ref(), &live.get())
        })
    };
    view! {
        <section class="pipeline" aria-labelledby="pipeline-title">
            <h2 id="pipeline-title" class="sr-only">"Compiler pipeline"</h2>
            <StageStrip stage compilation=shown />
            // The selected key's panel. Where the card is wide, its output
            // takes the main column, under the stage's name, and what the
            // stage is and does the narrow one beside it, so the output
            // starts high on a laptop; elsewhere they stack, the output
            // first (style.css).
            <article
                class="card stage-card"
                class:is-compare=side_by_side
                id="stage-output"
                role="tabpanel"
                aria-labelledby=move || format!("stage-tab-{}", stage.get().key())
                data-phase=move || stage.get().phase()
            >
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
                <div class="stage-notes">
                {move || {
                    let stage = stage.get();
                    match (stage.command(&filename.get(), optimize.get()), stage.internal()) {
                        | (Some(command), _) => view! { <code class="stage-command">{format!("$ {command}")}</code> }.into_any(),
                        | (None, Some(path)) => view! {
                            <p class="stage-command is-internal">
                                "No command prints this stage: it happens inside bcc, in "
                                // A narrow screen wraps the path after a
                                // `/`, never inside a name.
                                <a href=format!("{SOURCE_URL}{path}")><code class="stage-path">{path.split_inclusive('/').map(|part| view! { {part}<wbr /> }).collect_view()}</code></a>
                            </p>
                        }.into_any(),
                        | (None, None) => ().into_any(),
                    }
                }}
                <div class="stage-lead">
                <div class="stage-lead-main">
                <p class="stage-handoff">
                    <span>{move || stage.get().handoff()}</span>
                    // One button for every stage, so the keyboard focus stays
                    // on it as it moves along. After the last stage the
                    // whole-path lesson goes on to the abstract machine,
                    // which runs it; other programs, which it may not, start
                    // the path again.
                    <button
                        type="button"
                        class="stage-next"
                        on:click=move |_| match stage.get_untracked().next() {
                            | Some(next) => stage.set(next),
                            | None if on_path() => on_run.run(()),
                            | None => stage.set(Stage::ALL[0]),
                        }
                    >
                        {move || match stage.get().next() {
                            | Some(next) => format!("Next: {}", next.label()),
                            | None if on_path() => "Next: watch it run in the abstract machine".to_owned(),
                            | None => format!("Start again: {}", Stage::ALL[0].label()),
                        }}
                        <span aria-hidden="true">" →"</span>
                    </button>
                    // The pipeline opens at its first stage; a reader who
                    // came for the IR skips the front end from any of its
                    // stages.
                    <button
                        type="button"
                        class="stage-skip"
                        hidden=move || {
                            let stage = stage.get();
                            stage.phase() != "Front end" || stage.next() == Some(Stage::Ir)
                        }
                        on:click=move |_| show_ir(stage)
                    >
                        {format!("Skip to {}", Stage::Ir.label())}
                        <span aria-hidden="true">" ⇥"</span>
                    </button>
                </p>
                <p class="stage-path-note" hidden=move || path_note().is_none()>
                    <strong>"In this program: "</strong>
                    {move || path_note().map(|note| super::rich_text(&note))}
                </p>
                </div>
                <StageBrief stage />
                </div>
                <div class="stage-toolbar">
                    <p class="stage-note is-link" hidden=move || shown_links.with(Links::is_empty)>
                        {move || super::touch_words(if !shown_links.with(Links::is_exact) {
                            "Hover a source line, or move the cursor to it, to find its function here; do the same \
                             here to find the source. No line of this program has a source location, so it links \
                             per function."
                        } else if shown_links.with(Links::has_origins) {
                            stage.get().link_note()
                        } else {
                            "Hover a source line, or move the cursor to it, to find what it became here; do the \
                             same here to find its source."
                        })}
                    </p>
                    <button
                        type="button"
                        class="toggle-chip"
                        hidden=move || stage.get() != Stage::Preprocessor || output.with(Option::is_none)
                        aria-pressed=move || unfolded.get().headers.to_string()
                        title="Show the text each #include brought in, or fold it into one line"
                        on:click=move |_| unfolded.update(|unfolded| unfolded.headers = !unfolded.headers)
                    >
                        <span class="toggle-chip-dot" aria-hidden="true"></span>
                        "Show headers"
                    </button>
                    <button
                        type="button"
                        class="toggle-chip"
                        hidden=move || !has_target.get()
                        aria-pressed=move || unfolded.get().target.to_string()
                        title="Show the lines that name the target machine and its data layout, or fold them into one line"
                        on:click=move |_| unfolded.update(|unfolded| unfolded.target = !unfolded.target)
                    >
                        <span class="toggle-chip-dot" aria-hidden="true"></span>
                        "Show target"
                    </button>
                    <button
                        type="button"
                        class="toggle-chip"
                        hidden=move || !has_frames.get()
                        aria-pressed=move || unfolded.get().frames.to_string()
                        title="Show the .cfi_ directives, which describe each function's stack frame for debuggers and unwinding, or fold them into one line"
                        on:click=move |_| unfolded.update(|unfolded| unfolded.frames = !unfolded.frames)
                    >
                        <span class="toggle-chip-dot" aria-hidden="true"></span>
                        "Show call frames"
                    </button>
                    <button
                        type="button"
                        class="toggle-chip"
                        hidden=move || {
                            !stage.get().differs_by_level()
                                || output.with(Option::is_none) && !other_output.get()
                        }
                        aria-pressed=move || compare.get().to_string()
                        title="Show the -O0 and the -O2 output together: side by side, or one above the other on a narrow screen"
                        on:click=move |_| {
                            compare.update(|compare| *compare = !*compare);
                            pointing.clear_beside();
                        }
                    >
                        <span class="toggle-chip-dot" aria-hidden="true"></span>
                        "Compare -O0 with -O2"
                    </button>
                </div>
                </div>
                <div class="stage-work">
                <StopBanner
                    compilation=shown
                    level=Signal::derive(move || side_by_side.get().then(|| level_name(optimize.get())))
                />
                <p
                    class="link-reading"
                    aria-hidden="true"
                    hidden=move || {
                        let main = output.with(Option::is_some) && !shown_links.with(Links::is_empty);
                        let beside = side_by_side.get() && !other_links.with(Links::is_empty);
                        !main && !beside
                    }
                >
                    <ReadingText reading reading_room />
                </p>
                <div
                    class="stage-output"
                    class:is-compare=side_by_side
                    data-level=move || if optimize.get() { "O2" } else { "O0" }
                    hidden=move || outputs_hidden.get()
                >
                    <div class="stage-main">
                    <p class="stage-level" hidden=move || !side_by_side.get()>
                        {move || level_caption(optimize.get())}
                    </p>
                    // Beside the other level's output, this level's failure
                    // explains itself in its own place.
                    <p class="stage-missing stage-pane-missing" hidden=move || output.with(Option::is_some)>
                        {missing_text}
                    </p>
                    <div class="stage-pane-editor" hidden=move || output.with(Option::is_none)>
                    <CodeEditor
                        text
                        language
                        theme
                        label="Compiler output (read only)"
                        read_only=true
                        decorations=layers
                        on_hover_line=on_hover
                        on_cursor_line=on_cursor
                        reveal_linked=reveal
                        on_top_line=Callback::new(move |line| keeper.top_line(line))
                        keep_place=Callback::new(move |()| keeper.first_line())
                        line_numbers
                    />
                    </div>
                    <p class="stage-scroll-hint" hidden=move || stage.get() != Stage::MachineCode || output.with(Option::is_none)>
                        <i aria-hidden="true">"⇆"</i>
                        <span class="on-pointer">"Scroll"</span>
                        <span class="on-touch">"Swipe"</span>
                        " the listing sideways: the instructions run on past its right edge."
                    </p>
                    </div>
                    {move || side_by_side.get().then(|| view! {
                        <Beside stage optimize other other_links lesson theme pointing unfolded place />
                    })}
                    <footer class="link-legend" hidden=move || !shown_links.with(Links::has_origins)>
                        <span><i class="swatch swatch-link"></i>"came from the line you point at"</span>
                        <span><i class="swatch swatch-inlined"></i>"inlined from another function"</span>
                        <span><i class="swatch swatch-unclaimed"></i>"no single source line"</span>
                    </footer>
                </div>
                <StageNotation stage text hidden=Signal::derive(move || outputs_hidden.get()) />
                <div class="stage-missing" hidden=move || !outputs_hidden.get()>
                    <p>{missing_text}</p>
                    <div class="stage-missing-actions" hidden=move || stage.get() != Stage::Llvm || lesson.get()>
                        <button type="button" class="button small" on:click=move |_| show_ir(stage)>
                            "Show bcc IR, compiled live"
                        </button>
                        <code class="stage-command">{move || Stage::Llvm.command(&filename.get(), optimize.get()).map(|command| format!("$ {command}"))}</code>
                    </div>
                </div>
                </div>
                // What the stage did comes first, where a 1440x900 screen
                // shows it; how it works and what the optimizer changed
                // follow it.
                <p class="stage-explanation">{move || stage.get().explanation()}</p>
                <StageClauses stage />
                <FrontEndCard stage compilation on_reveal />
                <OptimizerNote stage optimize compilation other shown=Signal::derive(move || output.with(Option::is_some)) />
            </article>
            <Diagnostics compilation live on_reveal />
        </section>
    }
}

/// The sentence a link reading shows, over the longest sentences it can
/// show in the view, laid out unseen in the same place: the box is as tall
/// as the tallest of them, so it does not grow when the pointer reaches a
/// line with a longer sentence, and the listing under it stays put.
#[component]
pub fn ReadingText(
    reading: Signal<Option<String>>,
    reading_room: Signal<Vec<String>>,
) -> impl IntoView {
    view! {
        <span class="link-reading-text">
            {move || reading.get().map(|text| super::touch_words(&text))}
        </span>
        {move || {
            reading_room
                .get()
                .into_iter()
                .map(|text| view! { <span class="link-reading-room">{super::touch_words(&text)}</span> })
                .collect_view()
        }}
    }
}

/// What goes into the stage, what comes out and why a compiler has it, with
/// the terms linked to the glossary.
#[component]
fn StageBrief(stage: RwSignal<Stage>) -> impl IntoView {
    let row = move |label: &'static str, part: fn(Brief) -> &'static str| {
        view! {
            <div>
                <dt>{label}</dt>
                <dd>{move || linked(part(stage.get().brief()))}</dd>
            </div>
        }
    };
    view! {
        <dl class="stage-brief" aria-label="What this stage is">
            {row("In", |brief| brief.input)}
            {row("Out", |brief| brief.output)}
            {row("Why", |brief| brief.why)}
        </dl>
    }
}

/// `text` with each `[text](id)` a link to the glossary entry `#term-id`.
fn linked(text: &'static str) -> impl IntoView {
    linked_terms(text)
        .into_iter()
        .map(|(piece, id)| match id {
            | Some(id) =>
                view! { <a class="stage-term" href=format!("#term-{id}")>{piece}</a> }.into_any(),
            | None => piece.into_any(),
        })
        .collect_view()
}

/// The optimization level `optimized` names.
const fn level_name(optimized: bool) -> &'static str {
    if optimized { "-O2" } else { "-O0" }
}

/// The caption over one level's output when both are side by side.
fn level_caption(optimized: bool) -> &'static str {
    if optimized {
        "-O2: optimized"
    } else {
        "-O0: each operation as written"
    }
}

/// The other level's output, beside the selected level's: its lines linked
/// to the source as the selected level's are, so a source line marks what it
/// became at both levels. When that level has no output for the stage, why
/// not, as the selected level would say it.
#[component]
fn Beside(
    stage: RwSignal<Stage>,
    optimize: RwSignal<bool>,
    other: Memo<Option<Compilation>>,
    other_links: Memo<Links>,
    lesson: Memo<bool>,
    theme: Signal<&'static str>,
    pointing: Pointing,
    unfolded: RwSignal<Unfolded>,
    /// The reader's place, which this level's output keeps too.
    place: StoredValue<Option<Place>>,
) -> impl IntoView {
    let output = Memo::new(move |_| {
        let stage = stage.get();
        let unfolded = unfolded.get();
        other.with(|compilation| {
            compilation
                .as_ref()
                .and_then(|compilation| displayed(stage, compilation, unfolded))
                .map(|(text, _)| text)
        })
    });
    let text = Signal::derive(move || output.get().unwrap_or_default());
    let line_numbers = Signal::derive(move || {
        let stage = stage.get();
        let unfolded = unfolded.get();
        other.with(|compilation| {
            compilation
                .as_ref()
                .map_or_else(Vec::new, |compilation| gutter(stage, compilation, unfolded))
        })
    });
    let layers = Signal::derive(move || -> Vec<Layer> {
        let hovered = pointing.line(Side::Source);
        let linked = if hovered > 0 {
            other_links.with(|links| {
                links
                    .outputs_for(hovered)
                    .into_iter()
                    .map(|line| {
                        let class = if links.origin(line).call == 0 {
                            "link-line"
                        } else {
                            "link-inlined"
                        };
                        Decoration::line(line, class)
                    })
                    .collect()
            })
        } else {
            Vec::new()
        };
        let unclaimed = other_links.with(|links| {
            links
                .unclaimed()
                .map(|line| Decoration {
                    start_line: line,
                    end_line: line,
                    margin_class: Some("link-unclaimed".to_owned()),
                    ..Decoration::default()
                })
                .collect()
        });
        vec![
            ("links", linked),
            ("stage", output_marks(stage.get(), other, text)),
            ("unclaimed", unclaimed),
        ]
    });
    let reveal = Signal::derive(move || {
        let hovered = pointing.line(Side::Source);
        (hovered > 0)
            .then(|| other_links.with(|links| links.run_for(hovered)))
            .flatten()
    });
    let keeper = PlaceKeeper::new(place, other_links, other);
    let on_hover = Callback::new(move |line| {
        pointing.hover_beside(line);
        keeper.pointer(line);
    });
    let on_cursor = Callback::new(move |line| pointing.cursor_beside(line));
    // An editor disposed under the pointer or with the focus reports no
    // leaving.
    on_cleanup(move || pointing.clear_beside());
    let language = Signal::derive(move || stage.get().language());
    let waiting = Memo::new(move |_| other.with(Option::is_none));
    let failed = Memo::new(move |_| !waiting.get() && output.with(Option::is_none));
    let level = Signal::derive(move || Some(level_name(!optimize.get())));
    view! {
        <div class="stage-beside">
            <p class="stage-level">
                {move || level_caption(!optimize.get())}
            </p>
            <p class="stage-beside-waiting" hidden=move || !waiting.get()>
                {move || format!("Compiling at {}…", level_name(!optimize.get()))}
            </p>
            <div class="stage-pane-failure" hidden=move || !failed.get()>
                <StopBanner compilation=other level />
                <p class="stage-missing stage-pane-missing">
                    {move || other.with(|compilation| {
                        missing(stage.get(), lesson.get(), compilation.as_ref(), &LiveCompile::Compiled)
                    })}
                </p>
            </div>
            <div class="stage-pane-editor" hidden=move || waiting.get() || failed.get()>
                <CodeEditor
                    text
                    language
                    theme
                    label="Compiler output at the other optimization level (read only)"
                    read_only=true
                    decorations=layers
                    on_hover_line=on_hover
                    on_cursor_line=on_cursor
                    reveal_linked=reveal
                    on_top_line=Callback::new(move |line| keeper.top_line(line))
                    keep_place=Callback::new(move |()| keeper.first_line())
                    line_numbers
                />
            </div>
        </div>
    }
}

/// Where the reader is in one output editor: its first line in view and the
/// line under the pointer, from which it keeps the reader's place, shared
/// by both levels' outputs. When the editor's text changes (another level,
/// another stage, an edit), it opens at that place in the new text.
#[derive(Clone, Copy)]
struct PlaceKeeper {
    place:       StoredValue<Option<Place>>,
    top:         StoredValue<usize>,
    pointed:     StoredValue<usize>,
    /// Whether the editor just scrolled to the place itself: its report of
    /// that scroll leaves the place as it was, so that a round trip through
    /// a view without the place's line (`-O2` folded it away) comes back to
    /// it, not to where the other view had to stand in for it.
    placed:      StoredValue<bool>,
    /// The links of the text on screen, and its compilation.
    links:       Memo<Links>,
    compilation: Memo<Option<Compilation>>,
}

impl PlaceKeeper {
    fn new(
        place: StoredValue<Option<Place>>,
        links: Memo<Links>,
        compilation: Memo<Option<Compilation>>,
    ) -> Self {
        Self {
            place,
            top: StoredValue::new(1),
            pointed: StoredValue::new(0),
            placed: StoredValue::new(false),
            links,
            compilation,
        }
    }

    /// The editor scrolled: `line` is its first line in view.
    fn top_line(self, line: usize) {
        self.top.set_value(line.max(1));
        if self.placed.get_value() {
            self.placed.set_value(false);
            return;
        }
        self.note();
    }

    /// The pointer moved over `line` of the editor (0 when it left).
    fn pointer(self, line: usize) {
        self.pointed.set_value(line);
        self.note();
    }

    /// Read the place from what the editor shows: the line under the
    /// pointer, else the first line in view.
    fn note(self) {
        let top = self.top.get_value();
        let line = match self.pointed.get_value() {
            | 0 => top,
            | line => line.max(top),
        };
        if let Some(place) = self.links.with_untracked(|links| links.place(top, line)) {
            self.place.set_value(Some(place));
        }
    }

    /// The line the editor shows first in its new text (0: keep the scroll
    /// offset, before the reader has a place). The pointer's line in the old
    /// text means nothing in the new one, so it is forgotten until the
    /// pointer moves.
    fn first_line(self) -> usize {
        self.pointed.set_value(0);
        let Some(place) = self.place.get_value() else {
            return 0;
        };
        let function = self.compilation.with_untracked(|compilation| {
            compilation
                .as_ref()
                .and_then(|compilation| compilation.function_around(place.source))
        });
        let top = self
            .links
            .with_untracked(|links| links.top_for(place, function));
        self.placed.set_value(true);
        top
    }
}

/// Switches to the bcc IR stage and moves the focus to its tab, since the
/// button that asked for it disappears as the stage changes.
fn show_ir(stage: RwSignal<Stage>) {
    focus_stage(stage, Stage::Ir);
}

/// Selects `entry` and moves the focus to its key in the stage strip.
fn focus_stage(stage: RwSignal<Stage>, entry: Stage) {
    stage.set(entry);
    focus_stage_key(entry);
}

/// Moves the focus to `entry`'s key in the stage strip, if the strip is on
/// the page.
pub fn focus_stage_key(entry: Stage) {
    #[cfg(feature = "hydrate")]
    if let Some(tab) = document()
        .get_element_by_id(&format!("stage-tab-{}", entry.key()))
        .and_then(|tab| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(tab).ok())
    {
        _ = tab.focus();
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = entry;
}

/// Which stage an error stopped bcc in, and why.
#[component]
fn StopBanner(
    compilation: Memo<Option<Compilation>>,
    /// The optimization level it names, when both are on screen.
    #[prop(optional, into)]
    level: MaybeProp<&'static str>,
) -> impl IntoView {
    let stop = Memo::new(move |_| {
        compilation.with(|compilation| compilation.as_ref().and_then(Compilation::stop))
    });
    let at = move || level.get().map_or_default(|level| format!(" at {level}"));
    view! {
        <div class="stop-banner" role="status" hidden=move || stop.with(Option::is_none)>
            <p class="stop-title">
                <span class="stop-icon" aria-hidden="true">"✕"</span>
                {move || stop.with(|stop| stop.as_ref().map(|stop| format!("Compilation{} stopped in the {}", at(), stop.phase.to_lowercase())))}
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
                    // One key of the strip takes Tab, the selected one; the
                    // arrow keys move along the strip (`moved`).
                    tabindex=move || if stage.get() == entry { "0" } else { "-1" }
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
    // Where the keys scroll in one row (a phone), the selected one comes
    // into view.
    let strip = NodeRef::<leptos::html::Div>::new();
    Effect::new(move || {
        stage.track();
        let Some(strip) = strip.get() else {
            return;
        };
        if strip.scroll_width() <= strip.client_width() {
            return;
        }
        if let Some(key) = strip
            .query_selector("[aria-selected=\"true\"]")
            .ok()
            .flatten()
        {
            let offset =
                key.get_bounding_client_rect().left() - strip.get_bounding_client_rect().left();
            let left = f64::from(strip.scroll_left()) + offset - 12.0;
            let options = web_sys::ScrollToOptions::new();
            options.set_left(left.max(0.0));
            strip.scroll_to_with_scroll_to_options(&options);
        }
    });
    // The tab list's keys, as WAI-ARIA's pattern has them for a horizontal
    // tab list: `←` and `→` select the stage before or after (around the
    // ends), Home and End the first and last, and the focus follows; `↑`
    // and `↓` scroll the page. The pipeline view does not step the
    // machine, so the arrow keys are free here.
    let moved = move |event: web_sys::KeyboardEvent| {
        let current = stage.get_untracked();
        let index = Stage::ALL
            .iter()
            .position(|&each| each == current)
            .unwrap_or(0);
        let last = Stage::ALL.len() - 1;
        let next = match event.key().as_str() {
            | "ArrowRight" =>
                if index == last {
                    0
                } else {
                    index + 1
                },
            | "ArrowLeft" =>
                if index == 0 {
                    last
                } else {
                    index - 1
                },
            | "Home" => 0,
            | "End" => last,
            | _ => return,
        };
        event.prevent_default();
        focus_stage(stage, Stage::ALL[next]);
    };
    view! {
        <div class="stage-strip" role="tablist" aria-label="Compiler stages" node_ref=strip on:keydown=moved>
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
/// logical line, the folded headers of the preprocessed text, and the IRs'
/// folded target lines, with what they say on hover.
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
        | Stage::Asm => {
            let folded = text.with(|text| text.lines().next().is_some_and(frames::is_summary));
            let count = compilation.with(|compilation| {
                compilation
                    .as_ref()
                    .and_then(|compilation| compilation.output(stage))
                    .map_or(0, frames::count)
            });
            if folded && count > 0 {
                vec![Decoration {
                    hover: Some(frames::explanation(count)),
                    ..Decoration::line(1, "folded-line")
                }]
            } else {
                Vec::new()
            }
        },
        | Stage::Ir | Stage::IrAbi | Stage::Llvm => {
            let folded = text.with(|text| text.lines().next().is_some_and(target::is_summary));
            let hover = compilation.with(|compilation| {
                compilation
                    .as_ref()?
                    .output(stage)
                    .and_then(|output| target::opening(stage, output))
                    .map(|opening| target::explanation(&opening))
            });
            match (folded, hover) {
                | (true, Some(hover)) => vec![Decoration {
                    hover: Some(hover),
                    ..Decoration::line(1, "folded-line")
                }],
                | _ => Vec::new(),
            }
        },
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
    // The same comparison as bars: -O0 above -O2 for each measure.
    let measures = Memo::new(move |_| {
        let stage = stage.get();
        compilation.with(|current| {
            other.with(|other| match (current, other) {
                | (Some(current), Some(other)) if optimize.get() =>
                    optimizer_measures(stage, other, current),
                | (Some(current), Some(other)) => optimizer_measures(stage, current, other),
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
            <OptimizerBars measures />
        </aside>
    }
}

/// What the optimizer removed, drawn: for each measure, a bar for `-O0`
/// and one for `-O2`, both scaled to the larger, so a measure the
/// optimizer halved shows a bar half as long.
#[component]
fn OptimizerBars(measures: Memo<Vec<(&'static str, usize, usize)>>) -> impl IntoView {
    let label = move || {
        measures.with(|measures| {
            let rows: Vec<String> = measures
                .iter()
                .map(|(what, before, after)| format!("{what}: {before} at -O0, {after} at -O2"))
                .collect();
            format!("This stage at -O0 and at -O2: {}.", rows.join("; "))
        })
    };
    let bar = |count: usize, most: usize, level: &'static str| {
        // The share of the larger count, in thousandths, of the room the
        // count's digits leave.
        let share = (count * 1000).checked_div(most).unwrap_or(0);
        view! {
            <span class="optimizer-bar" data-level=level>
                <span
                    class="optimizer-bar-fill"
                    style=format!("width: calc((100% - 7ch) * {}.{:03})", share / 1000, share % 1000)
                ></span>
                <span class="optimizer-bar-count">{count}</span>
            </span>
        }
    };
    view! {
        <figure class="optimizer-bars" hidden=move || measures.with(Vec::is_empty) aria-label=label>
            <figcaption>
                "-O0 "<span class="optimizer-key" data-level="O0" aria-hidden="true"></span>
                " against -O2 "<span class="optimizer-key" data-level="O2" aria-hidden="true"></span>
            </figcaption>
            <div class="optimizer-bars-grid" aria-hidden="true">
                {move || measures.get().into_iter().map(|(what, before, after)| {
                    let most = before.max(after);
                    view! {
                        <span class="optimizer-measure">{what}</span>
                        <span class="optimizer-pair">
                            {bar(before, most, "O0")}
                            {bar(after, most, "O2")}
                        </span>
                    }
                }).collect_view()}
            </div>
        </figure>
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
        Unfolded,
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

    /// The front end's stages but the last offer the skip to the IR, which
    /// no other stage shows; the card is the selected stage key's tab panel,
    /// and that key alone takes Tab.
    #[cfg(feature = "ssr")]
    #[test]
    fn the_front_end_skips_to_the_ir_and_the_keys_are_tabs() {
        for stage in Stage::ALL {
            let html = render_stage(
                stage,
                false,
                false,
                LiveCompile::Compiling,
                [None, None, None],
            );
            let skip = html
                .split("<button")
                .find(|tag| tag.contains("stage-skip"))
                .and_then(|tag| tag.split('>').next())
                .unwrap_or_else(|| panic!("{stage:?} has no skip key"));
            let offered = stage.phase() == "Front end" && stage != Stage::SemanticTypes;
            assert_eq!(!skip.contains("hidden"), offered, "{stage:?}: {skip}");
            assert!(
                html.contains(&format!(
                    "role=\"tabpanel\" aria-labelledby=\"stage-tab-{}\"",
                    stage.key()
                )),
                "{stage:?}"
            );
            assert_eq!(
                html.matches("tabindex=\"0\" id=\"stage-tab-").count(),
                1,
                "{stage:?}"
            );
            assert_eq!(
                html.matches("tabindex=\"-1\" id=\"stage-tab-").count(),
                Stage::ALL.len() - 1,
                "{stage:?}"
            );
            assert!(
                html.contains(&format!("tabindex=\"0\" id=\"stage-tab-{}\"", stage.key())),
                "{stage:?}"
            );
        }
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
        render_stage(
            Stage::Ir,
            false,
            optimize,
            LiveCompile::Compiling,
            [current, held, other],
        )
    }

    /// The same at `stage`, with both levels side by side when `compare`,
    /// given the compiler's progress.
    #[cfg(feature = "ssr")]
    fn render_stage(
        stage: Stage,
        compare: bool,
        optimize: bool,
        live: LiveCompile,
        [current, held, other]: [Option<Compilation>; 3],
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
                    stage=RwSignal::new(stage)
                    optimize=RwSignal::new(optimize)
                    compilation
                    shown
                    links=Memo::new(|_| crate::pipeline::Links::default())
                    reading=Signal::stored(None)
                    reading_room=Signal::stored(Vec::new())
                    other
                    compare=RwSignal::new(compare)
                    other_links=Memo::new(|_| crate::pipeline::Links::default())
                    live=Signal::stored(live)
                    theme=Signal::stored("dark")
                    pointing=Pointing::new()
                    unfolded=RwSignal::new(Unfolded::default())
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

    /// A compilation whose back end refused the program, so it has IR but
    /// no assembly.
    #[cfg(feature = "ssr")]
    fn refused() -> Compilation {
        Compilation {
            tokens: Some(String::new()),
            ir: Some("function @refused_ir() -> i32 external {\n".into()),
            diagnostics: vec![CompileDiagnostic {
                severity: "error".into(),
                stage: "back-end".into(),
                message: "no x86-64 lowering for read_timestamp".into(),
                ..CompileDiagnostic::default()
            }],
            ..Compilation::default()
        }
    }

    /// A compilation with assembly.
    #[cfg(feature = "ssr")]
    fn assembled() -> Compilation {
        Compilation {
            tokens: Some(String::new()),
            ir: Some("function @main() -> i32 external {\n".into()),
            asm: Some("main:\n    xor eax, eax\n    ret_assembled\n".into()),
            ..Compilation::default()
        }
    }

    #[test]
    #[cfg(feature = "ssr")]
    fn a_level_that_failed_explains_itself_beside_the_other() {
        let why = "This stage did not run: bcc stopped in the back end.";
        let reason = "no x86-64 lowering for read_timestamp";
        // -O2 on screen, -O0 beside it failed.
        let beside = render_stage(
            Stage::Asm,
            true,
            true,
            LiveCompile::Compiled,
            [Some(assembled()), Some(assembled()), Some(refused())],
        );
        assert!(beside.contains("ret_assembled"), "{beside}");
        assert!(
            beside.contains("Compilation at -O0 stopped in the back end"),
            "{beside}"
        );
        assert!(beside.contains(reason), "{beside}");
        assert!(beside.contains(why), "{beside}");
        // -O0 on screen failed, -O2 beside it did not: its output stays.
        let selected = render_stage(
            Stage::Asm,
            true,
            false,
            LiveCompile::Compiled,
            [Some(refused()), Some(refused()), Some(assembled())],
        );
        assert!(selected.contains("ret_assembled"), "{selected}");
        assert!(
            selected.contains("Compilation at -O0 stopped in the back end"),
            "{selected}"
        );
        assert!(selected.contains(why), "{selected}");
        assert!(!selected.contains("Compilation at -O2"), "{selected}");
        // On its own, the failed level names no level.
        let alone = render_stage(
            Stage::Asm,
            false,
            false,
            LiveCompile::Compiled,
            [Some(refused()), Some(refused()), Some(assembled())],
        );
        assert!(
            alone.contains("Compilation stopped in the back end"),
            "{alone}"
        );
        assert!(alone.contains(why), "{alone}");
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
