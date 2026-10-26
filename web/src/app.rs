//! The debugger page: controls, bounded snapshot history, and layout.
//!
//! Every program state comes from the Rust engine in its Worker. Back replays
//! retained snapshots for presentation only; the engine stays at the newest
//! recorded state, and stepping forward through history reaches that point
//! before requesting a new engine step.

use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use leptos::{
    html,
    prelude::*,
    task::spawn_local,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    HtmlInputElement,
    ScrollIntoViewOptions,
    ScrollLogicalPosition,
};

use crate::{
    demos::{
        self,
        DEMOS,
    },
    engine::Engine,
    panels::{
        DiagnosticBanner,
        EffectsPanel,
        ObjectsPanel,
        StackPanel,
    },
    snapshot::{
        Diagnostic,
        Snapshot,
        status_label,
    },
    source_view::{
        self,
        SourceLine,
    },
};

#[component]
pub fn App() -> impl IntoView {
    let state = State::new();
    // Effects run only in the browser, so prerendering never starts a Worker.
    Effect::new(move || state.initialize());
    view! {
        <a class="skip-link" href="#source-panel">"Skip to debugger"</a>
        <Header state />
        <main>
            <Intro />
            <EngineNotice state />
            <section class="debugger" aria-label="C abstract machine debugger">
                <Toolbar state />
                <div class="workspace">
                    <SourcePanel state />
                    <div class="machine-panels">
                        <StackPanel snapshot=state.current />
                        <ObjectsPanel snapshot=state.current />
                    </div>
                </div>
                <EffectsPanel snapshot=state.current />
            </section>
            <DiagnosticBanner diagnostic=state.diagnostic />
            <p class="sr-only" role="status" aria-live="polite" aria-atomic="true">{state.announcement}</p>
        </main>
        <footer class="site-footer">
            <span>"bcc "<span class="muted">"/ experimental C abstract machine"</span></span>
            <span>"Local execution · No external dependencies"</span>
        </footer>
    }
}

const HISTORY_LIMIT: usize = 2000;
const RUN_LIMIT: usize = 1000;
const MAX_SOURCE_BYTES: u32 = 32 * 1024;
/// The site's README, which covers building it and the model's limitations.
const README_URL: &str = "https://github.com/BenBwall/bcc-rust/blob/main/web/README.md";
const RUN_YIELD_MS: u32 = 25;
const CURSOR_HINT: &str = "Highlight: next operation. Select a target line.";

/// Page state. Every field is a `Copy` handle, so closures capture it freely.
#[derive(Clone, Copy)]
struct State {
    engine:          StoredValue<Option<Rc<Engine>>, LocalStorage>,
    has_engine:      RwSignal<bool>,
    source:          RwSignal<String>,
    filename:        RwSignal<String>,
    history:         RwSignal<Vec<Snapshot>>,
    cursor:          RwSignal<Option<usize>>,
    /// Snapshots dropped from the front of `history`.
    first_step:      RwSignal<usize>,
    target_line:     RwSignal<Option<usize>>,
    busy:            RwSignal<bool>,
    running:         RwSignal<bool>,
    pause_requested: RwSignal<bool>,
    editor_text:     RwSignal<String>,
    editing:         RwSignal<bool>,
    failed:          RwSignal<bool>,
    /// A site error shown in place of the snapshot's diagnostic.
    error:           RwSignal<Option<Diagnostic>>,
    /// Why the engine is unavailable; shows setup instructions.
    unavailable:     RwSignal<Option<String>>,
    engine_label:    RwSignal<&'static str>,
    cursor_label:    RwSignal<String>,
    announcement:    RwSignal<String>,
    current:         Memo<Option<Snapshot>>,
    diagnostic:      Memo<Option<Diagnostic>>,
    edited:          Memo<bool>,
    code_view:       NodeRef<html::Div>,
    editor:          NodeRef<html::Textarea>,
}

