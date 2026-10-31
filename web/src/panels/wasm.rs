//! The WebAssembly view: which compiler made the module (bcc in the
//! browser, or clang ahead of time for a lesson, with a switch between
//! them), the narration of the last instruction and the machine's state
//! (operand stack, locals, call stack, the blocks a branch can reach, globals,
//! linear memory and the program's output), and the module's text-format
//! listing, which the page shows under the C source with the instruction just
//! executed and the next one.

mod memory;
mod state;

use leptos::prelude::*;

use super::rich_text;
use crate::{
    app::{
        links::{
            Pointing,
            Side,
        },
        wasm::{
            Compiler,
            WasmState,
        },
    },
    editor::CodeEditor,
};

#[component]
pub fn WasmView(
    wasm: WasmState,
    /// Whether the editor's text differs from the program shown.
    stale: Signal<bool>,
    on_jump: Callback<u64>,
    /// Step the module this compiler made instead.
    on_choose: Callback<Compiler>,
) -> impl IntoView {
    let shown = move || wasm.program.with(Option::is_some);
    let problem = move || wasm.bcc_problem();
    let retry = move || problem().is_some_and(|problem| problem.retry);
    view! {
        <section class="wasm-view" aria-labelledby="wasm-title">
            <h2 id="wasm-title" class="sr-only">"WebAssembly"</h2>
            <div class="stage-missing wasm-unavailable" hidden=move || shown() || wasm.unavailable.with(Option::is_none)>
                <p class="wasm-unavailable-title">"No WebAssembly module for this program"</p>
                <p>{move || wasm.unavailable.get().unwrap_or_default()}</p>
                <pre class="wasm-diagnostic" class:is-prose=move || problem().is_some_and(|problem| problem.prose) hidden=move || problem().is_none()>
                    {move || problem().map(|problem| problem.detail).unwrap_or_default()}
                </pre>
                <button type="button" class="button small" hidden=move || !retry() disabled=move || wasm.locked_tracked() on:click=move |_| on_choose.run(Compiler::Bcc)>
                    "Compile again"
                </button>
            </div>
            <p class="edit-hint" hidden=move || !stale.get() || !shown()>
                "You changed the program. This view still steps the module compiled from the source it started with: choose Run edited code to compile your edits with bcc and step them."
            </p>
            <div class="wasm-loading empty-state" hidden=move || shown() || wasm.unavailable.with(Option::is_some)>
                "Compiling with bcc and loading the module into the stepper…"
            </div>
            <Show when=shown>
                // The narration first: it explains the instruction the
                // listing beside (or, on narrow screens, above) it marks.
                <Narration wasm on_jump />
                <Provenance wasm on_choose />
                <div class="wasm-row">
                    <state::OperandStack wasm />
                    <state::Locals wasm />
                </div>
                <div class="wasm-row">
                    <state::CallStack wasm />
                    <state::Blocks wasm />
                </div>
                <memory::LinearMemory wasm />
                <state::Globals wasm />
                <state::Console wasm />
            </Show>
        </section>
    }
}

