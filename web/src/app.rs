//! The page: lesson picker, source editor, and the two views of a program,
//! the abstract machine stepping it and the compiler pipeline translating it.
//!
//! Every program state comes from the Rust engine in its Worker. Back replays
//! retained snapshots for presentation only; the engine stays at the newest
//! recorded state, and stepping forward through history reaches that point
//! before requesting a new engine step.

use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use leptos::{
    ev,
    prelude::*,
    task::spawn_local,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlInputElement;

use crate::{
    demos::{
        self,
        DEMOS,
        Demo,
    },
    editor::{
        CodeEditor,
        Decoration,
        Layer,
    },
    engine::Engine,
    palette,
    panels::{
        Memory,
        Narration,
        PipelineView,
        TreeView,
    },
    pipeline::{
        Compilation,
        Stage,
    },
    snapshot::{
        Diagnostic,
        Range,
        Snapshot,
        status_label,
    },
};

#[component]
pub fn App() -> impl IntoView {
    let state = State::new();
    // Effects run only in the browser, so prerendering never starts a Worker.
    Effect::new(move || state.initialize());
    Effect::new(move |previous: Option<()>| {
        if previous.is_none() {
            state.theme.set(initial_theme());
        }
    });
    Effect::new(move || {
        let theme = state.theme.get();
        if let Some(root) = document().document_element() {
            _ = root.set_attribute("data-theme", theme);
        }
    });
    Effect::new(move || {
        // The page lives as long as the listener.
        let _handle = window_event_listener(ev::keydown, move |event| state.shortcut(&event));
    });
    view! {
        <a class="skip-link" href="#workbench">"Skip to the workbench"</a>
        <Header state />
        <main>
            <Hero />
            <Lessons state />
            <EngineNotice state />
            <section class="workbench" id="workbench" aria-label="Workbench">
                <Toolbar state />
                <SiteError state />
                <div class="workbench-grid">
                    <SourcePane state />
                    <div class="insight" id="insight">
                        <Show
                            when=move || state.mode.get() == Mode::Machine
                            fallback=move || view! {
                                <PipelineView
                                    stage=state.stage
                                    compilation=state.compilation
                                    edited=state.edited
                                    live_error=state.live_error.read_only()
                                    theme=state.theme_name
                                    hover_source=state.hover_source.read_only()
                                    hover_output=state.hover_output
                                    filename=state.filename.read_only()
                                    source=state.editor_text.into()
                                />
                            }
                        >
                            <Narration snapshot=state.current recent=state.recent on_jump=Callback::new(move |index| state.jump(index)) placeholder=state.placeholder />
                            <TreeView snapshot=state.current />
                            <Memory snapshot=state.current />
                        </Show>
                    </div>
                </div>
            </section>
            <About />
            <p class="sr-only" role="status" aria-live="polite" aria-atomic="true">{state.announcement}</p>
        </main>
        <footer class="site-footer">
            <span><strong>"bcc"</strong>" · a C99 compiler written in Rust"</span>
            <span>"Everything runs in your browser. No code leaves this page."</span>
            <a href=REPOSITORY_URL>"Source on GitHub"</a>
        </footer>
    }
}

const HISTORY_LIMIT: usize = 2000;
const RUN_LIMIT: usize = 1000;
const MAX_SOURCE_BYTES: u32 = 32 * 1024;
const REPOSITORY_URL: &str = "https://github.com/BenBwall/bcc-rust";
const README_URL: &str = "https://github.com/BenBwall/bcc-rust/blob/main/web/README.md";
const RUN_YIELD_MS: u32 = 140;
/// Steps a held key may queue while the engine answers.
const MAX_QUEUED_STEPS: u32 = 4;
const THEME_KEY: &str = "bcc-theme";

/// Which view of the program the workbench shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Machine,
    Pipeline,
}