impl State {
    fn new() -> Self {
        let tour = &DEMOS[0];
        let history = RwSignal::new(Vec::<Snapshot>::new());
        let cursor = RwSignal::new(None::<usize>);
        let source = RwSignal::new(tour.source.to_owned());
        let editor_text = RwSignal::new(tour.source.to_owned());
        let error = RwSignal::new(None::<Diagnostic>);
        let current = Memo::new(move |_| {
            cursor
                .get()
                .and_then(|index| history.with(|history| history.get(index).cloned()))
        });
        Self {
            engine: StoredValue::new_local(None),
            has_engine: RwSignal::new(false),
            source,
            filename: RwSignal::new(tour.name.to_owned()),
            history,
            cursor,
            first_step: RwSignal::new(0),
            target_line: RwSignal::new(None),
            busy: RwSignal::new(false),
            running: RwSignal::new(false),
            pause_requested: RwSignal::new(false),
            editor_text,
            editing: RwSignal::new(false),
            failed: RwSignal::new(false),
            error,
            unavailable: RwSignal::new(None),
            engine_label: RwSignal::new("Connecting to Rust / Wasm"),
            cursor_label: RwSignal::new(CURSOR_HINT.into()),
            announcement: RwSignal::new(String::new()),
            current,
            diagnostic: Memo::new(move |_| {
                error.get().or_else(|| {
                    current.with(|state| state.as_ref().and_then(|state| state.diagnostic.clone()))
                })
            }),
            edited: Memo::new(move |_| {
                editor_text.with(|text| source.with(|source| text != source))
            }),
            code_view: NodeRef::new(),
            editor: NodeRef::new(),
        }
    }

    /// Start (or restart) the engine Worker and load the current program.
    fn initialize(self) {
        spawn_local(async move {
            self.busy.set(true);
            self.engine_label.set("Connecting to Rust / Wasm");
            self.drop_engine();
            match Engine::connect().await {
                | Ok(engine) => {
                    self.engine.set_value(Some(Rc::new(engine)));
                    self.has_engine.set(true);
                    self.unavailable.set(None);
                    self.engine_label.set("Rust / Wasm · bounded C subset");
                    let text = if self.edited.get_untracked() {
                        self.editor_text.get_untracked()
                    } else {
                        self.source.get_untracked()
                    };
                    if let Err(error) = self.load_program(text, self.filename.get_untracked()).await
                    {
                        if self.engine_connected() {
                            self.show_error(&error);
                        } else {
                            self.drop_engine();
                            self.show_unavailable(&error);
                        }
                    }
                },
                | Err(error) => self.show_unavailable(&error),
            }
            self.busy.set(false);
        });
    }

    /// Run `operation` unless another one is in progress, reporting its error.
    fn action<F>(self, operation: impl FnOnce() -> F + 'static)
    where
        F: Future<Output = Result<(), String>> + 'static,
    {
        if self.locked() {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            if let Err(error) = operation().await {
                self.show_error(&error);
            }
            self.busy.set(false);
        });
    }

    async fn load_program(self, text: String, name: String) -> Result<(), String> {
        if text.len() > MAX_SOURCE_BYTES as usize {
            return Err("The bounded Rust engine accepts at most 32 KiB of source.".into());
        }
        self.source.set(text.clone());
        self.editor_text.set(text.clone());
        self.filename.set(name);
        self.clear_history();
        self.failed.set(false);
        self.show_state(false);
        let snapshot = self.engine()?.load(&text).await?;
        self.history.set(vec![snapshot]);
        self.cursor.set(Some(0));
        self.show_state(true);
        Ok(())
    }

    /// Select a demo or opened file: load it, or stage it while the engine is
    /// unavailable.
    async fn stage_program(self, text: String, name: String) -> Result<(), String> {
        let result = if self.has_engine.get_untracked() {
            self.load_program(text, name).await
        } else {
            self.source.set(text.clone());
            self.editor_text.set(text);
            self.filename.set(name);
            self.clear_history();
            self.show_state(false);
            Ok(())
        };
        self.editing.set(false);
        result
    }

    async fn step_forward(self) -> Result<(), String> {
        let cursor = self.cursor.get_untracked().unwrap_or(0);
        if cursor + 1 < self.history.with_untracked(Vec::len) {
            self.cursor.set(Some(cursor + 1));
            return Ok(());
        }
        let snapshot = self.engine()?.step().await?;
        let mut index = cursor + 1;
        self.history.update(|history| {
            history.push(snapshot);
            if history.len() > HISTORY_LIMIT {
                _ = history.remove(0);
                index -= 1;
                self.first_step.update(|first| *first += 1);
            }
        });
        self.cursor.set(Some(index));
        Ok(())
    }