/// Who compiled the module and what that means, the switch between bcc and
/// clang when a lesson has both, and why bcc's module is missing when
/// clang's stands in.
#[component]
fn Provenance(wasm: WasmState, on_choose: Callback<Compiler>) -> impl IntoView {
    let compiler = move || wasm.compiler();
    let both = move || wasm.choice.with(|choice| choice.offered().len() > 1);
    let problem = move || {
        wasm.choice.with(|choice| {
            (choice.compiler == Some(Compiler::Clang) && choice.preferred == Compiler::Bcc)
                .then(|| choice.bcc_problem())
                .flatten()
        })
    };
    let command = move || {
        wasm.program.with(|program| {
            program
                .as_ref()
                .map(|program| program.command.clone())
                .unwrap_or_default()
        })
    };
    let choice = move |option: Compiler| {
        view! {
            <button
                type="button"
                class="opt-option"
                aria-pressed=move || (compiler() == Some(option)).to_string()
                disabled=move || wasm.locked_tracked()
                on:click=move |_| on_choose.run(option)
            >
                {option.name()}
            </button>
        }
    };
    view! {
        <article class="card wasm-provenance" data-compiler=move || compiler().map_or("", |compiler| match compiler {
            | Compiler::Bcc => "bcc",
            | Compiler::Clang => "clang",
        })>
            <div class="wasm-provenance-head">
                <h3 class="wasm-provenance-title">
                    {move || match compiler() {
                        | Some(Compiler::Bcc) => "Compiled by bcc in your browser",
                        | _ => "Compiled by Clang (wasi-sdk), ahead of time",
                    }}
                </h3>
                <div class="opt-toggle wasm-compiler-switch" role="group" aria-label="Which compiler's WebAssembly to step" hidden=move || !both()>
                    {choice(Compiler::Bcc)}
                    {choice(Compiler::Clang)}
                </div>
            </div>
            <p class="wasm-provenance-body">
                {move || match compiler() {
                    | Some(Compiler::Bcc) => rich_text(
                        "bcc, the C compiler this site is about, is itself compiled to WebAssembly and runs inside this page. \
                         It just turned the C source into the module you are stepping, at `-O0` and for WASI \
                         (`wasm32-wasip1`), on your computer: nothing was sent to a server. Edit the program and \
                         choose Run edited code, and bcc compiles your version.",
                    ),
                    | _ => rich_text(
                        "Clang, from the wasi-sdk toolchain, built this lesson's module when the site was built, at `-O0` \
                         and without a C library. Only the lessons have one.",
                    ),
                }}
                {move || both().then(|| rich_text(
                    " Switch compilers to compare: the same C becomes different instructions, because each compiler \
                     makes its own choices. bcc's module also starts in `$_start`, which its small C library adds.",
                ))}
            </p>
            <div class="wasm-fallback" hidden=move || problem().is_none()>
                <p>{move || problem().map(|problem| problem.headline).unwrap_or_default()}</p>
                <pre class="wasm-diagnostic" class:is-prose=move || problem().is_some_and(|problem| problem.prose)>{move || problem().map(|problem| problem.detail).unwrap_or_default()}</pre>
                <p>"You are stepping Clang's module instead."</p>
                <button type="button" class="button small" hidden=move || !problem().is_some_and(|problem| problem.retry) disabled=move || wasm.locked_tracked() on:click=move |_| on_choose.run(Compiler::Bcc)>
                    "Compile again with bcc"
                </button>
            </div>
            <code class="wasm-command">{command}</code>
        </article>
    }
}