/// Page state. Every field is a `Copy` handle, so closures capture it freely.
#[derive(Clone, Copy)]
struct State {
    engine:          StoredValue<Option<Rc<Engine>>, LocalStorage>,
    has_engine:      RwSignal<bool>,
    /// The program the engine runs.
    source:          RwSignal<String>,
    /// The editor's text, which differs from `source` after an edit.
    editor_text:     RwSignal<String>,
    filename:        RwSignal<String>,
    history:         RwSignal<Vec<Snapshot>>,
    cursor:          RwSignal<Option<usize>>,
    /// Snapshots dropped from the front of `history`.
    first_step:      RwSignal<usize>,
    target_line:     RwSignal<Option<usize>>,
    busy:            RwSignal<bool>,
    running:         RwSignal<bool>,
    /// Steps requested (by key repeat) while one was in flight.
    queued:          RwSignal<u32>,
    pause_requested: RwSignal<bool>,
    failed:          RwSignal<bool>,
    /// A site error shown in place of the snapshot's diagnostic.
    error:           RwSignal<Option<Diagnostic>>,
    /// Why the engine is unavailable.
    unavailable:     RwSignal<Option<String>>,
    announcement:    RwSignal<String>,
    mode:            RwSignal<Mode>,
    stage:           RwSignal<Stage>,
    theme:           RwSignal<&'static str>,
    theme_name:      Signal<&'static str>,
    /// The compiler's output for the editor's text, when known.
    live:            RwSignal<Option<(String, Compilation)>>,
    live_error:      RwSignal<Option<String>>,
    hover_source:    RwSignal<usize>,
    hover_output:    RwSignal<usize>,
    reveal:          RwSignal<Option<usize>>,
    current:         Memo<Option<Snapshot>>,
    recent:          Memo<Vec<(usize, String, String)>>,
    edited:          Memo<bool>,
    compilation:     Memo<Option<Compilation>>,
    placeholder:     Memo<&'static str>,
}

impl State {
    fn new() -> Self {
        let tour = &DEMOS[0];
        let history = RwSignal::new(Vec::<Snapshot>::new());
        let cursor = RwSignal::new(None::<usize>);
        let source = RwSignal::new(tour.source.to_owned());
        let editor_text = RwSignal::new(tour.source.to_owned());
        let error = RwSignal::new(None::<Diagnostic>);
        let live = RwSignal::new(None::<(String, Compilation)>);
        let theme = RwSignal::new("dark");
        let current = Memo::new(move |_| {
            cursor
                .get()
                .and_then(|index| history.with(|history| history.get(index).cloned()))
        });
        let recent = Memo::new(move |_| {
            let Some(cursor) = cursor.get() else {
                return Vec::new();
            };
            history.with(|history| {
                history[..cursor]
                    .iter()
                    .enumerate()
                    .rev()
                    .take(4)
                    .map(|(index, snapshot)| {
                        (
                            index,
                            snapshot.event.title.clone(),
                            snapshot.event.category.clone(),
                        )
                    })
                    .collect()
            })
        });
        Self {
            engine: StoredValue::new_local(None),
            has_engine: RwSignal::new(false),
            source,
            editor_text,
            filename: RwSignal::new(tour.name.to_owned()),
            history,
            cursor,
            first_step: RwSignal::new(0),
            target_line: RwSignal::new(None),
            busy: RwSignal::new(false),
            running: RwSignal::new(false),
            queued: RwSignal::new(0),
            pause_requested: RwSignal::new(false),
            failed: RwSignal::new(false),
            error,
            unavailable: RwSignal::new(None),
            announcement: RwSignal::new(String::new()),
            mode: RwSignal::new(Mode::Machine),
            stage: RwSignal::new(Stage::Ir),
            theme,
            theme_name: Signal::derive(move || theme.get()),
            live,
            live_error: RwSignal::new(None),
            hover_source: RwSignal::new(0),
            hover_output: RwSignal::new(0),
            reveal: RwSignal::new(None),
            current,
            recent,
            edited: Memo::new(move |_| {
                editor_text.with(|text| source.with(|source| text != source))
            }),
            compilation: Memo::new(move |_| {
                editor_text.with(|text| {
                    if let Some(demo) = demos::matching(text) {
                        return crate::pipeline::precomputed(demo.name.trim_end_matches(".c"));
                    }
                    live.with(|live| {
                        live.as_ref()
                            .filter(|(source, _)| source == text)
                            .map(|(_, compilation)| compilation.clone())
                    })
                })
            }),
            placeholder: Memo::new(move |_| {
                editor_text.with(|text| demos::matching(text).map_or("", |demo| demo.summary))
            }),
        }
    }