    /// Step until a terminal state, Pause, the Run budget, or `line`.
    fn run(self, line: Option<usize>) {
        if self.locked() || !self.can_step_untracked() {
            return;
        }
        self.running.set(true);
        self.pause_requested.set(false);
        spawn_local(async move {
            let mut steps = 0;
            let mut reached = false;
            let mut result = Ok(());
            // Yield between real Rust steps so Pause stays available and large
            // loops do not lock the page. The budget also bounds accidental
            // infinite loops.
            while !self.pause_requested.get_untracked()
                && self.can_step_untracked()
                && steps < RUN_LIMIT
            {
                result = self.step_forward().await;
                if result.is_err() {
                    break;
                }
                steps += 1;
                self.show_state(false);
                reached = line.is_some_and(|line| self.current_contains(line));
                if reached {
                    break;
                }
                TimeoutFuture::new(RUN_YIELD_MS).await;
            }
            match result {
                | Err(error) => self.show_error(&error),
                | Ok(()) if steps == RUN_LIMIT && self.can_step_untracked() => {
                    self.report(format!(
                        "Paused after {RUN_LIMIT} operations. Run again to continue."
                    ));
                },
                | Ok(()) => match line {
                    | Some(line) if reached => self.report(format!("Reached line {line}.")),
                    | Some(line) if self.can_step_untracked() => {
                        self.report(format!("Paused before reaching line {line}."));
                    },
                    | Some(line) => {
                        let status = self.current.with_untracked(|state| {
                            state
                                .as_ref()
                                .map(|state| state.status.replace('_', " "))
                                .unwrap_or_default()
                        });
                        self.report(format!("Stopped before reaching line {line}: {status}."));
                    },
                    | None => self.announce_state(),
                },
            }
            self.running.set(false);
            self.reveal_current_line();
        });
    }

    fn back(self) {
        if self.locked() || self.edited.get_untracked() {
            return;
        }
        if let Some(cursor) = self.cursor.get_untracked().filter(|&cursor| cursor > 0) {
            self.cursor.set(Some(cursor - 1));
            self.show_state(true);
        }
    }

    fn select_line(self, line: usize) {
        self.target_line.set(Some(line));
        self.cursor_label.set(format!("Target set to line {line}."));
    }

    async fn open_file(self, input: HtmlInputElement) -> Result<(), String> {
        let Some(file) = input.files().and_then(|files| files.get(0)) else {
            return Ok(());
        };
        if file.size() > f64::from(MAX_SOURCE_BYTES) {
            return Err("The bounded Rust engine accepts source files up to 32 KiB.".into());
        }
        let text = JsFuture::from(file.text())
            .await
            .ok()
            .and_then(|text| text.as_string())
            .ok_or("The selected file could not be read as text.")?;
        let result = self.stage_program(text, file.name()).await;
        input.set_value("");
        result
    }

    fn set_editing(self, editing: bool) {
        self.editing.set(editing);
        if editing {
            let editor = self.editor;
            request_animation_frame(move || {
                if let Some(editor) = editor.get_untracked() {
                    _ = editor.focus();
                }
            });
        }
    }

    fn engine(self) -> Result<Rc<Engine>, String> {
        self.engine.with_value(Clone::clone).ok_or_else(|| {
            "Rust engine disconnected. Retry engine to start a fresh instance.".into()
        })
    }

    fn clear_history(self) {
        self.history.set(Vec::new());
        self.cursor.set(None);
        self.first_step.set(0);
        self.target_line.set(None);
        self.cursor_label.set(CURSOR_HINT.into());
    }

    fn show_state(self, reveal: bool) {
        self.error.set(None);
        if reveal {
            self.reveal_current_line();
            self.announce_state();
        }
    }

    fn show_error(self, error: &str) {
        self.failed.set(true);
        if self.has_engine.get_untracked() && !self.engine_connected() {
            self.drop_engine();
            self.show_unavailable(error);
        }
        self.error.set(Some(Diagnostic {
            kind:     None,
            title:    "Unable to continue".into(),
            message:  error.into(),
            citation: "Reset or load a program to restart from a known engine state.".into(),
            detail:   String::new(),
        }));
        self.announcement.set(error.into());
    }