/// What the last instruction did.
#[component]
fn Narration(wasm: WasmState, on_jump: Callback<u64>) -> impl IntoView {
    let narration = Memo::new(move |_| {
        wasm.current
            .with(|current| current.as_ref().map(|current| current.narration.clone()))
    });
    let field = move |read: fn(&crate::wasm::WasmNarration) -> String| {
        move || narration.with(|narration| narration.as_ref().map(read).unwrap_or_default())
    };
    let category = move || {
        narration.with(|narration| {
            narration.as_ref().map_or_else(
                || "control".to_owned(),
                |narration| narration.category.clone(),
            )
        })
    };
    let step = move || {
        wasm.current
            .with(|current| current.as_ref().map(|current| current.step))
    };
    let source = move || {
        wasm.current.with(|current| {
            current
                .as_ref()
                .and_then(|current| current.last.or(current.next))
                .and_then(|place| place.source)
        })
    };
    let offset = move || {
        wasm.current.with(|current| {
            current
                .as_ref()
                .and_then(|current| current.last)
                .map(|place| format!("byte {:#x}", place.offset))
        })
    };
    let breakpoint = move || {
        wasm.current.with(|current| {
            current
                .as_ref()
                .is_some_and(|current| current.stopped.as_deref() == Some("breakpoint"))
        })
    };
    let results = move || {
        wasm.current.with(|current| {
            current
                .as_ref()
                .filter(|current| current.status == "finished")
                .map(|current| {
                    if current.results.is_empty() {
                        "nothing".to_owned()
                    } else {
                        current
                            .results
                            .iter()
                            .map(|value| value.text.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                })
        })
    };
    let exit = move || {
        wasm.current
            .with(|current| current.as_ref().and_then(|current| current.exit))
    };
    let output = move || {
        wasm.current.with(|current| {
            current
                .as_ref()
                .filter(|current| current.status == "trapped")
                .map(|_| ())
        })
    };
    let recent = Memo::new(move |_| wasm.recent());
    view! {
        <article class="card narration wasm-narration" data-category=category aria-labelledby="wasm-narration-title">
            <div class="narration-meta">
                <span class="category-badge">{field(|narration| narration.label.clone())}</span>
                <span class="narration-step">{move || step().map(|step| format!("Instruction {step}")).unwrap_or_default()}</span>
                <code class="wasm-instruction" hidden=move || narration.with(|n| n.as_ref().is_none_or(|n| n.instruction.is_empty()))>
                    {field(|narration| narration.instruction.clone())}
                </code>
                <span class="wasm-offset" hidden=move || offset().is_none()>{offset}</span>
                <span class="wasm-chip is-breakpoint" hidden=move || !breakpoint()>
                    <span class="breakpoint-dot" aria-hidden="true"></span>"Breakpoint"
                </span>
                <span class="wasm-chip" hidden=move || source().is_none()>
                    {move || source().map(|line| format!("C line {line}")).unwrap_or_default()}
                </span>
            </div>
            <h2 id="wasm-narration-title" class="narration-title">
                {move || narration.with(|narration| match narration {
                    | Some(narration) => rich_text(&narration.title),
                    | None => "Loading…".into_any(),
                })}
            </h2>
            <p class="narration-body">
                {move || narration.with(|narration| narration.as_ref().map(|narration| rich_text(&narration.explanation)))}
            </p>
            <p class="return-callout" hidden=move || results().is_none()>
                "Program finished: it returned "<strong>{move || results().unwrap_or_default()}</strong>
            </p>
            <p class="return-callout" hidden=move || exit().is_none()>
                "Program exited with status "<strong>{move || exit().unwrap_or_default()}</strong>
                ", the number it gave WASI's "<code>"proc_exit"</code>": C's exit status, what "<code>"main"</code>" returned or passed to "<code>"exit"</code>"."
            </p>
            <div class="fault-callout" hidden=move || output().is_none()>
                <p class="fault-kind">"Trap"</p>
                <p class="fault-message">"Execution stopped for good. Back and Reset still work, so you can look at the steps that led here."</p>
            </div>
            <ul class="clauses" aria-label="WebAssembly ideas this step uses">
                {move || narration.with(|narration| {
                    narration
                        .as_ref()
                        .map(|narration| narration.concepts.clone())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|concept| view! {
                            <li class="clause">
                                <span class="clause-id" title=concept.citation.clone()>{concept.section}</span>
                                <span class="clause-title">{concept.title}</span>
                                <span class="clause-gloss">{rich_text(&concept.gloss)}</span>
                            </li>
                        })
                        .collect_view()
                })}
            </ul>
            <div class="recent" hidden=move || recent.with(Vec::is_empty)>
                <h3>"Earlier steps"</h3>
                <ol>
                    {move || recent.get().into_iter().map(|(step, title, category)| view! {
                        <li>
                            <button type="button" class="recent-step" data-category=category on:click=move |_| on_jump.run(step)>
                                <span class="recent-dot" aria-hidden="true"></span>
                                <span class="recent-index">{step}</span>
                                <span class="recent-title">{title.replace('`', "")}</span>
                            </button>
                        </li>
                    }).collect_view()}
                </ol>
            </div>
        </article>
    }
}

/// The module's text format, one instruction per line, under the C source.
#[component]
pub fn WasmListing(
    wasm: WasmState,
    theme: Signal<&'static str>,
    pointing: Pointing,
    on_breakpoint: Callback<usize>,
) -> impl IntoView {
    let text = Signal::derive(move || {
        wasm.program.with(|program| {
            program
                .as_ref()
                .map(|program| program.wat.clone())
                .unwrap_or_default()
        })
    });
    let linked_source = move || pointing.line(Side::Source);
    let layers = Signal::derive(move || wasm.listing_marks(linked_source()));
    let reveal = Signal::derive(move || {
        let hovered = linked_source();
        let linked = (hovered > 0)
            .then(|| {
                wasm.program.with(|program| {
                    program
                        .as_ref()
                        .and_then(|program| program.wat_lines_for(hovered).first().copied())
                })
            })
            .flatten();
        linked.or_else(|| wasm.reveal_wat.get())
    });
    let producer = move || {
        wasm.program.with(|program| {
            program
                .as_ref()
                .map_or_else(String::new, |program| format!("{} bytes", program.size))
        })
    };
    let command = move || {
        wasm.program.with(|program| {
            program
                .as_ref()
                .map(|program| program.command.clone())
                .unwrap_or_default()
        })
    };
    let unresolved = move || {
        wasm.program.with(|program| {
            program
                .as_ref()
                .map(|program| program.unresolved.join(", "))
                .unwrap_or_default()
        })
    };
    let has_lines = move || {
        wasm.program
            .with(|program| program.as_ref().is_some_and(|program| program.has_lines))
    };
    view! {
        <div class="wat-pane" hidden=move || wasm.program.with(Option::is_none)>
            <header class="pane-header wat-header">
                <div class="pane-title">
                    <h2 id="wat-title" class="file-chip wat-chip" title=command>
                        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M2.5 3.5h11v9h-11zM5 6.5h6M5 9.5h4" /></svg>
                        "WebAssembly text"
                    </h2>
                    <span class="wat-compiler" title=command data-compiler=move || match wasm.compiler() {
                        | Some(Compiler::Bcc) => "bcc",
                        | _ => "clang",
                    }>
                        {move || match wasm.compiler() {
                            | Some(Compiler::Bcc) => "compiled by bcc",
                            | _ => "compiled by Clang (wasi-sdk)",
                        }}
                    </span>
                    <span class="wat-producer">{producer}</span>
                </div>
            </header>
            <p class="stage-note" hidden=move || unresolved().is_empty()>
                {move || format!("Imports the stepper does not provide (calling one traps): {}", unresolved())}
            </p>
            <div class="wat-editor">
                <CodeEditor
                    text
                    language=Signal::stored("wat")
                    theme
                    label="WebAssembly text format (read only). Click a line number to set a breakpoint."
                    read_only=true
                    decorations=layers
                    on_hover_line=Callback::new(move |line| pointing.hover(Side::Output, line))
                    on_cursor_line=Callback::new(move |line| pointing.cursor(Side::Output, line))
                    on_gutter_click=on_breakpoint
                    reveal
                    follow_on_page=Signal::derive(move || linked_source() == 0)
                />
            </div>
            <footer class="source-legend wat-legend">
                <span><i class="swatch swatch-arrow"></i>"C line running"</span>
                <span><i class="swatch swatch-current"></i>"just executed"</span>
                <span><i class="swatch swatch-next-instruction"></i>"next"</span>
                <span><i class="swatch swatch-breakpoint"></i>"click a line number to break there"</span>
                <span class="legend-hover" hidden=move || !has_lines()><i class="swatch swatch-link"></i>"hover or move the cursor to link C and WebAssembly"</span>
                <span class="legend-tap" hidden=move || !has_lines()><i class="swatch swatch-link"></i>"tap a line to link C and WebAssembly"</span>
            </footer>
        </div>
    }
}