    /// Start (or restart) the engine Worker and load the current program.
    fn initialize(self) {
        spawn_local(async move {
            self.busy.set(true);
            self.drop_engine();
            match Engine::connect().await {
                | Ok(engine) => {
                    self.engine.set_value(Some(Rc::new(engine)));
                    self.has_engine.set(true);
                    self.unavailable.set(None);
                    let text = self.source.get_untracked();
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
        if !self.accepts(&text) {
            return Ok(());
        }
        self.source.set(text.clone());
        self.editor_text.set(text.clone());
        self.filename.set(name);
        self.clear_history();
        self.failed.set(false);
        self.error.set(None);
        let snapshot = self.engine()?.load(&text).await?;
        self.history.set(vec![snapshot]);
        self.cursor.set(Some(0));
        self.announce_state();
        Ok(())
    }

    /// Select a demo or an opened file: load it, or stage it while the engine
    /// is unavailable.
    async fn stage_program(self, text: String, name: String) -> Result<(), String> {
        if self.has_engine.get_untracked() {
            self.load_program(text, name).await
        } else if self.accepts(&text) {
            self.source.set(text.clone());
            self.editor_text.set(text);
            self.filename.set(name);
            self.clear_history();
            Ok(())
        } else {
            Ok(())
        }
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

    fn step(self) {
        if self.running.get_untracked() || !self.can_step_untracked() {
            return;
        }
        if self.busy.get_untracked() {
            self.queued
                .update(|queued| *queued = (*queued + 1).min(MAX_QUEUED_STEPS));
            return;
        }
        self.action(move || async move {
            let result = async {
                loop {
                    self.step_forward().await?;
                    self.after_step();
                    if self.queued.get_untracked() == 0 || !self.can_step_untracked() {
                        return Ok(());
                    }
                    self.queued.update(|queued| *queued -= 1);
                }
            }
            .await;
            self.queued.set(0);
            result
        });
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
            // Yield between real Rust steps so the animation is visible, Pause
            // stays available, and large loops do not lock the page. The
            // budget also bounds accidental infinite loops.
            while !self.pause_requested.get_untracked()
                && self.can_step_untracked()
                && steps < RUN_LIMIT
            {
                result = self.step_forward().await;
                if result.is_err() {
                    break;
                }
                steps += 1;
                self.reveal_current();
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
                    | _ => self.announce_state(),
                },
            }
            self.running.set(false);
        });
    }

    fn back(self) {
        if self.locked() || self.edited.get_untracked() {
            return;
        }
        if let Some(cursor) = self.cursor.get_untracked().filter(|&cursor| cursor > 0) {
            self.cursor.set(Some(cursor - 1));
            self.after_step();
        }
    }

    /// Show retained snapshot `index` (from the recent-steps list).
    fn jump(self, index: usize) {
        if self.locked() || index >= self.history.with_untracked(Vec::len) {
            return;
        }
        self.cursor.set(Some(index));
        self.after_step();
    }

    fn reset(self) {
        self.action(move || {
            self.load_program(self.source.get_untracked(), self.filename.get_untracked())
        });
    }

    fn toggle_run(self) {
        if self.running.get_untracked() {
            self.pause_requested.set(true);
        } else {
            self.run(None);
        }
    }

    fn select_demo(self, demo: &'static Demo) {
        self.mode.set(Mode::Machine);
        self.action(move || self.stage_program(demo.source.into(), demo.name.into()));
    }

    /// Load the editor's text into the engine (and the compiler, when live).
    fn run_edits(self) {
        let text = self.editor_text.get_untracked();
        let name = if demos::matching(&text).is_some() {
            self.filename.get_untracked()
        } else {
            "program.c".into()
        };
        self.action(move || self.load_program(text, name));
    }

    fn set_mode(self, mode: Mode) {
        self.mode.set(mode);
        if mode == Mode::Pipeline {
            self.request_compilation();
        }
    }

    /// Ask the Worker to compile edited source for the pipeline view.
    fn request_compilation(self) {
        let text = self.editor_text.get_untracked();
        if demos::matching(&text).is_some()
            || self
                .live
                .with_untracked(|live| live.as_ref().is_some_and(|(source, _)| *source == text))
        {
            return;
        }
        let Ok(engine) = self.engine() else {
            return;
        };
        spawn_local(async move {
            match engine.compile(&text).await {
                | Ok(compilation) => {
                    self.live_error.set(None);
                    self.live.set(Some((text, compilation)));
                },
                | Err(error) => self.live_error.set(Some(error)),
            }
        });
    }

    fn set_target(self, line: usize) {
        if line == 0 {
            return;
        }
        let target = (self.target_line.get_untracked() != Some(line)).then_some(line);
        self.target_line.set(target);
        self.announcement.set(target.map_or_else(
            || "Run target cleared.".to_owned(),
            |line| format!("Run target set to line {line}. Choose Run to line."),
        ));
    }

    async fn open_file(self, input: HtmlInputElement) -> Result<(), String> {
        let Some(file) = input.files().and_then(|files| files.get(0)) else {
            return Ok(());
        };
        // Clear the input so that choosing the same file again reopens it.
        input.set_value("");
        if file.size() > f64::from(MAX_SOURCE_BYTES) {
            self.reject(format!(
                "{} is {} KiB, but the bounded Rust engine accepts source files up to 32 KiB.",
                file.name(),
                (file.size() / 1024.0).ceil()
            ));
            return Ok(());
        }
        let Some(text) = JsFuture::from(file.text())
            .await
            .ok()
            .and_then(|text| text.as_string())
        else {
            self.reject(format!("{} could not be read as text.", file.name()));
            return Ok(());
        };
        self.stage_program(text, file.name()).await
    }

    fn shortcut(self, event: &web_sys::KeyboardEvent) {
        if event.ctrl_key() || event.meta_key() || event.alt_key() || event.default_prevented() {
            return;
        }
        let typing = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|element| {
                element
                    .closest("input, textarea, select, .monaco-editor, [contenteditable]")
                    .ok()
                    .flatten()
                    .is_some()
            });
        if typing || self.mode.get_untracked() != Mode::Machine {
            return;
        }
        match event.key().as_str() {
            | "ArrowRight" | "j" => self.step(),
            | "ArrowLeft" | "k" => self.back(),
            | "r" => self.toggle_run(),
            | "R" => self.reset(),
            | _ => return,
        }
        event.prevent_default();
    }

    fn toggle_theme(self) {
        let next = if self.theme.get_untracked() == "dark" {
            "light"
        } else {
            "dark"
        };
        self.theme.set(next);
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            _ = storage.set_item(THEME_KEY, next);
        }
    }