    fn show_unavailable(self, error: &str) {
        self.unavailable.set(Some(error.into()));
        self.engine_label.set("Rust / Wasm unavailable");
        self.announcement
            .set("Rust WebAssembly engine unavailable. Build instructions are displayed.".into());
    }

    fn drop_engine(self) {
        if let Some(engine) = self.engine.try_update_value(Option::take).flatten() {
            engine.dispose("Rust engine stopped.");
        }
        self.has_engine.set(false);
    }

    fn report(self, text: String) {
        self.cursor_label.set(text.clone());
        self.announcement.set(text);
    }

    fn announce_state(self) {
        let text = self.current.with_untracked(|state| {
            state.as_ref().map(|state| {
                let mut text = format!(
                    "Step {}. {}. {}",
                    state.step,
                    state.status.replace('_', " "),
                    state.event
                );
                if let Some(diagnostic) = &state.diagnostic {
                    text.push(' ');
                    text.push_str(&diagnostic.message);
                }
                text
            })
        });
        if let Some(text) = text {
            self.announcement.set(text);
        }
    }

    fn reveal_current_line(self) {
        let Some(line) = self
            .current
            .with_untracked(|state| state.as_ref()?.location.map(|range| range.start.line))
        else {
            return;
        };
        let code_view = self.code_view;
        // Wait for the view to apply the new snapshot.
        request_animation_frame(move || {
            let Some(container) = code_view.get_untracked() else {
                return;
            };
            if let Ok(Some(row)) =
                container.query_selector(&format!(".code-line[data-line=\"{line}\"]"))
            {
                let options = ScrollIntoViewOptions::new();
                options.set_block(ScrollLogicalPosition::Nearest);
                options.set_inline(ScrollLogicalPosition::Nearest);
                row.scroll_into_view_with_scroll_into_view_options(&options);
            }
        });
    }

    fn current_contains(self, line: usize) -> bool {
        self.current.with_untracked(|state| {
            state
                .as_ref()
                .and_then(|state| state.location)
                .is_some_and(|range| range.contains_line(line))
        })
    }

    fn engine_connected(self) -> bool {
        self.engine
            .with_value(|engine| engine.as_ref().is_some_and(|engine| engine.connected()))
    }

    fn locked(self) -> bool {
        self.busy.get_untracked() || self.running.get_untracked()
    }

    /// Tracked: whether Step and Run can request another evaluation.
    fn can_step(self) -> bool {
        self.has_engine.get()
            && !self.edited.get()
            && !self.failed.get()
            && self
                .current
                .with(|state| state.as_ref().is_some_and(Snapshot::can_step))
            && self.engine_connected()
    }

    fn can_step_untracked(self) -> bool {
        untrack(move || self.can_step())
    }

    fn locked_tracked(self) -> bool {
        self.busy.get() || self.running.get()
    }
}

#[component]
fn Header(state: State) -> impl IntoView {
    view! {
        <header class="site-header">
            <div class="brand">
                <span class="brand-mark" aria-hidden="true">"b>"</span>
                <div>
                    <span class="wordmark">"bcc"</span>
                    <span class="brand-divider" aria-hidden="true">"/"</span>
                    <h1>"C abstract machine"</h1>
                </div>
            </div>
            <div class="header-meta">
                <span class="tag">"C99"</span>
                <span class="engine-indicator">{state.engine_label}</span>
            </div>
        </header>
    }
}

#[component]
fn Intro() -> impl IntoView {
    view! {
        <div class="intro">
            <div>
                <p class="eyebrow">"INSIDE THE EXECUTION"</p>
                <h2>"Every object. Every effect."</h2>
                <p>"Explore a bounded C subset, one Rust engine operation at a time."</p>
            </div>
            <a class="quiet-link" href=README_URL>"Build & limitations "<span aria-hidden="true">"↗"</span></a>
        </div>
        <details class="model-note">
            <summary>"Prototype model "<span>"Independent C subset · 32-bit int · left-to-right evaluation"</span></summary>
            <p>
                "This Rust evaluator is independent of bcc’s full C frontend. It supports local integers, pointers, fixed \
                 arrays, named calls, and basic control flow. It has no preprocessing, globals, library I/O, or allocation. \
                 Nested side effects are rejected. Stores are atomic; deferred effects and full C99 sequencing checks are \
                 not implemented. This prototype starts automatic lifetimes at declarations; C99 fixed objects instead \
                 begin lifetime at block entry. A completed run does not establish that a program is free of undefined \
                 behavior."
            </p>
        </details>
    }
}

