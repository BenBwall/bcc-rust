//! The compiler pipeline: a strip of stages from tokens to machine code, an
//! explanation of the selected stage, and bcc's real output for it in a
//! read-only editor. Hovering a source line highlights the output lines that
//! came from it (and the reverse) where the stage records source locations.

use leptos::prelude::*;

use crate::{
    editor::{
        CodeEditor,
        Decoration,
        Layer,
    },
    pipeline::{
        Compilation,
        Links,
        Stage,
        links,
    },
};

#[component]
#[expect(clippy::too_many_arguments, reason = "The page state the view reads.")]
pub fn PipelineView(
    stage: RwSignal<Stage>,
    compilation: Memo<Option<Compilation>>,
    edited: Memo<bool>,
    live_error: ReadSignal<Option<String>>,
    theme: Signal<&'static str>,
    hover_source: ReadSignal<usize>,
    hover_output: RwSignal<usize>,
    filename: ReadSignal<String>,
    /// The source the compilation came from.
    source: Signal<String>,
) -> impl IntoView {
    let output = Memo::new(move |_| {
        let stage = stage.get();
        compilation.with(|compilation| {
            compilation
                .as_ref()
                .and_then(|compilation| compilation.output(stage))
                .map(str::to_owned)
        })
    });
    let text = Signal::derive(move || output.get().unwrap_or_default());
    let language = Signal::derive(move || stage.get().language());
    let links = Memo::new(move |_| {
        output.with(|output| {
            output
                .as_deref()
                .map(|output| source.with(|source| links(stage.get(), output, source)))
                .unwrap_or_default()
        })
    });
    let layers = Signal::derive(move || -> Vec<Layer> {
        let hovered = hover_source.get();
        let linked = if hovered > 0 {
            links.with(|links| links.outputs_for(hovered))
        } else {
            Vec::new()
        };
        vec![(
            "links",
            linked
                .into_iter()
                .map(|line| Decoration::line(line, "link-line"))
                .collect(),
        )]
    });
    let reveal = Signal::derive(move || {
        let hovered = hover_source.get();
        (hovered > 0)
            .then(|| links.with(|links| links.outputs_for(hovered).first().copied()))
            .flatten()
    });
    let on_hover = Callback::new(move |line| hover_output.set(line));
    view! {
        <section class="pipeline" aria-labelledby="pipeline-title">
            <h2 id="pipeline-title" class="sr-only">"Compiler pipeline"</h2>
            <StageStrip stage />
            <article class="card stage-card" id="stage-output">
                <header class="stage-header">
                    <div>
                        <p class="stage-kicker">{move || stage.get().phase()}</p>
                        <h3>{move || stage.get().label()}</h3>
                    </div>
                    <code class="stage-command">
                        {move || format!("$ {}", stage.get().command(&filename.get()))}
                    </code>
                </header>
                <p class="stage-explanation">{move || stage.get().explanation()}</p>
                <p class="stage-note is-link" hidden=move || links.with(Links::is_empty)>
                    {move || if links.with(Links::is_exact) {
                        "Hover a source line to find what it became here, or hover here to find its source."
                    } else {
                        "Hover a source line to find its function here, or hover here to find the source. bcc's IR \
                         keeps no source locations yet, so these stages link per function, not per line."
                    }}
                </p>
                <div class="stage-output" hidden=move || output.with(Option::is_none)>
                    <CodeEditor
                        text
                        language
                        theme
                        label="Compiler output (read only)"
                        read_only=true
                        decorations=layers
                        on_hover_line=on_hover
                        reveal
                    />
                </div>
                <div class="stage-missing" hidden=move || output.with(Option::is_some)>
                    {move || missing(stage.get(), edited.get(), compilation.with(Option::is_some), live_error.get())}
                </div>
            </article>
            <Diagnostics compilation />
        </section>
    }
}

/// The stages as tabs, grouped under front, middle and back end.
#[component]
fn StageStrip(stage: RwSignal<Stage>) -> impl IntoView {
    view! {
            <div class="stage-phases" aria-hidden="true">
                <span data-phase="Front end">"Front end"</span>
                <span data-phase="Middle end">"Middle end"</span>
                <span data-phase="Back end">"Back end"</span>
            </div>
            <ol class="stage-strip" role="tablist" aria-label="Compiler stages">
                {Stage::ALL.iter().enumerate().map(|(index, &entry)| view! {
                    <li class="stage-item">
                        <button
                            type="button"
                            role="tab"
                            class="stage-button"
                            data-phase=entry.phase()
                            aria-selected=move || (stage.get() == entry).to_string()
                            aria-controls="stage-output"
                            on:click=move |_| stage.set(entry)
                        >
                            <span class="stage-index">{index + 1}</span>
                            <span class="stage-name">{entry.label()}</span>
                            <span class="sr-only">{format!(" ({})", entry.phase())}</span>
                        </button>
                    </li>
                }).collect_view()}
            </ol>
    }
}

/// bcc's warnings and errors for the program.
#[component]
fn Diagnostics(compilation: Memo<Option<Compilation>>) -> impl IntoView {
    let diagnostics = Memo::new(move |_| {
        compilation.with(|compilation| {
            compilation
                .as_ref()
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
                        <pre class="diagnostics-list">{list.join("\n")}</pre>
                    }.into_any(),
                    | None => view! { <p class="muted">"No compilation for this source yet."</p> }.into_any(),
                }}
            </article>
    }
}

/// Why a stage has no output to show.
fn missing(stage: Stage, edited: bool, compiled: bool, live_error: Option<String>) -> String {
    if stage == Stage::Llvm && edited && compiled {
        return "LLVM IR is precomputed for the bundled lessons only: the in-browser build of bcc \
                has no LLVM. Every other stage is compiled live from your edits."
            .to_owned();
    }
    if edited && !compiled {
        let reason = live_error.unwrap_or_else(|| "Compiling your edits…".to_owned());
        return format!(
            "{reason} The compiler views are available for the bundled lessons; pick one, or undo \
             your edits to see its output again. LLVM IR is only ever precomputed, because the \
             browser build of bcc has no LLVM."
        );
    }
    "bcc produced no output for this stage.".to_owned()
}