    fn engine(self) -> Result<Rc<Engine>, String> {
        self.engine.with_value(Clone::clone).ok_or_else(|| {
            "Rust engine disconnected. Retry the engine to start a fresh instance.".into()
        })
    }

    fn clear_history(self) {
        self.history.set(Vec::new());
        self.cursor.set(None);
        self.first_step.set(0);
        self.target_line.set(None);
    }

    fn after_step(self) {
        self.error.set(None);
        self.reveal_current();
        self.announce_state();
    }

    fn reveal_current(self) {
        let line = self.current.with_untracked(|state| {
            state
                .as_ref()
                .and_then(|state| state.event.range.or(state.location))
                .map(|range| range.start.line)
        });
        self.reveal.set(line);
    }

    fn show_error(self, error: &str) {
        self.failed.set(true);
        if self.has_engine.get_untracked() && !self.engine_connected() {
            self.drop_engine();
            self.show_unavailable(error);
        }
        self.error.set(Some(Diagnostic {
            kind:    None,
            title:   "Unable to continue".into(),
            message: error.into(),
            clause:  None,
            note:    "Reset or load a program to restart from a known engine state.".into(),
        }));
        self.announcement.set(error.into());
    }

    /// Whether `text` fits the engine's source limit; a larger program is
    /// rejected without touching the loaded one.
    fn accepts(self, text: &str) -> bool {
        let fits = text.len() <= MAX_SOURCE_BYTES as usize;
        if !fits {
            self.reject(format!(
                "This program is {} KiB, but the bounded Rust engine accepts at most 32 KiB of \
                 source.",
                text.len().div_ceil(1024)
            ));
        }
        fits
    }

    /// Report a request the page refused before it reached the engine, which
    /// keeps the program it had.
    fn reject(self, message: String) {
        self.announcement.set(message.clone());
        self.error.set(Some(Diagnostic {
            kind: None,
            title: "Program not loaded".into(),
            message,
            clause: None,
            note: "The previous program is still loaded and can keep running. Choose a smaller \
                   file, or pick a lesson."
                .into(),
        }));
    }

    fn show_unavailable(self, error: &str) {
        self.unavailable.set(Some(error.into()));
        self.announcement
            .set("The Rust WebAssembly engine is unavailable.".into());
    }

    fn drop_engine(self) {
        if let Some(engine) = self.engine.try_update_value(Option::take).flatten() {
            engine.dispose("Rust engine stopped.");
        }
        self.has_engine.set(false);
    }

    fn report(self, text: String) {
        self.announcement.set(text);
    }