#[component]
fn EngineNotice(state: State) -> impl IntoView {
    view! {
        <section class="notice" aria-labelledby="notice-title" hidden=move || state.unavailable.with(Option::is_none)>
            <div class="notice-icon" aria-hidden="true">"!"</div>
            <div>
                <h2 id="notice-title">"Rust / Wasm engine unavailable"</h2>
                <p>{move || state.unavailable.get().unwrap_or_default()}</p>
                <p>"The engine needs a browser with Web Workers and WebAssembly. Retry it, or reload the page if it keeps failing."</p>
                <button type="button" class="small-button" disabled=move || state.busy.get() on:click=move |_| state.initialize()>
                    "Retry engine"
                </button>
            </div>
        </section>
    }
}

#[component]
fn Toolbar(state: State) -> impl IntoView {
    let run_label = move || match (state.running.get(), state.pause_requested.get()) {
        | (true, true) => "Pausing…",
        | (true, false) => "Pause",
        | (false, _) => "Run",
    };
    let status = move || {
        state.current.with(|snapshot| match snapshot {
            | None => "Not loaded".to_owned(),
            | Some(snapshot)
                if snapshot.diagnostic.as_ref().and_then(|d| d.kind.as_deref())
                    == Some("indeterminate") =>
                "Indeterminate read".to_owned(),
            | Some(snapshot) => status_label(&snapshot.status),
        })
    };
    view! {
        <div class="toolbar">
            <div class="transport" role="group" aria-label="Execution controls">
                <button
                    type="button"
                    title="Reset to the initial state"
                    disabled=move || state.locked_tracked() || !state.has_engine.get()
                    on:click=move |_| state.action(move || state.load_program(state.source.get_untracked(), state.filename.get_untracked()))
                >
                    <span aria-hidden="true">"↺"</span>" Reset"
                </button>
                <button
                    type="button"
                    title=move || {
                        if state.first_step.get() > 0 {
                            format!("Back through the most recent {HISTORY_LIMIT} snapshots")
                        } else {
                            "Back one engine snapshot".into()
                        }
                    }
                    disabled=move || state.locked_tracked() || state.edited.get() || state.cursor.get().is_none_or(|cursor| cursor == 0)
                    on:click=move |_| state.back()
                >
                    <span aria-hidden="true">"←"</span>" Back"
                </button>
                <button
                    class="primary"
                    type="button"
                    title="Step one evaluation"
                    disabled=move || state.locked_tracked() || !state.can_step()
                    on:click=move |_| state.action(move || async move {
                        state.step_forward().await?;
                        state.show_state(true);
                        Ok(())
                    })
                >
                    <span aria-hidden="true">"↳"</span>" Step"
                </button>
                <button
                    type="button"
                    title=move || if state.running.get() { "Pause execution" } else { "Run until completion or undefined behavior" }
                    aria-pressed=move || state.running.get().to_string()
                    disabled=move || {
                        state.busy.get() || (state.running.get() && state.pause_requested.get()) || (!state.running.get() && !state.can_step())
                    }
                    on:click=move |_| if state.running.get_untracked() { state.pause_requested.set(true) } else { state.run(None) }
                >
                    <span aria-hidden="true">"▷"</span>" "<span>{run_label}</span>
                </button>
            </div>
            <div class="execution-meta">
                <span class="status-chip" data-status=move || state.current.with(|s| s.as_ref().map_or_else(|| "idle".into(), |s| s.status.clone()))>
                    {status}
                </span>
                <span class="step-counter">
                    "STEP "<strong>{move || state.current.with(|s| s.as_ref().map_or_else(|| "—".into(), |s| s.step.to_string()))}</strong>
                </span>
            </div>
        </div>
    }
}

#[component]
fn SourcePanel(state: State) -> impl IntoView {
    let lines = Memo::new(move |_| state.source.with(|source| source_view::lines(source)));
    let location = Memo::new(move |_| {
        state
            .current
            .with(|snapshot| snapshot.as_ref().and_then(|snapshot| snapshot.location))
    });
    let selected_demo = move || {
        state.source.with(|source| {
            DEMOS
                .iter()
                .find(|demo| demo.source == source)
                .map_or("custom", |demo| demo.key)
        })
    };
    let on_select = Callback::new(move |line| state.select_line(line));
    let target = state.target_line.read_only();
    // `autocorrect` is a Safari attribute with no typed Leptos builder.
    Effect::new(move || {
        if let Some(editor) = state.editor.get() {
            _ = editor.set_attribute("autocorrect", "off");
        }
    });
    view! {
        <section class="source-panel" id="source-panel" tabindex="-1" aria-labelledby="source-title">
            <div class="panel-heading source-heading">
                <div><span class="panel-index">"01"</span><h2 id="source-title">"Source"</h2></div>
                <span class="mono muted">{state.filename}</span>
            </div>
            <div class="program-tools">
                <label class="sr-only" for="demo-select">"Choose a program"</label>
                <select
                    id="demo-select"
                    prop:value=selected_demo
                    disabled=move || state.locked_tracked()
                    on:change=move |event| {
                        if let Some(demo) = demos::find(&event_target_value(&event)) {
                            state.action(move || state.stage_program(demo.source.into(), demo.name.into()));
                        }
                    }
                >
                    {DEMOS.iter().map(|demo| view! { <option value=demo.key>{demo.label}</option> }).collect_view()}
                    <option value="custom" hidden>"Custom program"</option>
                </select>
                <label class="file-button" for="file-input">
                    "Open .c"
                    <input
                        id="file-input"
                        type="file"
                        accept=".c,.h,text/plain,text/x-c"
                        disabled=move || state.locked_tracked()
                        on:change=move |event| {
                            let input: HtmlInputElement = event.target().expect("change events have a target").unchecked_into();
                            state.action(move || state.open_file(input));
                        }
                    />
                </label>
            </div>
            <div class="source-tabs" role="group" aria-label="Source view">
                <button type="button" aria-pressed=move || (!state.editing.get()).to_string() on:click=move |_| state.set_editing(false)>
                    "Execution"
                </button>
                <button type="button" aria-pressed=move || state.editing.get().to_string() on:click=move |_| state.set_editing(true)>
                    "Edit source"
                </button>
                <span class="mono muted">
                    {move || location.get().map_or_else(|| "No current evaluation".into(), |range| format!("Ln {}, Col {}", range.start.line, range.start.column))}
                </span>
            </div>
            <div
                class="code-view"
                role="region"
                tabindex="0"
                aria-label="Source code; select a line to run to it"
                hidden=move || state.editing.get()
                node_ref=state.code_view
            >
                <For each=move || lines.get() key=Clone::clone children=move |line| view! { <SourceLine line location target on_select /> } />
            </div>
            <div class="editor-view" hidden=move || !state.editing.get()>
                <label class="sr-only" for="source-editor">"C source code"</label>
                <textarea
                    id="source-editor"
                    spellcheck="false"
                    autocapitalize="off"
                    autocomplete="off"
                    wrap="off"
                    aria-describedby="editor-help"
                    node_ref=state.editor
                    prop:value=move || state.editor_text.get()
                    disabled=move || state.locked_tracked()
                    on:input=move |event| state.editor_text.set(event_target_value(&event))
                ></textarea>
                <div class="editor-footer">
                    <span id="editor-help">
                        {move || if state.edited.get() { "Source changed. Load program to execute your edits." } else { "Load edits to restart execution." }}
                    </span>
                    <button
                        type="button"
                        class="primary"
                        disabled=move || state.locked_tracked() || !state.has_engine.get()
                        on:click=move |_| state.action(move || async move {
                            state.load_program(state.editor_text.get_untracked(), state.filename.get_untracked()).await?;
                            state.set_editing(false);
                            Ok(())
                        })
                    >
                        "Load program"
                    </button>
                </div>
            </div>
            <div class="source-footer">
                <span>{state.cursor_label}</span>
                <button
                    type="button"
                    class="small-button"
                    disabled=move || state.locked_tracked() || !state.can_step() || state.target_line.get().is_none()
                    on:click=move |_| state.run(state.target_line.get_untracked())
                >
                    "Run to line "{move || state.target_line.get().map_or_else(|| "—".into(), |line| line.to_string())}" "
                    <span aria-hidden="true">"→"</span>
                </button>
            </div>
        </section>
    }
}