    fn announce_state(self) {
        let text = self.current.with_untracked(|state| {
            state.as_ref().map(|state| {
                let mut text = format!(
                    "Step {}. {}. {}",
                    state.step,
                    state.event.title.replace('`', ""),
                    state.event.explanation.replace('`', "")
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

    /// The source editor's decorations: execution marks, run target, object
    /// colours, and pipeline line links.
    fn source_layers(self) -> Vec<Layer> {
        let snapshot = self.current.get();
        let edited = self.edited.get();
        let mut execution = Vec::new();
        let mut objects = Vec::new();
        if let Some(snapshot) = snapshot.as_ref().filter(|_| !edited) {
            let fault = snapshot.event.category == "fault";
            if let Some(range) = snapshot.event.range {
                let class = if fault { "exec-fault" } else { "exec-current" };
                let mut mark = boxed(
                    range,
                    &format!("{class} cat-{}", snapshot.event.category),
                    3,
                );
                if fault {
                    mark.inline_class = Some("exec-fault-text".into());
                }
                execution.push(mark);
                execution.push(Decoration::line(
                    range.start.line,
                    if fault {
                        "exec-line-fault"
                    } else {
                        "exec-line"
                    },
                ));
            }
            if let Some(next) = snapshot.location.filter(|_| snapshot.can_step())
                && Some(next) != snapshot.event.range
            {
                execution.push(boxed(next, "exec-next", 2));
            }
            self.source.with(|source| {
                let names: Vec<&str> = snapshot
                    .objects
                    .iter()
                    .map(|object| object.name.as_str())
                    .collect();
                for (start, end) in palette::identifiers(source) {
                    let name = &source[start..end];
                    if names.contains(&name) {
                        let (line, column) = palette::position(source, start);
                        let (end_line, end_column) = palette::position(source, end);
                        objects.push(Decoration {
                            start_line: line,
                            start_column: column,
                            end_line,
                            end_column,
                            inline_class: Some(format!("obj-ident {}", palette::class(name))),
                            ..Decoration::default()
                        });
                    }
                }
            });
        }
        let target = self
            .target_line
            .get()
            .map(|line| Decoration {
                glyph_class: Some("target-glyph".into()),
                ..Decoration::line(line, "target-line")
            })
            .into_iter()
            .collect();
        let hovered = self.hover_output.get();
        let links = if self.mode.get() == Mode::Pipeline && hovered > 0 {
            let stage = self.stage.get();
            self.compilation.with(|compilation| {
                compilation
                    .as_ref()
                    .and_then(|compilation| compilation.output(stage))
                    .map(|output| {
                        self.editor_text.with(|source| {
                            crate::pipeline::links(stage, output, source)
                                .sources_for(hovered)
                                .into_iter()
                                .map(|line| Decoration::line(line, "link-line"))
                                .collect()
                        })
                    })
                    .unwrap_or_default()
            })
        } else {
            Vec::new()
        };
        vec![
            ("execution", execution),
            ("objects", objects),
            ("target", target),
            ("links", links),
        ]
    }
}

/// The saved theme, or the system preference.
fn initial_theme() -> &'static str {
    let Some(window) = web_sys::window() else {
        return "dark";
    };
    let saved = window
        .local_storage()
        .ok()
        .flatten()
        .and_then(|storage| storage.get_item(THEME_KEY).ok().flatten());
    match saved.as_deref() {
        | Some("light") => "light",
        | Some("dark") => "dark",
        | _ if window
            .match_media("(prefers-color-scheme: light)")
            .ok()
            .flatten()
            .is_some_and(|query| query.matches()) =>
            "light",
        | _ => "dark",
    }
}

/// One continuous box behind `range` (Monaco draws a range's `className` as
/// a single overlay, where an inline class would box each token).
fn boxed(range: Range, class: &str, z_index: i32) -> Decoration {
    Decoration {
        start_line: range.start.line,
        start_column: range.start.column,
        end_line: range.end.line,
        end_column: range.end.column,
        class_name: Some(class.to_owned()),
        z_index,
        ..Decoration::default()
    }
}

#[component]
fn Header(state: State) -> impl IntoView {
    view! {
        <header class="site-header">
            <a class="brand" href="#top" aria-label="bcc home">
                <span class="brand-mark" aria-hidden="true">
                    <svg viewBox="0 0 32 32" width="30" height="30">
                        <rect x="1" y="1" width="30" height="30" rx="8" class="brand-tile" />
                        <path d="M9 10l6 6-6 6" class="brand-chevron" />
                        <path d="M17 22h7" class="brand-cursor" />
                    </svg>
                </span>
                <span class="wordmark">"bcc"</span>
                <span class="brand-divider" aria-hidden="true"></span>
                <span class="brand-subtitle">"How C runs"</span>
            </a>
            <nav class="site-nav" aria-label="Sections">
                <a href="#lessons">"Lessons"</a>
                <a href="#workbench">"Workbench"</a>
                <a href="#about">"How it works"</a>
            </nav>
            <div class="header-actions">
                <button
                    type="button"
                    class="icon-button theme-toggle"
                    aria-label="Toggle light and dark theme"
                    title="Toggle light and dark theme"
                    on:click=move |_| state.toggle_theme()
                >
                    <svg class="icon-sun" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><circle cx="12" cy="12" r="4.2" /><path d="M12 2.5v2.2M12 19.3v2.2M4.6 4.6l1.6 1.6M17.8 17.8l1.6 1.6M2.5 12h2.2M19.3 12h2.2M4.6 19.4l1.6-1.6M17.8 6.2l1.6-1.6" /></svg>
                    <svg class="icon-moon" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z" /></svg>
                </button>
                <a class="icon-button" href=REPOSITORY_URL aria-label="bcc on GitHub" title="bcc on GitHub">
                    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path class="fill" d="M12 2a10 10 0 0 0-3.2 19.5c.5.1.7-.2.7-.5v-1.7c-2.8.6-3.4-1.3-3.4-1.3-.4-1.2-1.1-1.5-1.1-1.5-.9-.6.1-.6.1-.6 1 .1 1.6 1 1.6 1 .9 1.6 2.4 1.1 3 .9.1-.7.4-1.1.6-1.4-2.2-.3-4.6-1.1-4.6-5 0-1.1.4-2 1-2.7-.1-.3-.4-1.3.1-2.7 0 0 .8-.3 2.8 1a9.6 9.6 0 0 1 5 0c1.9-1.3 2.8-1 2.8-1 .5 1.4.2 2.4.1 2.7.6.7 1 1.6 1 2.7 0 3.9-2.4 4.7-4.6 5 .4.3.7.9.7 1.9V21c0 .3.2.6.7.5A10 10 0 0 0 12 2z" /></svg>
                </a>
            </div>
        </header>
    }
}

#[component]
fn Hero() -> impl IntoView {
    view! {
        <section class="hero" id="top">
            <p class="eyebrow">"An interactive introduction to compilers and the C abstract machine"</p>
            <h1>"What really happens when C runs?"</h1>
            <p class="lede">
                "The C standard doesn’t describe your CPU. It describes an "<em>"abstract machine"</em>
                ": objects with lifetimes, values that are read and stored, and rules for exactly when each \
                 effect happens. Step through real programs one evaluation at a time, then watch bcc, a C \
                 compiler written in Rust, translate the same code into IR, assembly and machine code."
            </p>
            <ol class="hero-steps" aria-label="How to use this page">
                <li><span class="hero-step-number">"1"</span>"Pick a lesson"</li>
                <li><span class="hero-step-number">"2"</span>"Step through it with "<kbd>"→"</kbd></li>
                <li><span class="hero-step-number">"3"</span>"Open the compiler pipeline"</li>
            </ol>
        </section>
    }
}

#[component]
fn Lessons(state: State) -> impl IntoView {
    let selected = move || {
        state
            .editor_text
            .with(|text| demos::matching(text).map(|demo| demo.key))
    };
    view! {
        <section class="lessons" id="lessons" aria-labelledby="lessons-title">
            <div class="section-heading">
                <h2 id="lessons-title">"Lessons"</h2>
                <p>"Each lesson is a small, complete C program. Edit any of them, or open your own .c file."</p>
            </div>
            <ol class="lesson-list">
                {DEMOS
                    .iter()
                    .enumerate()
                    .map(|(index, demo)| {
                        let key = demo.key;
                        view! {
                            <li>
                                <button
                                    type="button"
                                    class="lesson-card"
                                    class:is-fault=demo.faults
                                    aria-pressed=move || (selected() == Some(key)).to_string()
                                    disabled=move || state.locked_tracked()
                                    on:click=move |_| state.select_demo(demo)
                                >
                                    <span class="lesson-number">{format!("{:02}", index + 1)}</span>
                                    <span class="lesson-title">{demo.title}</span>
                                    <span class="lesson-summary">{demo.summary}</span>
                                    <span class="lesson-topics">
                                        {demo.topics.iter().map(|topic| view! { <span class="topic">{*topic}</span> }).collect_view()}
                                    </span>
                                </button>
                            </li>
                        }
                    })
                    .collect_view()}
            </ol>
        </section>
    }
}

#[component]
fn EngineNotice(state: State) -> impl IntoView {
    view! {
        <section class="notice" aria-labelledby="notice-title" hidden=move || state.unavailable.with(Option::is_none)>
            <div class="notice-icon" aria-hidden="true">"!"</div>
            <div>
                <h2 id="notice-title">"The Rust engine is unavailable"</h2>
                <p>{move || state.unavailable.get().unwrap_or_default()}</p>
                <p>"The engine needs a browser with Web Workers and WebAssembly. Retry it, or reload the page if it keeps failing."</p>
                <button type="button" class="button" disabled=move || state.busy.get() on:click=move |_| state.initialize()>
                    "Retry engine"
                </button>
            </div>
        </section>
    }
}

/// The page-level diagnostic: an engine request that failed, or a program the
/// page refused to load, with how to recover.
#[component]
fn SiteError(state: State) -> impl IntoView {
    let field = move |read: fn(&Diagnostic) -> String| {
        move || {
            state
                .error
                .with(|error| error.as_ref().map(read).unwrap_or_default())
        }
    };
    view! {
        <section class="notice site-error" aria-labelledby="site-error-title" hidden=move || state.error.with(Option::is_none)>
            <div class="notice-icon" aria-hidden="true">"!"</div>
            <div>
                <h2 id="site-error-title">{field(|error| error.title.clone())}</h2>
                <p>{field(|error| error.message.clone())}</p>
                <p>{field(|error| error.note.clone())}</p>
                <div class="notice-actions">
                    <button
                        type="button"
                        class="button"
                        hidden=move || !state.failed.get()
                        disabled=move || state.locked_tracked() || !state.has_engine.get()
                        on:click=move |_| state.reset()
                    >
                        "Reset program"
                    </button>
                    <button type="button" class="button ghost" on:click=move |_| state.error.set(None)>
                        "Dismiss"
                    </button>
                </div>
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
            | None if state.unavailable.with(Option::is_some) => "Engine unavailable".to_owned(),
            | None => "Loading engine…".to_owned(),
            | Some(snapshot)
                if snapshot.diagnostic.as_ref().and_then(|d| d.kind.as_deref())
                    == Some("indeterminate") =>
                "Stopped: indeterminate read".to_owned(),
            | Some(snapshot) => status_label(&snapshot.status).to_owned(),
        })
    };
    let status_kind = move || {
        state.current.with(|snapshot| {
            snapshot
                .as_ref()
                .map_or_else(|| "idle".to_owned(), |snapshot| snapshot.status.clone())
        })
    };
    let tab = move |mode: Mode, label: &'static str, icon: &'static str| {
        view! {
            <button
                type="button"
                role="tab"
                class="mode-tab"
                aria-selected=move || (state.mode.get() == mode).to_string()
                aria-controls="insight"
                on:click=move |_| state.set_mode(mode)
            >
                <span class="mode-icon" aria-hidden="true">{icon}</span>
                {label}
            </button>
        }
    };
    view! {
        <div class="toolbar">
            <div class="mode-tabs" role="tablist" aria-label="View">
                {tab(Mode::Machine, "Abstract machine", "◈")}
                {tab(Mode::Pipeline, "Compiler pipeline", "⇢")}
            </div>
            <div class="transport" role="group" aria-label="Execution controls" hidden=move || state.mode.get() != Mode::Machine>
                <button
                    type="button"
                    class="button ghost"
                    title="Reset to the initial state (Shift+R)"
                    disabled=move || state.locked_tracked() || !state.has_engine.get()
                    on:click=move |_| state.reset()
                >
                    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M4 10a6 6 0 1 0 1.8-4.3M4 3.5v3.2h3.2" /></svg>
                    "Reset"
                </button>
                <button
                    type="button"
                    class="button"
                    title="Back one step (←)"
                    disabled=move || state.locked_tracked() || state.edited.get() || state.cursor.get().is_none_or(|cursor| cursor == 0)
                    on:click=move |_| state.back()
                >
                    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M12 5l-5 5 5 5" /></svg>
                    "Back"
                </button>
                <button
                    type="button"
                    class="button primary"
                    title="Step one evaluation (→)"
                    disabled=move || state.locked_tracked() || !state.can_step()
                    on:click=move |_| state.step()
                >
                    "Step"
                    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M8 5l5 5-5 5" /></svg>
                </button>
                <button
                    type="button"
                    class="button"
                    title=move || if state.running.get() { "Pause (R)" } else { "Run until the program ends (R)" }
                    aria-pressed=move || state.running.get().to_string()
                    disabled=move || {
                        state.busy.get() || (state.running.get() && state.pause_requested.get()) || (!state.running.get() && !state.can_step())
                    }
                    on:click=move |_| state.toggle_run()
                >
                    {move || if state.running.get() {
                        view! { <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M7 5v10M13 5v10" /></svg> }.into_any()
                    } else {
                        view! { <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path class="fill" d="M6 4.5v11l9-5.5z" /></svg> }.into_any()
                    }}
                    {run_label}
                </button>
                <button
                    type="button"
                    class="button ghost"
                    hidden=move || state.target_line.get().is_none()
                    disabled=move || state.locked_tracked() || !state.can_step()
                    on:click=move |_| state.run(state.target_line.get_untracked())
                >
                    {move || format!("Run to line {}", state.target_line.get().unwrap_or_default())}
                </button>
            </div>
            <div class="run-status" hidden=move || state.mode.get() != Mode::Machine>
                <span class="status-pill" data-status=status_kind>{status}</span>
                <span class="step-counter">"Step "<strong>{move || state.current.with(|s| s.as_ref().map_or_else(|| "–".into(), |s| s.step.to_string()))}</strong></span>
            </div>
        </div>
    }
}

#[component]
fn SourcePane(state: State) -> impl IntoView {
    let layers = Signal::derive(move || state.source_layers());
    let on_change = Callback::new(move |text: String| {
        if text != state.editor_text.get_untracked() {
            state.editor_text.set(text);
        }
    });
    let on_hover = Callback::new(move |line| state.hover_source.set(line));
    let on_gutter = Callback::new(move |line| state.set_target(line));
    view! {
        <section class="pane source-pane" aria-labelledby="source-title">
            <header class="pane-header">
                <div class="pane-title">
                    <h2 id="source-title" class="sr-only">"Source"</h2>
                    <span class="file-chip">
                        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M4 1.5h5l3.5 3.5v9.5h-8.5zM9 1.5V5h3.5" /></svg>
                        {move || state.filename.get()}
                    </span>
                    <span class="edited-badge" hidden=move || !state.edited.get()>"edited"</span>
                </div>
                <div class="pane-actions">
                    <button
                        type="button"
                        class="button small primary"
                        hidden=move || !state.edited.get()
                        disabled=move || state.locked_tracked() || !state.has_engine.get()
                        on:click=move |_| state.run_edits()
                    >
                        "Run edited code"
                    </button>
                    <label class="button small ghost file-button" for="file-input">
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
            </header>
            <CodeEditor
                text=state.editor_text
                language=Signal::stored("c")
                theme=state.theme_name
                label="C source code. Edit it, then choose Run edited code."
                decorations=layers
                on_change
                on_hover_line=on_hover
                on_gutter_click=on_gutter
                reveal=state.reveal.into()
            />
            <footer class="source-legend">
                <span><i class="swatch swatch-current"></i>"just evaluated"</span>
                <span><i class="swatch swatch-next"></i>"next"</span>
                <span><i class="swatch swatch-target"></i>"click a line number to set a run target"</span>
            </footer>
            <p class="edit-hint" hidden=move || !state.edited.get()>
                "You changed the program. The abstract machine and the compiler views still show the last version you ran: choose "<strong>"Run edited code"</strong>" to restart with your edits."
            </p>
        </section>
    }
}

#[component]
fn About() -> impl IntoView {
    view! {
        <section class="about" id="about" aria-labelledby="about-title">
            <div class="section-heading">
                <h2 id="about-title">"How it works"</h2>
                <p>"Two very different ways of giving a C program meaning, side by side."</p>
            </div>
            <div class="about-grid">
                <article>
                    <h3>"The abstract machine"</h3>
                    <p>
                        "The C standard defines a program by what an idealized machine does with it. There are no registers \
                         here, only objects: regions of storage with a type, a lifetime and a value. Evaluating an \
                         expression reads objects (lvalue conversion), computes new values, and stores them as side \
                         effects that must be complete at the next sequence point."
                    </p>
                    <p>
                        "The engine on this page is a small, independent interpreter written in Rust and compiled to \
                         WebAssembly. Every explanation comes from what it actually did, and cites the paragraph of C99 \
                         (ISO/IEC 9899, as in N1256) that requires it."
                    </p>
                </article>
                <article>
                    <h3>"The compiler"</h3>
                    <p>
                        "A real compiler doesn’t run your program; it translates it. bcc reads tokens, builds a syntax \
                         tree, checks types, lowers everything to an SSA intermediate representation, optimizes it, and \
                         finally selects x86-64 instructions and encodes them as bytes."
                    </p>
                    <p>
                        "It is allowed to produce any code that behaves like the abstract machine would. That freedom is \
                         why undefined behavior matters: the compiler may assume it never happens."
                    </p>
                </article>
                <article>
                    <h3>"Limits of this model"</h3>
                    <p>
                        "The engine runs a bounded subset of C: int and char locals, int pointers, fixed int arrays, \
                         functions, if, while and for. No preprocessor, globals, structs, library calls or floating \
                         point. Operands are evaluated left to right, and nested side effects are rejected rather than \
                         sequenced."
                    </p>
                    <p>
                        "Compiler outputs for the lessons were produced by the native bcc build. LLVM IR cannot be \
                         produced in the browser, so it is shown for the bundled lessons only. "
                        <a href=README_URL>"Read the full notes"</a>"."
                    </p>
                </article>
            </div>
        </section>
    }
}
