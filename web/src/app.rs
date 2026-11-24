//! The page: lesson picker, source editor, and the three views of a
//! program: the abstract machine stepping it, the compiler pipeline
//! translating it, and its WebAssembly translation stepped instruction by
//! instruction (`app/wasm.rs`).
//!
//! Every program state comes from the Rust engine in its Worker. Back replays
//! retained snapshots for presentation only; the engine stays at the newest
//! recorded state, and stepping forward through history reaches that point
//! before requesting a new engine step.

use std::rc::Rc;

use gloo_timers::{
    callback::Timeout,
    future::TimeoutFuture,
};
use leptos::{
    ev,
    prelude::*,
    task::spawn_local,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlInputElement;

use self::links::Side;
use crate::{
    demos::{
        self,
        DEMOS,
        Demo,
        Part,
    },
    editor::{
        CodeEditor,
        Decoration,
        Layer,
        Marker,
    },
    engine::{
        self,
        Engine,
    },
    glossary::{
        COMPILER_TERMS,
        Glossary,
    },
    palette,
    panels::{
        Cells,
        Datasheet,
        Deck,
        Knob,
        LiveCompile,
        Narration,
        Output,
        PipelineView,
        Reel,
        Schematic,
        WasmListing,
        WasmView,
        evaluation,
        focus_stage_key,
        source_marks,
    },
    pipeline::{
        Compilation,
        CompileDiagnostic,
        Stage,
        Unfolded,
    },
    snapshot::{
        Diagnostic,
        N1256_URL,
        Range,
        Snapshot,
        Value,
        status_label,
    },
};

#[component]
#[expect(clippy::too_many_lines, reason = "The page's one tree of views.")]
pub fn App() -> impl IntoView {
    let state = State::new();
    // Effects run only in the browser, so prerendering never starts a Worker.
    Effect::new(move || state.initialize());
    Effect::new(move |previous: Option<()>| {
        if previous.is_none() {
            state.theme.set(initial_theme());
            state.wasm.restore_choice();
        }
    });
    Effect::new(move || {
        let theme = state.theme.get();
        if let Some(root) = document().document_element() {
            _ = root.set_attribute("data-theme", theme);
        }
    });
    // Compile the other level as soon as the switch changes.
    Effect::new(move || {
        state.optimize.track();
        state.compile_now();
    });
    // The WebAssembly view follows the program the page runs.
    Effect::new(move || {
        state.source.track();
        if state.mode.get() == Mode::Wasm && state.has_engine.get() {
            state.wasm.ensure_loaded(state);
        }
    });
    // A view switch unmounts the output's editor, which reports no leaving.
    Effect::new(move || {
        state.mode.track();
        state.pointing.clear_output();
    });
    links::announce(state);
    Effect::new(move || {
        // The page lives as long as the listener.
        let _handle = window_event_listener(ev::keydown, move |event| state.shortcut(&event));
    });
    let on_screen = RwSignal::new(false);
    Effect::new(move || watch_workbench(on_screen));
    view! {
        <a class="skip-link" href="#workbench">"Skip to the workbench"</a>
        <Header state />
        <main>
            <Hero state />
            <Lessons state />
            <EngineNotice state />
            <section
                class="workbench"
                class:is-on-screen=move || on_screen.get()
                id="workbench"
                aria-label="Workbench"
            >
                <Toolbar state />
                <SiteError state />
                <div class="workbench-grid">
                    <SourcePane state />
                    <div class="insight" id="insight">
                        <Show
                            when=move || state.mode.get() == Mode::Machine
                            fallback=move || view! {
                                <Show
                                    when=move || state.mode.get() == Mode::Wasm
                                    fallback=move || view! {
                                        <PipelineView
                                            stage=state.stage
                                            optimize=state.optimize
                                            compilation=state.compilation
                                            shown=state.shown
                                            links=state.links
                                            reading=Signal::derive(move || state.link_reading())
                                            other=state.other_level
                                            compare=state.compare
                                            other_links=state.other_links
                                            live=state.live_status.into()
                                            theme=state.theme_name
                                            pointing=state.pointing
                                            unfolded=state.unfolded
                                            filename=state.filename.read_only()
                                            source=state.editor_text.into()
                                            on_reveal=Callback::new(move |line| state.show_source_line(line))
                                            on_run=Callback::new(move |()| state.watch_run())
                                        />
                                    }
                                >
                                    <WasmView
                                        wasm=state.wasm
                                        stale=Signal::derive(move || state.editor_text.with(|text| state.wasm.stale(text)))
                                        on_jump=Callback::new(move |step| state.wasm.jump(state, step))
                                        on_choose=Callback::new(move |compiler| state.wasm.choose(state, compiler))
                                    />
                                </Show>
                            }
                        >
                            <MachineView state />
                        </Show>
                    </div>
                </div>
            </section>
            <About />
            <Glossary on_stage=Callback::new(move |stage| state.open_view(Mode::Pipeline, Some(stage))) />
            <p class="sr-only" role="status" aria-live="polite" aria-atomic="true">{state.announcement}</p>
            <p class="sr-only" role="status" aria-live="polite" aria-atomic="true">{state.pointing.announcement}</p>
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
/// The SPEED knob's detents: Run's pause between steps, in milliseconds.
const RUN_DELAYS: [u32; 9] = [20, 40, 70, 100, 140, 200, 300, 450, 700];
/// Steps a held key may queue while the engine answers.
const MAX_QUEUED_STEPS: u32 = 4;
const THEME_KEY: &str = "bcc-theme";
/// How long typing must pause before the compiler runs.
const COMPILE_DEBOUNCE_MS: u32 = 400;
/// Compilations kept, so switching level or lesson back is instant.
const COMPILED_LIMIT: usize = 8;

/// A source and whether it is compiled at `-O2`.
type CompileKey = (String, bool);

/// Recent compilations, or why the Worker could not compile, oldest first.
type Compiled = Vec<(CompileKey, Result<Compilation, String>)>;

/// Which view of the program the workbench shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Machine,
    Pipeline,
    Wasm,
}

/// Page state. Every field is a `Copy` handle, so closures capture it freely.
#[derive(Clone, Copy)]
pub(crate) struct State {
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
    /// What the reader unfolded: the preprocessed text's headers, the IRs'
    /// target lines, the assembly's call-frame directives.
    unfolded:        RwSignal<Unfolded>,
    theme:           RwSignal<&'static str>,
    theme_name:      Signal<&'static str>,
    /// bcc's compiler Worker, started on the first compilation.
    compiler:        StoredValue<Option<Rc<Engine>>, LocalStorage>,
    starting:        RwSignal<bool>,
    /// Whether the pipeline view shows `-O2` rather than `-O0` output.
    optimize:        RwSignal<bool>,
    compiled:        RwSignal<Compiled>,
    /// The compilation the Worker is running; one runs at a time.
    in_flight:       RwSignal<Option<CompileKey>>,
    /// The debounce timer while typing; dropping it cancels it.
    debounce:        StoredValue<Option<Timeout>, LocalStorage>,
    pending:         RwSignal<bool>,
    live_status:     Memo<LiveCompile>,
    /// The lines linked between the source and the view beside it.
    pointing:        links::Pointing,
    reveal:          RwSignal<Option<usize>>,
    current:         Memo<Option<Snapshot>>,
    recent:          Memo<Vec<(usize, String, String)>>,
    /// The steps of the expression the current step belongs to.
    group:           Memo<Option<evaluation::Group>>,
    edited:          Memo<bool>,
    /// The editor's text compiled at the selected level, with the
    /// precomputed LLVM IR for a lesson.
    compilation:     Memo<Option<Compilation>>,
    /// What the pipeline view shows: `compilation`, or the last output while
    /// the next one compiles.
    shown:           Memo<Option<Compilation>>,
    /// The links between the source and `compilation`'s output for the stage,
    /// which every move of the pointer or the cursor reads.
    links:           Memo<crate::pipeline::Links>,
    /// The same at the other level, for the optimizer comparison.
    other_level:     Memo<Option<Compilation>>,
    /// Whether the pipeline shows `-O0` and `-O2` side by side.
    compare:         RwSignal<bool>,
    /// The links between the source and `other_level`'s output for the
    /// stage, while the two levels are side by side.
    other_links:     Memo<crate::pipeline::Links>,
    /// bcc's diagnostics as squiggles in the source editor: the pipeline's
    /// compilation's, or in the WebAssembly view that view's.
    markers:         Memo<Vec<Marker>>,
    placeholder:     Memo<&'static str>,
    /// The WebAssembly view.
    wasm:            wasm::WasmState,
    /// bcc's recent compilations of the editor's text for that view.
    wasm_checks:     RwSignal<wasm::Checks>,
    /// The tape's scale, as a factor of the scale that fits the run to the
    /// screen.
    zoom:            RwSignal<f64>,
    /// How long Run pauses between steps, in milliseconds.
    run_delay:       RwSignal<u32>,
    /// Where the playhead is across the tape's viewport, while it is in view.
    head:            RwSignal<Option<f64>>,
}

impl State {
    #[expect(
        clippy::too_many_lines,
        reason = "One line per piece of the page's state."
    )]
    fn new() -> Self {
        let tour = &DEMOS[0];
        let history = RwSignal::new(Vec::<Snapshot>::new());
        let cursor = RwSignal::new(None::<usize>);
        let source = RwSignal::new(tour.source.to_owned());
        let editor_text = RwSignal::new(tour.source.to_owned());
        let error = RwSignal::new(None::<Diagnostic>);
        let optimize = RwSignal::new(false);
        let compiled = RwSignal::new(Compiled::new());
        let in_flight = RwSignal::new(None::<CompileKey>);
        let starting = RwSignal::new(false);
        let pending = RwSignal::new(false);
        let theme = RwSignal::new("dark");
        let mode = RwSignal::new(Mode::Machine);
        // The pipeline opens at its first stage, wherever it is opened from, so
        // a reader meets the stages in their order; the stage keys and the
        // front end's "Skip to bcc IR" lead straight to the IR.
        let (stage, unfolded) = (
            RwSignal::new(Stage::Lines),
            RwSignal::new(Unfolded::default()),
        );
        let compare = RwSignal::new(false);
        let wasm_checks = RwSignal::new(wasm::Checks::default());
        let live = LiveViews::new(
            editor_text,
            optimize,
            compiled,
            in_flight,
            pending,
            starting,
        );
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
        let group = Memo::new(move |_| {
            let cursor = cursor.get()?;
            source.with(|source| history.with(|history| evaluation::group(history, cursor, source)))
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
            mode,
            stage,
            unfolded,
            theme,
            theme_name: Signal::derive(move || theme.get()),
            compiler: StoredValue::new_local(None),
            starting,
            optimize,
            compiled,
            in_flight,
            debounce: StoredValue::new_local(None),
            pending,
            live_status: live.status,
            pointing: links::Pointing::new(),
            reveal: RwSignal::new(None),
            current,
            recent,
            group,
            edited: Memo::new(move |_| {
                editor_text.with(|text| source.with(|source| text != source))
            }),
            compilation: live.compilation,
            links: links_memo(live.compilation, stage, unfolded, editor_text),
            shown: live.shown,
            other_level: live.other_level,
            compare,
            other_links: other_links_memo(live.other_level, compare, stage, unfolded, editor_text),
            markers: markers(mode, editor_text, wasm_checks, live.markers),
            placeholder: Memo::new(move |_| {
                editor_text.with(|text| demos::matching(text).map_or("", |demo| demo.summary))
            }),
            wasm: wasm::WasmState::new(),
            wasm_checks,
            zoom: RwSignal::new(1.0),
            run_delay: RwSignal::new(RUN_YIELD_MS),
            head: RwSignal::new(None),
        }
    }

    /// Start (or restart) the engine Worker and load the current program.
    fn initialize(self) {
        spawn_local(async move {
            self.busy.set(true);
            self.drop_engine();
            match Engine::connect(&engine::MACHINE).await {
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
            crate::editor::prefetch();
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
        // A new program starts at the scale that fits it.
        self.zoom.set(1.0);
        self.failed.set(false);
        self.error.set(None);
        self.compile_now();
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
            self.compile_now();
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
                TimeoutFuture::new(self.run_delay.get_untracked()).await;
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

    /// Step over: run to the last step of the expression the current step
    /// belongs to, through any call it makes. At that last step, or at a
    /// step that evaluates no expression, run to the last step of the next
    /// one instead (a step that evaluates none is its own last step).
    fn over(self) {
        if self.locked() || !self.can_step_untracked() {
            return;
        }
        self.running.set(true);
        self.pause_requested.set(false);
        spawn_local(async move {
            let cursor = self.cursor.get_untracked().unwrap_or(0);
            let mut over = self
                .history
                .with_untracked(|history| evaluation::Over::new(history, cursor));
            let mut steps = 0;
            let mut result = Ok(());
            while !self.pause_requested.get_untracked()
                && self.can_step_untracked()
                && steps < RUN_LIMIT
            {
                result = self.step_forward().await;
                if result.is_err() {
                    break;
                }
                steps += 1;
                let Some(cursor) = self.cursor.get_untracked() else {
                    break;
                };
                if let Some(stop) = self
                    .history
                    .with_untracked(|history| over.stepped(history, cursor))
                {
                    if stop != cursor {
                        self.cursor.set(Some(stop));
                    }
                    break;
                }
            }
            match result {
                | Err(error) => self.show_error(&error),
                | Ok(()) if steps == RUN_LIMIT && self.can_step_untracked() => {
                    self.report(format!(
                        "Paused after {RUN_LIMIT} operations. Step over again to continue."
                    ));
                },
                | Ok(()) => self.after_step(),
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

    /// Load a lesson, in the view it is about: a lesson about the compiler
    /// opens the pipeline at its stage, unless the WebAssembly view is open.
    fn select_demo(self, demo: &'static Demo) {
        self.open_lesson_view(demo);
        self.action(move || self.stage_program(demo.source.into(), demo.name.into()));
    }

    /// Open a lesson chosen in Table 1, which loads into the workbench out
    /// of sight below the table: bring the workbench on screen at once, and
    /// again once the lesson has loaded, since its listing and tape then
    /// take their own heights and the browser's scroll anchoring moves the
    /// page to keep whatever it anchored on in place.
    fn choose_lesson(self, demo: &'static Demo) {
        if self.locked() {
            return;
        }
        self.open_lesson_view(demo);
        show_workbench(true);
        self.action(move || async move {
            let loaded = self
                .stage_program(demo.source.into(), demo.name.into())
                .await;
            show_workbench(false);
            loaded
        });
    }

    /// The view a lesson opens in. The WebAssembly view steps every lesson,
    /// compiled by bcc itself, so it stays open. Otherwise a lesson about the
    /// compiler opens the pipeline at its stage, and the others the abstract
    /// machine.
    fn open_lesson_view(self, demo: &'static Demo) {
        if let Some(stage) = demo.stage {
            self.stage.set(stage);
        }
        if self.mode.get_untracked() != Mode::Wasm {
            self.mode.set(if demo.stage.is_some() {
                Mode::Pipeline
            } else {
                Mode::Machine
            });
        }
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

    /// Open the pipeline at its first stage. A bundled lesson in the editor
    /// gives way to the lesson that follows one program through every
    /// stage; the learner's own program, opened or edited, stays.
    fn follow_path(self) {
        let lesson = self
            .editor_text
            .with_untracked(|text| demos::matching(text).is_some());
        match DEMOS.iter().find(|demo| !demo.path.is_empty()) {
            | Some(demo) if lesson => {
                self.mode.set(Mode::Pipeline);
                self.select_demo(demo);
            },
            | _ => self.open_view(Mode::Pipeline, Some(Stage::Lines)),
        }
    }

    /// Leave the pipeline's last stage for the abstract machine, to watch
    /// the program run: the workbench comes on screen and the narration's
    /// heading takes the keyboard focus the pipeline's button had.
    fn watch_run(self) {
        self.open_view(Mode::Machine, None);
        let _ = Timeout::new(0, || {
            let document = document();
            if let Some(workbench) = document.get_element_by_id("workbench") {
                let options = web_sys::ScrollIntoViewOptions::new();
                options.set_block(web_sys::ScrollLogicalPosition::Start);
                options.set_behavior(web_sys::ScrollBehavior::Instant);
                workbench.scroll_into_view_with_scroll_into_view_options(&options);
            }
            if let Some(heading) = document
                .get_element_by_id("narration-title")
                .and_then(|heading| heading.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = heading.focus();
            }
        })
        .forget();
    }

    /// Open `mode` (and pipeline `stage`) from a link elsewhere on the page,
    /// which itself scrolls to the workbench.
    fn open_view(self, mode: Mode, stage: Option<Stage>) {
        if let Some(stage) = stage {
            self.stage.set(stage);
        }
        self.mode.set(mode);
        self.compile_now();
    }

    /// Open the pipeline at `stage` from a button of the machine's note,
    /// which goes with the machine's view: the focus moves to the stage's
    /// key once the pipeline is drawn, rather than falling to the page.
    fn open_stage(self, stage: Stage) {
        self.open_view(Mode::Pipeline, Some(stage));
        Timeout::new(0, move || focus_stage_key(stage)).forget();
    }

    fn set_mode(self, mode: Mode) {
        hold_source_pane();
        self.mode.set(mode);
        // Loading the program compiles it, so the squiggles' compilation
        // waits for it rather than compiling the same text twice.
        if mode == Mode::Wasm {
            self.wasm.ensure_loaded(self);
        }
        self.compile_now();
    }

    /// Compile the editor's text once typing pauses.
    fn compile_after_typing(self) {
        self.pending.set(true);
        let timer = Timeout::new(COMPILE_DEBOUNCE_MS, move || {
            self.pending.set(false);
            self.pump();
        });
        // Replacing the previous timer cancels it.
        self.debounce.set_value(Some(timer));
    }

    /// Compile the editor's text now, as after choosing a lesson or a view.
    fn compile_now(self) {
        self.debounce.set_value(None);
        self.pending.set(false);
        self.pump();
    }

    /// Start the next compilation the page needs, unless one is running:
    /// the editor's text at the selected level, then at the other level for
    /// the optimizer comparison. Text is compiled when the pipeline view
    /// shows it or when it is not a bundled lesson, so its errors show as
    /// squiggles; a lesson in the abstract machine view never loads the
    /// compiler. In the WebAssembly view the text is compiled for that
    /// target instead. A result for text the editor no longer holds is
    /// stale and dropped.
    fn pump(self) {
        if self.in_flight.with_untracked(Option::is_some) || self.pending.get_untracked() {
            return;
        }
        // The WebAssembly view's squiggles come from its own compilation.
        if self.mode.get_untracked() == Mode::Wasm {
            self.wasm.check(self);
            return;
        }
        let Some(key) = self.next_compilation() else {
            return;
        };
        // Larger source than the in-browser compiler takes is refused here,
        // without loading it.
        if let Some(refusal) = crate::pipeline::too_large(&key.0) {
            self.compiled
                .update(|compiled| remember(compiled, key, Err(refusal)));
            return;
        }
        self.in_flight.set(Some(key.clone()));
        spawn_local(async move {
            let result = match self.compiler().await {
                // The editor may have changed while the compiler loaded:
                // drop the request and start whatever is needed now, once
                // typing pauses.
                | Ok(_)
                    if self.pending.get_untracked()
                        || self.next_compilation().as_ref() != Some(&key) =>
                {
                    self.in_flight.set(None);
                    self.pump();
                    return;
                },
                | Ok(compiler) => {
                    let result = compiler.compile(&key.0, key.1).await;
                    // A compiler that timed out or trapped is gone; the
                    // next compilation starts a fresh one.
                    if !compiler.connected() {
                        self.compiler.set_value(None);
                    }
                    result.map_err(compiler_failure)
                },
                | Err(error) => Err(error),
            };
            self.in_flight.set(None);
            if self.editor_text.with_untracked(|text| *text == key.0) {
                self.compiled
                    .update(|compiled| remember(compiled, key, result));
            }
            self.pump();
        });
    }

    fn next_compilation(self) -> Option<CompileKey> {
        let pipeline = self.mode.get_untracked() == Mode::Pipeline;
        let level = self.optimize.get_untracked();
        self.editor_text.with_untracked(|text| {
            self.compiled
                .with_untracked(|compiled| wanted(compiled, text, level, pipeline))
        })
    }

    /// The compiler Worker, started on first use.
    async fn compiler(self) -> Result<Rc<Engine>, String> {
        if let Some(compiler) = self
            .compiler
            .with_value(Clone::clone)
            .filter(|compiler| compiler.connected())
        {
            return Ok(compiler);
        }
        self.starting.set(true);
        let connected = Engine::connect(&engine::COMPILER).await.map(Rc::new);
        self.starting.set(false);
        let compiler = connected?;
        self.compiler.set_value(Some(Rc::clone(&compiler)));
        Ok(compiler)
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
                    .closest("input, textarea, select, .code-editor, [contenteditable]")
                    .ok()
                    .flatten()
                    .is_some()
            });
        if typing || self.mode.get_untracked() == Mode::Pipeline {
            return;
        }
        if self.mode.get_untracked() == Mode::Wasm {
            let wasm = self.wasm;
            match event.key().as_str() {
                | "ArrowRight" | "j" => wasm.run_command(self, crate::wasm::WasmRun::Step),
                | "ArrowLeft" | "k" => wasm.back(self),
                | "o" => wasm.run_command(self, crate::wasm::WasmRun::StepOver),
                | "u" => wasm.run_command(self, crate::wasm::WasmRun::StepOut),
                | "n" => wasm.run_command(self, crate::wasm::WasmRun::NextLine),
                | "r" => wasm.toggle_run(self),
                | "R" => wasm.reset(self),
                | _ => return,
            }
            event.prevent_default();
            return;
        }
        match event.key().as_str() {
            | "ArrowRight" | "j" => self.step(),
            | "ArrowLeft" | "k" => self.back(),
            | "o" => self.over(),
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

    /// Show `line` of the source, as a link from the pipeline asks. On
    /// narrow screens the source sits above the pipeline, so bring it on
    /// screen first, below the sticky toolbar; the editor then scrolls the
    /// line itself clear of it.
    fn show_source_line(self, line: usize) {
        if let Ok(Some(editor)) = document().query_selector(".source-pane > .code-editor") {
            let options = web_sys::ScrollIntoViewOptions::new();
            options.set_block(web_sys::ScrollLogicalPosition::Nearest);
            options.set_behavior(web_sys::ScrollBehavior::Instant);
            editor.scroll_into_view_with_scroll_into_view_options(&options);
        }
        self.reveal.set(Some(line));
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
        // The next engine has no WebAssembly module loaded.
        self.wasm.engine_stopped();
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
                if let Some(tape) = tape_summary(state) {
                    text.push(' ');
                    text.push_str(&tape);
                }
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
    /// colours, and in the pipeline view the stage's own marks and the line
    /// links (with the exact range of a hovered preprocessing token).
    #[expect(clippy::too_many_lines, reason = "One block per layer of marks.")]
    fn source_layers(self) -> Vec<Layer> {
        if self.mode.get() == Mode::Wasm {
            let (execution, links) = if self.wasm.stale(&self.editor_text.get()) {
                (Vec::new(), Vec::new())
            } else {
                self.wasm.source_marks(self.pointing.line(Side::Output))
            };
            // Each layer is listed, so a mark another view drew (such as the
            // pipeline stage's) is cleared.
            return vec![
                ("execution", execution),
                ("objects", Vec::new()),
                ("target", self.target_layer()),
                ("stage", Vec::new()),
                ("links", links),
            ];
        }
        let snapshot = self.current.get();
        let edited = self.edited.get();
        let mut execution = Vec::new();
        let mut objects = Vec::new();
        // The pipeline view marks what the compiler did with the source,
        // not where the abstract machine is: its stepping marks would
        // say "just evaluated" in a view that evaluates nothing.
        let pipeline = self.mode.get() == Mode::Pipeline;
        if let Some(snapshot) = snapshot.as_ref().filter(|_| !(edited || pipeline)) {
            let fault = snapshot.event.category == "fault";
            if let Some(range) = snapshot.event.range {
                let class = if fault { "exec-fault" } else { "exec-current" };
                let mut mark = boxed(
                    range,
                    &format!("{class} cat-{}", snapshot.event.category),
                    3,
                );
                // On the lit box the code is dark, whatever its token colour.
                mark.inline_class = Some(
                    if fault {
                        "exec-fault-text"
                    } else {
                        "exec-current-text"
                    }
                    .into(),
                );
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
            objects.extend(function_marks(snapshot));
            objects.extend(goto_mark(snapshot));
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
        let target = if pipeline {
            Vec::new()
        } else {
            self.target_layer()
        };
        let links = if pipeline {
            self.hover_links()
        } else {
            Vec::new()
        };
        let stage_marks = if pipeline {
            let stage = self.stage.get();
            self.compilation.with(|compilation| {
                compilation
                    .as_ref()
                    .map_or_default(|compilation| source_marks(stage, compilation))
            })
        } else {
            Vec::new()
        };
        vec![
            ("execution", execution),
            ("objects", objects),
            ("target", target),
            ("stage", stage_marks),
            ("links", links),
        ]
    }

    /// The source lines (and the exact range, for a preprocessing token)
    /// that the pipeline output line under the pointer came from.
    fn hover_links(self) -> Vec<Decoration> {
        // A line of the other level's output, beside the selected one, links
        // through that level's own lines.
        let beside = self.pointing.beside();
        let (hovered, links) = if beside > 0 {
            (beside, self.other_links)
        } else {
            (self.pointing.line(Side::Output), self.links)
        };
        if hovered == 0 {
            return Vec::new();
        }
        links.with(|links| {
            let mut marks: Vec<Decoration> = links
                .sources_for(hovered)
                .into_iter()
                .map(|line| Decoration::line(line, "link-line"))
                .collect();
            // The call an inlined instruction was copied for.
            let origin = links.origin(hovered);
            if origin.call != 0 && origin.call != origin.line {
                marks.push(Decoration::line(origin.call, "link-call"));
            }
            if let Some(range) = links.range_for(hovered) {
                marks.push(Decoration {
                    start_line: range.line,
                    start_column: range.column,
                    end_line: range.end_line,
                    end_column: range.end_column.max(range.column + 1),
                    class_name: Some("link-range".to_owned()),
                    z_index: 4,
                    ..Decoration::default()
                });
            }
            marks
        })
    }

    /// The run target's mark, set by clicking a line number.
    fn target_layer(self) -> Vec<Decoration> {
        self.target_line
            .get()
            .map(|line| Decoration {
                glyph_class: Some("target-glyph".into()),
                ..Decoration::line(line, "target-line")
            })
            .into_iter()
            .collect()
    }
}

/// The page's views of live compilation, derived from the compilation cache.
struct LiveViews {
    status:      Memo<LiveCompile>,
    compilation: Memo<Option<Compilation>>,
    shown:       Memo<Option<Compilation>>,
    other_level: Memo<Option<Compilation>>,
    markers:     Memo<Vec<Marker>>,
}

impl LiveViews {
    fn new(
        editor_text: RwSignal<String>,
        optimize: RwSignal<bool>,
        compiled: RwSignal<Compiled>,
        in_flight: RwSignal<Option<CompileKey>>,
        pending: RwSignal<bool>,
        starting: RwSignal<bool>,
    ) -> Self {
        let at_level = move |level: bool| {
            editor_text.with(|text| compiled.with(|compiled| compilation_of(compiled, text, level)))
        };
        let compilation = Memo::new(move |_| at_level(optimize.get()));
        let shown = Memo::new(move |previous: Option<&Option<Compilation>>| {
            let waiting = pending.get()
                || in_flight.with(|key| {
                    key.as_ref()
                        .is_some_and(|key| editor_text.with(|text| key.0 == *text))
                });
            compilation
                .with(|current| shown(current.as_ref(), previous.and_then(Option::as_ref), waiting))
        });
        // Squiggles come only from the editor's own text: the previous
        // output's diagnostics would mark the wrong places.
        let markers = Memo::new(move |_| {
            compilation.with(|compilation| {
                compilation
                    .iter()
                    .flat_map(|compilation| &compilation.diagnostics)
                    .filter_map(marker)
                    .collect()
            })
        });
        let status = Memo::new(move |_| {
            let level = optimize.get();
            let pending = pending.get();
            let starting = starting.get();
            editor_text.with(|text| {
                in_flight.with(|in_flight| {
                    compiled.with(|compiled| {
                        live_status(compiled, text, level, in_flight.as_ref(), pending, starting)
                    })
                })
            })
        });
        Self {
            status,
            compilation,
            shown,
            other_level: Memo::new(move |_| at_level(!optimize.get())),
            markers,
        }
    }
}

/// Why the compiler Worker gave no answer, for the page: a timeout, or a
/// trap whose cause is known, says so itself.
fn compiler_failure(error: String) -> String {
    if error == engine::COMPILER.timed_out()
        || error == engine::COMPILER.stopped()
        || error == engine::OUT_OF_MEMORY
        || error == engine::TOO_DEEP
        || error.starts_with(engine::INTERNAL_ERROR)
    {
        error
    } else {
        format!("bcc's in-browser compiler crashed ({error}).")
    }
}

/// The links between the source and the output of `stage` in `compilation`.
fn links_memo(
    compilation: Memo<Option<Compilation>>,
    stage: RwSignal<Stage>,
    unfolded: RwSignal<Unfolded>,
    source: RwSignal<String>,
) -> Memo<crate::pipeline::Links> {
    Memo::new(move |_| links_memo_value(compilation, stage, unfolded, source))
}

/// The links between the source and the output of `stage` at the other
/// level, `other`, while the two levels are shown side by side (`compare`).
fn other_links_memo(
    other: Memo<Option<Compilation>>,
    compare: RwSignal<bool>,
    stage: RwSignal<Stage>,
    unfolded: RwSignal<Unfolded>,
    source: RwSignal<String>,
) -> Memo<crate::pipeline::Links> {
    Memo::new(move |_| {
        if !compare.get() || !stage.get().differs_by_level() {
            return crate::pipeline::Links::default();
        }
        links_memo_value(other, stage, unfolded, source)
    })
}

/// The value of [`links_memo`], tracking what it reads.
fn links_memo_value(
    compilation: Memo<Option<Compilation>>,
    stage: RwSignal<Stage>,
    unfolded: RwSignal<Unfolded>,
    source: RwSignal<String>,
) -> crate::pipeline::Links {
    let stage = stage.get();
    let unfolded = unfolded.get();
    compilation.with(|compilation| {
        compilation.as_ref().map_or_default(|compilation| {
            source.with(|source| crate::pipeline::links(stage, compilation, source, unfolded))
        })
    })
}

/// The compilation of `text` at `level`, if it succeeded, with the
/// precomputed LLVM IR for a lesson.
fn compilation_of(compiled: &Compiled, text: &str, level: bool) -> Option<Compilation> {
    let lesson = demos::matching(text).map(|demo| demo.name.trim_end_matches(".c"));
    let llvm = lesson.and_then(|stem| crate::pipeline::precomputed_llvm(stem, level));
    let llvm_lines = lesson
        .filter(|_| llvm.is_some())
        .map_or_default(|stem| crate::pipeline::precomputed_llvm_lines(stem, level));
    let live = lookup(compiled, text, level).and_then(|result| result.as_ref().ok().cloned());
    match (live, llvm) {
        | (Some(live), llvm) => Some(Compilation {
            llvm: llvm.map(str::to_owned),
            llvm_lines,
            ..live
        }),
        | (None, Some(llvm)) => Some(Compilation {
            llvm: Some(llvm.to_owned()),
            llvm_lines,
            ..Compilation::default()
        }),
        | (None, None) => None,
    }
}

/// What the pipeline view shows: the `current` compilation or, while the
/// next one compiles (`waiting`), the `previous` output, so typing does not
/// make the view flicker. Only `current` places markers and links and lists
/// the diagnostics and the optimizer's changes: the previous output came
/// from other text or another level.
fn shown(
    current: Option<&Compilation>,
    previous: Option<&Compilation>,
    waiting: bool,
) -> Option<Compilation> {
    let live = current.is_some_and(|current| current.tokens.is_some());
    match previous {
        | Some(previous) if waiting && !live => Some(Compilation {
            // A lesson's precomputed LLVM IR is always current.
            llvm: current.and_then(|current| current.llvm.clone()),
            llvm_lines: current.map_or_else(Vec::new, |current| current.llvm_lines.clone()),
            ..previous.clone()
        }),
        | _ => current.cloned(),
    }
}

/// Keeps `result` for `key` as the newest entry of `compiled`, which holds
/// at most [`COMPILED_LIMIT`]: each entry holds its whole source, so a
/// refused program, kept so its refusal shows, is let go like any other.
fn remember(compiled: &mut Compiled, key: CompileKey, result: Result<Compilation, String>) {
    compiled.retain(|(known, _)| *known != key);
    compiled.push((key, result));
    if compiled.len() > COMPILED_LIMIT {
        _ = compiled.remove(0);
    }
}

/// The newest result for `text` at `level`.
fn lookup<'a>(
    compiled: &'a Compiled,
    text: &str,
    level: bool,
) -> Option<&'a Result<Compilation, String>> {
    compiled
        .iter()
        .rev()
        .find(|((source, optimized), _)| source == text && *optimized == level)
        .map(|(_, result)| result)
}

/// Why the compiler Worker could not compile `text` at either level. Text
/// that crashed or stalled the compiler is not tried again at the other
/// level, so the failure stands for both.
fn failure<'a>(compiled: &'a Compiled, text: &str) -> Option<&'a str> {
    compiled
        .iter()
        .rev()
        .find_map(|((source, _), result)| {
            (source == text).then_some(result.as_ref().err()).flatten()
        })
        .map(String::as_str)
}

/// The next compilation the page needs for `text`: at the selected `level`,
/// then at the other level when the `pipeline` view shows the optimizer
/// comparison. Outside the pipeline view a bundled lesson needs none.
fn wanted(compiled: &Compiled, text: &str, level: bool, pipeline: bool) -> Option<CompileKey> {
    if (!pipeline && demos::matching(text).is_some()) || failure(compiled, text).is_some() {
        return None;
    }
    [Some(level), pipeline.then_some(!level)]
        .into_iter()
        .flatten()
        .find(|&level| lookup(compiled, text, level).is_none())
        .map(|level| (text.to_owned(), level))
}

/// The compiler's progress on `text` at `level`, given what it is compiling
/// (`in_flight`), whether typing is still being debounced (`pending`), and
/// whether its Worker is `starting`.
fn live_status(
    compiled: &Compiled,
    text: &str,
    level: bool,
    in_flight: Option<&CompileKey>,
    pending: bool,
    starting: bool,
) -> LiveCompile {
    let busy = if starting {
        LiveCompile::Loading
    } else {
        LiveCompile::Compiling
    };
    if in_flight.is_some_and(|key| key.0 == text && key.1 == level) {
        return busy;
    }
    match lookup(compiled, text, level) {
        | Some(Ok(_)) => LiveCompile::Compiled,
        | Some(Err(error)) => LiveCompile::Failed(error.clone()),
        | None => match failure(compiled, text) {
            | Some(error) => LiveCompile::Failed(error.to_owned()),
            // Anything else in flight finishes (or is dropped) first.
            | None if pending || in_flight.is_some() => busy,
            | None => LiveCompile::Idle,
        },
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

/// The editor's squiggles: the `pipeline`'s compilation's, or in the
/// WebAssembly view bcc's compilation of the text for that view, for
/// wasm32-wasip1, whose C library headers the x86-64 pipeline does not have.
fn markers(
    mode: RwSignal<Mode>,
    editor_text: RwSignal<String>,
    checks: RwSignal<wasm::Checks>,
    pipeline: Memo<Vec<Marker>>,
) -> Memo<Vec<Marker>> {
    Memo::new(move |_| {
        if mode.get() == Mode::Wasm {
            editor_text.with(|text| {
                checks.with(|checks| checks.diagnostics(text).iter().filter_map(marker).collect())
            })
        } else {
            pipeline.get()
        }
    })
}

/// A squiggle for `diagnostic`, if it is placed in the source.
fn marker(diagnostic: &CompileDiagnostic) -> Option<Marker> {
    let range = diagnostic.range?;
    Some(Marker {
        start_line:   range.line,
        start_column: range.column,
        end_line:     range.end_line,
        end_column:   range.end_column,
        severity:     diagnostic.severity.clone(),
        message:      diagnostic.message.clone(),
    })
}

/// One continuous box behind `range` (Monaco draws a range's `className` as
/// a single overlay, where an inline class would box each token).
/// The listing's arrows to code: each function a live object points to
/// has its name in its definition marked, with the pointers that lead there
/// written before it (`f, ops[0] →`), so a pointer to a function points into
/// the listing as a pointer to an object points into the cells.
fn function_marks(snapshot: &Snapshot) -> Vec<Decoration> {
    let mut targets: Vec<(Range, String, Vec<String>)> = Vec::new();
    let mut note = |value: &Value, label: String| {
        if let Value::Function {
            name, at: Some(at), ..
        } = value
        {
            match targets.iter_mut().find(|(target, ..)| target == at) {
                | Some((.., labels)) => labels.push(label),
                | None => targets.push((*at, name.clone(), vec![label])),
            }
        }
    };
    // Every scalar of every live object, named as the source would name it:
    // `f`, `ops[1]`, `s.apply`, `o.in.f`, `c.ops[0]`.
    let mut pending: Vec<(&Value, String)> = snapshot
        .objects
        .iter()
        .filter(|object| object.alive)
        .map(|object| (&object.value, object.name.clone()))
        .collect();
    pending.reverse();
    while let Some((value, label)) = pending.pop() {
        match value {
            | Value::Array { elements } =>
                for (index, element) in elements.iter().enumerate().rev() {
                    pending.push((element, format!("{label}[{index}]")));
                },
            | Value::Struct { members, .. } =>
                for member in members.iter().rev() {
                    pending.push((&member.value, format!("{label}.{}", member.name)));
                },
            | value => note(value, label),
        }
    }
    targets
        .into_iter()
        .map(|(at, name, labels)| {
            let shown = match labels.as_slice() {
                | [a, b, c, _, ..] => format!("{a}, {b}, {c} +{} →", labels.len() - 3),
                | _ => format!("{} →", labels.join(", ")),
            };
            let verb = if labels.len() == 1 { "points" } else { "point" };
            Decoration {
                start_line: at.start.line,
                start_column: at.start.column,
                end_line: at.end.line,
                end_column: at.end.column,
                inline_class: Some("fn-target".to_owned()),
                hover: Some(format!(
                    "{} {verb} to the function {name}",
                    labels.join(", ")
                )),
                before: Some(shown),
                before_class: Some("fn-pointer-label".to_owned()),
                z_index: 4,
                ..Decoration::default()
            }
        })
        .collect()
}

/// The listing's arrow for a jump: while a `goto` leaves blocks and
/// jumps, its label is marked, with where the goto is written before it
/// (`goto, line 7 →`), so the jump reads from the goto to its target.
fn goto_mark(snapshot: &Snapshot) -> Option<Decoration> {
    let label = snapshot.event.label?;
    let goto = snapshot.event.range?.start;
    let from = goto.line;
    // Compare positions, not lines: a goto may share its label's line.
    let direction = if (goto.line, goto.column) > (label.start.line, label.start.column) {
        "back"
    } else {
        "ahead"
    };
    Some(Decoration {
        start_line: label.start.line,
        start_column: label.start.column,
        end_line: label.end.line,
        end_column: label.end.column,
        inline_class: Some("jump-target".to_owned()),
        hover: Some(format!(
            "The goto on line {from} jumps {direction} to this label"
        )),
        before: Some(format!("goto, line {from} →")),
        before_class: Some("jump-label".to_owned()),
        z_index: 4,
        ..Decoration::default()
    })
}

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
                <span class="wordmark">"bcc"</span>
                <span class="brand-subtitle">"How C runs"</span>
            </a>
            <nav class="site-nav" aria-label="Sections">
                <a href="#lessons">"Lessons"</a>
                <a href="#workbench">"Workbench"</a>
                <a href="#about">"How it works"</a>
                <a href="#glossary">"Glossary"</a>
            </nav>
            <div class="header-actions">
                // A two-position switch: daylight, or the device under low
                // light.
                <button
                    type="button"
                    class="theme-toggle"
                    aria-label="Dark theme"
                    aria-pressed=move || (state.theme.get() == "dark").to_string()
                    title="Toggle light and dark theme"
                    on:click=move |_| state.toggle_theme()
                >
                    <span class="theme-position is-light" aria-hidden="true">"Light"</span>
                    <span class="theme-position is-dark" aria-hidden="true">"Dark"</span>
                </button>
                <a class="header-link" href=REPOSITORY_URL>"GitHub"<span class="sr-only">": bcc’s source"</span></a>
            </div>
        </header>
    }
}

#[component]
fn Hero(state: State) -> impl IntoView {
    view! {
        <section class="hero" id="top">
            <div class="hero-text">
                <p class="eyebrow">"bcc · an interactive introduction to compilers and the C abstract machine"</p>
                <h1>"What really happens when C runs?"</h1>
                // The forms the compiler gives the program are words a
                // newcomer may not know: each links to its glossary entry
                // and shows its definition on hover or focus.
                <p class="lede">
                    "The C standard describes an "<em>"abstract machine"</em>", not your CPU: objects with \
                     lifetimes, values read and stored, rules for when each effect happens. Step through programs \
                     on a tape, one evaluation at a time, then watch bcc, a C compiler in Rust, turn them into "
                    <FormTerm id="token" written="tokens" />", a "<FormTerm id="syntax-tree" written="syntax tree" />", "
                    <FormTerm id="ir" written="IR" />", "<FormTerm id="assembly" written="assembly" />" and "
                    <FormTerm id="machine-code" written="machine code" />"."
                </p>
                // Each step is a link to where it happens; the second, which
                // starts the first lesson, is where a first visit begins.
                <ol class="hero-steps" aria-label="How to use this page">
                    <li>
                        <a href="#lessons"><span class="hero-step-number">"1"</span>"Pick a lesson"</a>
                    </li>
                    <li>
                        <a class="is-primary" href="#workbench" on:click=move |_| state.open_view(Mode::Machine, None)>
                            <span class="hero-step-number">"2"</span>"Step through it"<kbd class="hero-key" aria-hidden="true">"→"</kbd>
                        </a>
                    </li>
                    <li>
                        // The pipeline from its first stage; a bundled lesson
                        // gives way to the one that walks a program through
                        // every stage.
                        <a href="#workbench" on:click=move |_| state.follow_path()>
                            <span class="hero-step-number">"3"</span>"Follow a program through the compiler"
                        </a>
                    </li>
                </ol>
            </div>
            <ReadingMap state />
        </section>
    }
}

/// A word of the landing for one of the forms the compiler gives a program:
/// a link to its glossary entry, with the entry's definition under the word
/// while a mouse is over it or the keyboard focus is on it (Escape hides
/// it). A touch screen has no hover, so a first tap shows the definition,
/// with a link to the entry in it, and a second tap on the word follows the
/// link; a tap elsewhere, or Escape, hides it. A screen reader reads the
/// definition as the link's description.
#[component]
fn FormTerm(id: &'static str, written: &'static str) -> impl IntoView {
    // The page is rendered at build time, so a word without an entry fails
    // the build.
    let term = COMPILER_TERMS
        .iter()
        .find(|term| term.id == id)
        .expect("each form the landing names is a compiler term");
    let open = RwSignal::new(false);
    let place = RwSignal::new(None::<FormTipPlace>);
    let word = NodeRef::<leptos::html::Span>::new();
    // Whether the pointer that pressed the word is a touch, and whether the
    // definition showed before it did: a tap that finds it hidden shows it
    // instead of following the link.
    let pressed = StoredValue::new((false, false));
    // Whether a press inside the word, its definition included, is under
    // way: a tap on the definition's text moves the focus to the page,
    // which must not hide it. A click or a key ends the press. The
    // definition it keeps open hides when the focus lands elsewhere.
    let held = StoredValue::new(false);
    let kept = StoredValue::new(false);
    let show = move || {
        place.set(form_tip_place(word));
        open.set(true);
    };
    Effect::new(move || {
        // The page lives as long as the listeners.
        let _handle = window_event_listener(ev::pointerdown, move |event| {
            if open.get_untracked() && !within(word, event.target()) {
                held.set_value(false);
                kept.set_value(false);
                open.set(false);
            }
        });
        let _handle = window_event_listener(ev::focusin, move |event| {
            if kept.get_value() && !within(word, event.target()) {
                kept.set_value(false);
                open.set(false);
            }
        });
        // Escape hides the definition wherever the focus is: a tap on its
        // text keeps it open with the focus on the page, out of the word's
        // own key handler's reach.
        let _handle = window_event_listener(ev::keydown, move |event| {
            if event.key() == "Escape" && open.get_untracked() {
                held.set_value(false);
                kept.set_value(false);
                open.set(false);
            }
        });
        // A resize or a rotation moves the word, so an open definition
        // follows it.
        let _handle = window_event_listener(ev::resize, move |_| {
            if open.get_untracked() {
                place.set(form_tip_place(word));
            }
        });
    });
    let tip = format!("form-tip-{id}");
    view! {
        <span
            class="form-term"
            node_ref=word
            on:pointerenter=move |event| {
                if event.pointer_type() == "mouse" {
                    show();
                }
            }
            on:pointerleave=move |event| {
                if event.pointer_type() == "mouse" {
                    open.set(false);
                }
            }
            on:pointerdown=move |_| held.set_value(true)
            on:click=move |_| held.set_value(false)
            on:focusin=move |_| {
                kept.set_value(false);
                show();
            }
            // Focus that moves onto the definition's own link, or leaves
            // for the page under a press on the definition, keeps it open.
            on:focusout=move |event| {
                if !within(word, event.related_target()) {
                    if held.get_value() {
                        kept.set_value(true);
                    } else {
                        open.set(false);
                    }
                }
            }
            on:keydown=move |event| {
                held.set_value(false);
                if event.key() == "Escape" && open.get_untracked() {
                    event.stop_propagation();
                    open.set(false);
                }
            }
        >
            <a
                href=format!("#term-{id}")
                aria-describedby=tip.clone()
                on:pointerdown=move |event| pressed.set_value((event.pointer_type() != "mouse", open.get_untracked()))
                on:click=move |event| {
                    let (touch, was_open) = pressed.get_value();
                    pressed.set_value((false, false));
                    if touch && !was_open {
                        event.prevent_default();
                        show();
                    }
                }
            >
                {written}
            </a>
            <span
                class="form-tip"
                hidden=move || !open.get()
                style:left=move || place.get().map(|place| format!("{}px", place.left))
                style:width=move || place.get().map(|place| format!("{}px", place.width))
                style:top=move || place.get().map(|place| format!("{}px", place.top))
            >
                <span class="form-tip-text" role="tooltip" id=tip>
                    <b class="form-tip-word">{term.name}</b>
                    {term.definition}
                </span>
                // For the pointer: the keyboard and screen readers have the
                // word's own link to the same entry.
                <a class="form-tip-link" href=format!("#term-{id}") tabindex="-1" aria-hidden="true">
                    "Read it in the glossary →"
                </a>
            </span>
        </span>
    }
}

/// Where a landing word's definition goes in the lede: under the word, as
/// wide as the lede allows up to 340 px, and moved left as far as keeps it
/// inside the lede.
#[derive(Clone, Copy, PartialEq)]
struct FormTipPlace {
    left:  f64,
    width: f64,
    top:   f64,
}

#[cfg(feature = "hydrate")]
fn form_tip_place(word: NodeRef<leptos::html::Span>) -> Option<FormTipPlace> {
    let word = word.get_untracked()?;
    let lede = word.closest(".lede").ok()??.get_bounding_client_rect();
    let link = word.first_element_child()?.get_bounding_client_rect();
    let width = lede.width().min(340.0);
    Some(FormTipPlace {
        left: (link.left() - lede.left()).clamp(0.0, (lede.width() - width).max(0.0)),
        width,
        top: link.bottom() - lede.top() + 6.0,
    })
}

#[cfg(not(feature = "hydrate"))]
fn form_tip_place(_word: NodeRef<leptos::html::Span>) -> Option<FormTipPlace> {
    None
}

/// Whether `target` is inside the element `node` holds.
fn within(node: NodeRef<leptos::html::Span>, target: Option<web_sys::EventTarget>) -> bool {
    let target = target.and_then(|target| target.dyn_into::<web_sys::Node>().ok());
    node.get_untracked()
        .is_some_and(|node| node.contains(target.as_ref()))
}

/// How to read the page, drawn as a train graph (Marey's time-distance
/// diagram): the stations are the forms a program takes, top to bottom, and
/// one statement travels across them. The abstract machine runs it where it
/// stands, as the C standard defines it; the compiler carries it down,
/// station by station, to the bytes a processor runs. Each station opens
/// the view that shows it.
#[component]
fn ReadingMap(state: State) -> impl IntoView {
    // Each station: its row, name, what the statement looks like there, the
    // stage that shows it and its glossary entry, if it has one.
    const STATIONS: [(&str, &str, Stage, Option<&str>); 6] = [
        (
            "Tokens",
            "total = total + next ;",
            Stage::Tokens,
            Some("token"),
        ),
        (
            "Syntax tree",
            "=(total, +(total, next))",
            Stage::SyntaxTree,
            Some("syntax-tree"),
        ),
        ("Types", "int = int + int", Stage::SemanticTypes, None),
        ("IR", "load · add · store", Stage::Ir, Some("ir")),
        ("Assembly", "addl %ecx, %eax", Stage::Asm, Some("assembly")),
        (
            "Machine code",
            "01 c8",
            Stage::MachineCode,
            Some("machine-code"),
        ),
    ];
    let row = |index: usize| 74.0 + crate::panels::px(index) * 34.0;
    let stop = |index: usize| 150.0 + crate::panels::px(index) * 52.0;
    // The compiler's train: a dwell at each station, then down to the next.
    let mut track = format!("M118 {:.0}", row(0) - 34.0);
    for index in 0..STATIONS.len() {
        _ = std::fmt::Write::write_fmt(
            &mut track,
            format_args!(" L{:.0} {:.0} h18", stop(index), row(index)),
        );
    }
    view! {
        <figure class="reading-map" aria-labelledby="map-title">
            <figcaption class="map-caption" id="map-title">
                <b>"Figure 0"</b>" — How to read this page: one statement, two journeys"
            </figcaption>
            <div class="map-screen">
                <div class="map-screen-inner">
                <svg class="map-graph" viewBox="0 0 560 300" role="presentation" aria-hidden="true">
                    <line class="map-station" x1="112" y1="24" x2="548" y2="24" />
                    <line class="map-station" x1="112" y1="40" x2="548" y2="40" />
                    {(0..STATIONS.len()).map(|index| view! {
                        <line class="map-station" x1="112" y1=row(index) x2="548" y2=row(index) />
                    }).collect_view()}
                    // The abstract machine runs the statement where it stands.
                    <path class="map-run" d="M118 40 H548" />
                    {[(150.0, "read total"), (250.0, "read next"), (346.0, "add"), (420.0, "store total")].into_iter().map(|(x, what)| view! {
                        <line class="map-tick" x1=x y1="34" x2=x y2="46" />
                        <text class="map-event" x=x + 4.0 y="33">{what}</text>
                    }).collect_view()}
                    <path class="map-train" d=track />
                    {STATIONS.iter().enumerate().map(|(index, (_, form, ..))| view! {
                        <circle class="map-stop" cx=stop(index) cy=row(index) r="3" />
                        <text class="map-form" x=stop(index) + 24.0 y=row(index) + 4.0>{*form}</text>
                    }).collect_view()}
                    <text class="map-source" x="118" y="19">"lesson.c: total = total + next;"</text>
                </svg>
                <ol class="map-labels">
                    <li class="is-source"><span>"C source"</span></li>
                    <li class="is-machine">
                        <a href="#workbench" on:click=move |_| state.open_view(Mode::Machine, None)>"Abstract machine"</a>
                    </li>
                    // A mouse over a station shows what the form is, in the
                    // glossary's words (the lede's definitions), or for the
                    // types, which have no entry, in the stage's; a screen
                    // reader reads it as the link's description.
                    {STATIONS.iter().enumerate().map(|(index, (name, _, stage, term))| {
                        let stage = *stage;
                        let meaning = COMPILER_TERMS
                            .iter()
                            .find(|entry| Some(entry.id) == *term)
                            .map_or_else(|| stage.handoff(), |entry| entry.definition);
                        view! {
                            <li style:--row=index.to_string()>
                                <a href="#workbench" title=meaning on:click=move |_| state.open_view(Mode::Pipeline, Some(stage))>{*name}</a>
                            </li>
                        }
                    }).collect_view()}
                </ol>
                </div>
            </div>
            <p class="map-legend">
                <span class="map-key is-run">"Orange: "</span>"the abstract machine gives the statement its meaning, one evaluation at a time. "
                <span class="map-key is-train">"White: "</span>"the compiler carries it down to machine code. "
                <a href="#workbench" on:click=move |_| state.open_view(Mode::Wasm, None)>"WebAssembly"</a>
                " runs bcc’s output for real. Terms like "
                <a href="#term-lvalue">"lvalue"</a>" and "<a href="#term-sequence-point">"sequence point"</a>
                " are in the "<a href="#glossary">"glossary"</a>"."
            </p>
        </figure>
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
                <h2 id="lessons-title"><span class="section-number">"Table 1"</span>"Lessons"</h2>
                <p>"Each lesson is a small, complete C program. Edit any of them, or open your own .c file."</p>
            </div>
            {Part::ALL
                .into_iter()
                .enumerate()
                .map(|(number, part)| {
                    let first = DEMOS.iter().position(|demo| demo.part == part).unwrap_or(0);
                    let heading = format!("lessons-part-{}", number + 1);
                    let labelled = heading.clone();
                    view! {
                        <section class="lesson-part" aria-labelledby=labelled>
                            <h3 class="lesson-part-title" id=heading>
                                <b>{format!("Part {}", number + 1)}</b>
                                {format!(" — {}", part.title())}
                            </h3>
                            <p class="lesson-part-summary">{part.summary()}</p>
                            <ol class="lesson-list" start=(first + 1).to_string()>
                                {DEMOS
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, demo)| demo.part == part)
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
                                                    on:click=move |_| state.choose_lesson(demo)
                                                >
                                                    <span class="lesson-number">{format!("{:02}", index + 1)}</span>
                                                    <span class="lesson-title">{demo.title}</span>
                                                    <span class="lesson-summary">{demo.summary}</span>
                                                    <span class="lesson-topics">{demo.topics.join(" / ")}</span>
                                                </button>
                                            </li>
                                        }
                                    })
                                    .collect_view()}
                            </ol>
                        </section>
                    }
                })
                .collect_view()}
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
                .with(|error| error.as_ref().map_or_default(read))
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
    let toolbar = NodeRef::<leptos::html::Div>::new();
    publish_height(toolbar);
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
    let tab = move |mode: Mode, label: &'static str, short: &'static str, icon: &'static str| {
        view! {
            <button
                type="button"
                class="mode-tab"
                aria-pressed=move || (state.mode.get() == mode).to_string()
                aria-controls="insight"
                on:click=move |_| state.set_mode(mode)
            >
                <span class="mode-icon" aria-hidden="true">{icon}</span>
                <span class="mode-label-long">{label}</span>
                <span class="mode-label-short" aria-hidden="true">{short}</span>
            </button>
        }
    };
    view! {
        <div class="toolbar" node_ref=toolbar>
            // Three pressed-or-not keys, not a tab list: the arrow keys step
            // the machine wherever the focus is, even on these keys, which
            // a chosen lesson focuses so that `→` steps at once; a tab
            // list's arrows would move between the keys instead.
            <div class="mode-tabs" role="group" aria-label="View">
                {tab(Mode::Machine, "Abstract machine", "Machine", "1")}
                {tab(Mode::Pipeline, "Compiler pipeline", "Compiler", "2")}
                {tab(Mode::Wasm, "WebAssembly", "Wasm", "3")}
            </div>
            <Show when=move || state.mode.get() == Mode::Machine>
                <Encoders state />
            </Show>
            <div class="transport" role="group" aria-label="Execution controls" hidden=move || state.mode.get() != Mode::Machine>
                <button
                    type="button"
                    class="button ghost"
                    title="Reset to the initial state (Shift+R)"
                    disabled=move || state.locked_tracked() || !state.has_engine.get()
                    on:click=move |_| state.reset()
                >
                    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M4 10a6 6 0 1 0 1.8-4.3M4 3.5v3.2h3.2" /></svg>
                    <span class="button-label">"Reset"</span>
                </button>
                <button
                    type="button"
                    class="button"
                    title="Back one step (←)"
                    disabled=move || state.locked_tracked() || state.edited.get() || state.cursor.get().is_none_or(|cursor| cursor == 0)
                    on:click=move |_| state.back()
                >
                    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M12 5l-5 5 5 5" /></svg>
                    <span class="button-label">"Back"</span>
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
                // Keeps its word on phones, as the WebAssembly view's Over
                // does: its icon alone would not say what it does.
                <button
                    type="button"
                    class="button labelled"
                    title=move || state.group.with(|group| match group {
                        | Some(group) if matches!(group.over, Some(evaluation::OverStop::At(_) | evaluation::OverStop::End)) =>
                            format!("Step over: run to the last step of {}, through any call it makes (O)", group.text),
                        | _ => "Step over: run to the last step of the next expression (O)".to_owned(),
                    })
                    disabled=move || state.locked_tracked() || !state.can_step()
                    on:click=move |_| state.over()
                >
                    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M4 13a6 6 0 0 1 11-3M15 6v4h-4" /></svg>
                    <span class="button-label">"Over"</span>
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
                    <span class="button-label">{run_label}</span>
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
                <span class="step-counter">
                    <span class="step-counter-label">"Step "</span>
                    <strong>{move || state.current.with(|s| s.as_ref().map_or_else(|| "–".into(), |s| s.step.to_string()))}</strong>
                    <span class="step-counter-total">{move || state.history.with(|history| history.last().map_or_default(|last| format!(" / {}", last.step)))}</span>
                </span>
            </div>
            <wasm::WasmTransport state hidden=Signal::derive(move || state.mode.get() != Mode::Wasm) />
            <wasm::StartAtMain state hidden=Signal::derive(move || state.mode.get() != Mode::Wasm) />
            <div class="run-status wasm-status" hidden=move || state.mode.get() != Mode::Wasm>
                <span class="status-pill" data-status=move || wasm::status(state.wasm).1>{move || wasm::status(state.wasm).0}</span>
                <span class="step-counter"><span class="step-counter-label">"Instruction "</span><strong>{move || state.wasm.current.with(|s| s.as_ref().map_or_else(|| "–".into(), |s| s.step.to_string()))}</strong></span>
            </div>
        </div>
    }
}

/// Bring the workbench to the top of the window for a lesson chosen in
/// Table 1 and, with `focus`, give the keyboard focus to the key of the view
/// it opened in, at the head of the control strip, so the reader goes on
/// from there rather than from the table. The page glides there as it does
/// to its links, or jumps when motion is reduced (the style sheet's
/// `scroll-behavior`).
fn show_workbench(focus: bool) {
    let _ = Timeout::new(0, move || {
        let document = document();
        if let Some(workbench) = document.get_element_by_id("workbench") {
            let options = web_sys::ScrollIntoViewOptions::new();
            options.set_block(web_sys::ScrollLogicalPosition::Start);
            options.set_behavior(web_sys::ScrollBehavior::Auto);
            workbench.scroll_into_view_with_scroll_into_view_options(&options);
        }
        if focus
            && let Some(tab) = document
                .query_selector(".mode-tab[aria-pressed=\"true\"]")
                .ok()
                .flatten()
                .and_then(|tab| tab.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let options = web_sys::FocusOptions::new();
            options.set_prevent_scroll(true);
            let _ = tab.focus_with_options(&options);
        }
    })
    .forget();
}

/// Undo the browser's scroll anchoring while a change of view resizes the
/// toolbar and the source pane above the narration (at once, and again when
/// the WebAssembly listing arrives): anchoring keeps the narration in place,
/// which scrolls the top of the source under the toolbar. A scroll by exactly
/// the change in what lies above the narration is anchoring and is undone;
/// any other scroll (the reader's, the focus being scrolled into view, a
/// revealed line) ends the hold, as does `HOLD_MS` passing.
fn hold_source_pane() {
    use std::cell::{
        Cell,
        RefCell,
    };

    use gloo_timers::callback::Interval;

    let document = document();
    let (Some(window), Some(workbench), Some(insight)) = (
        web_sys::window(),
        document.get_element_by_id("workbench"),
        document.get_element_by_id("insight"),
    ) else {
        return;
    };
    // What lies above the narration (the toolbar, and on narrow screens the
    // source pane), and the page's scroll.
    let measure = {
        let window = window.clone();
        move || {
            (
                insight.get_bounding_client_rect().top()
                    - workbench.get_bounding_client_rect().top(),
                window.scroll_y().unwrap_or_default(),
            )
        }
    };
    let last = Cell::new(measure());
    let held = Cell::new(true);
    let interval = Rc::new(RefCell::new(None));
    *interval.borrow_mut() = Some(Interval::new(HOLD_CHECK_MS, move || {
        if !held.get() {
            return;
        }
        let ((height, scroll), (last_height, last_scroll)) = (measure(), last.get());
        let (grown, scrolled) = (height - last_height, scroll - last_scroll);
        if grown.abs() >= 1.0 && (scrolled - grown).abs() < 2.0 {
            let options = web_sys::ScrollToOptions::new();
            options.set_top(-scrolled);
            options.set_behavior(web_sys::ScrollBehavior::Instant);
            window.scroll_by_with_scroll_to_options(&options);
            last.set((height, scroll - scrolled));
        } else {
            held.set(scrolled.abs() < 1.0);
            last.set((height, scroll));
        }
    }));
    Timeout::new(HOLD_MS, move || {
        interval.borrow_mut().take();
    })
    .forget();
}

/// How long a change of view watches for scroll anchoring, at most.
const HOLD_MS: u32 = 8000;
/// How often the hold compares the narration's offset and the page's scroll.
const HOLD_CHECK_MS: u32 = 50;

/// Publish the toolbar's height as `--toolbar-height` on the root element
/// whenever its controls wrap, so the sticky source pane, the WebAssembly
/// listing's height and the scroll padding all sit below it.
fn publish_height(toolbar: NodeRef<leptos::html::Div>) {
    Effect::new(move |observing: Option<bool>| {
        if observing == Some(true) {
            return true;
        }
        let Some(element) = toolbar.get() else {
            return false;
        };
        let publish = move || {
            if let Some(root) = document()
                .document_element()
                .and_then(|root| root.dyn_into::<web_sys::HtmlElement>().ok())
            {
                _ = root.style().set_property(
                    "--toolbar-height",
                    &format!("{}px", element.offset_height()),
                );
            }
        };
        publish();
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(publish);
        if let Ok(observer) = web_sys::ResizeObserver::new(closure.as_ref().unchecked_ref())
            && let Some(element) = toolbar.get_untracked()
        {
            observer.observe(&element);
        }
        // The toolbar lives as long as the page.
        closure.forget();
        true
    });
}

/// Keeps `on_screen` up to date with whether any of the workbench is in the
/// viewport. On a phone the execution keys are fixed to the screen's edge, and
/// away from the workbench they would cover the page and step nothing in sight.
fn watch_workbench(on_screen: RwSignal<bool>) {
    let Some(workbench) = document().get_element_by_id("workbench") else {
        return;
    };
    let closure = wasm_bindgen::closure::Closure::<dyn FnMut(js_sys::Array)>::new(
        move |entries: js_sys::Array| {
            if let Ok(latest) = entries
                .at(-1)
                .dyn_into::<web_sys::IntersectionObserverEntry>()
            {
                on_screen.set(latest.is_intersecting());
            }
        },
    );
    if let Ok(observer) = web_sys::IntersectionObserver::new(closure.as_ref().unchecked_ref()) {
        observer.observe(&workbench);
    }
    // The workbench lives as long as the page.
    closure.forget();
}

#[component]
fn SourcePane(state: State) -> impl IntoView {
    let layers = Signal::derive(move || state.source_layers());
    let on_change = Callback::new(move |text: String| {
        if text != state.editor_text.get_untracked() {
            state.editor_text.set(text);
            state.compile_after_typing();
        }
    });
    let on_hover = Callback::new(move |line| state.pointing.hover(Side::Source, line));
    let on_cursor = Callback::new(move |line| state.pointing.cursor(Side::Source, line));
    let on_gutter = Callback::new(move |line| state.set_target(line));
    view! {
        <section
            class="pane source-pane"
            class:is-wasm=move || state.mode.get() == Mode::Wasm
            style:--source-lines=move || state.editor_text.with(|text| text.split('\n').count().max(4).to_string())
            aria-labelledby="source-title"
        >
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
                label="C source code. Edit it, then choose Run edited code. Tab moves to the next control; Alt+M makes Tab indent."
                decorations=layers
                markers=state.markers.into()
                on_change
                on_hover_line=on_hover
                on_cursor_line=on_cursor
                on_gutter_click=on_gutter
                reveal=state.reveal.into()
                follow_on_page=Signal::stored(true)
            />
            <footer class="source-legend" hidden=move || state.mode.get() != Mode::Machine>
                <span><i class="swatch swatch-current"></i>"just evaluated"</span>
                <span><i class="swatch swatch-next"></i>"next"</span>
                <span><i class="swatch swatch-target"></i>"click a line number to set a run target"</span>
                <span><i class="swatch swatch-squiggle"></i>"bcc error"</span>
            </footer>
            <p
                class="link-reading is-source"
                aria-hidden="true"
                hidden=move || state.mode.get() != Mode::Pipeline
            >
                {move || state.link_reading().map(|text| crate::panels::touch_words(&text))}
            </p>
            <footer class="source-legend" hidden=move || state.mode.get() != Mode::Pipeline>
                <span class="legend-hover"><i class="swatch swatch-link"></i>"hover or click a line to find what it became"</span>
                <span class="legend-tap"><i class="swatch swatch-link"></i>"tap a line to find what it became"</span>
                <span><i class="swatch swatch-squiggle"></i>"bcc error"</span>
            </footer>
            <Show when=move || state.mode.get() == Mode::Wasm>
                <WasmListing
                    wasm=state.wasm
                    theme=state.theme_name
                    pointing=state.pointing
                    on_breakpoint=Callback::new(move |line| state.wasm.toggle_breakpoint(state, line))
                />
            </Show>
            <p class="edit-hint" hidden=move || !state.edited.get() || state.mode.get() == Mode::Wasm>
                "You changed the program. The abstract machine still shows the last version you ran: choose "<strong>"Run edited code"</strong>" to restart it with your edits. bcc compiles your edits as you type, marks its errors with red squiggles, and the compiler pipeline view follows along."
            </p>
        </section>
    }
}

#[component]
fn About() -> impl IntoView {
    view! {
        <section class="about" id="about" aria-labelledby="about-title">
            <div class="section-heading">
                <h2 id="about-title"><span class="section-number">"Specifications"</span>"How it works"</h2>
                <p>"Different ways of giving a C program meaning, side by side."</p>
            </div>
            <div class="about-grid">
                <article>
                    <h3><span class="spec-number">"§1"</span>"The abstract machine"</h3>
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
                    <h3><span class="spec-number">"§2"</span>"The compiler"</h3>
                    <p>
                        "A real compiler doesn’t run your program; it translates it. bcc splices lines, cuts them into \
                         tokens, runs the preprocessor, builds a syntax tree, checks types, lowers everything to an SSA \
                         intermediate representation, optimizes it, and finally selects x86-64 instructions and encodes \
                         them as bytes."
                    </p>
                    <p>
                        "It is allowed to produce any code that behaves like the abstract machine would. That freedom is \
                         why undefined behavior matters: the compiler may assume it never happens."
                    </p>
                    <p>
                        "The WebAssembly view runs a compiled program for real: a small WebAssembly interpreter written in \
                         Rust executes it one instruction at a time, with its operand stack, locals, calls and linear \
                         memory in view, and each instruction linked back to its line of C."
                    </p>
                </article>
                <article>
                    <h3><span class="spec-number">"§3"</span>"Limits of this model"</h3>
                    <p>
                        "The engine runs a bounded subset of C, the part the lessons use: the integer types from char \
                         to unsigned long long, written in decimal, octal or hexadecimal, and the three floating \
                         types, at their x86-64 Linux sizes; enumerations and their constants, usable wherever C takes a \
                         constant, such as a case label or an array's size; the arithmetic, shift, bitwise, \
                         comparison and logical operators, with && and || stopping as soon as they know the result, \
                         ?: and the comma operator; pointers to numbers, chars, structures or other pointers; arrays \
                         of numbers, of chars or of pointers, and string literals; structures, nested or reached \
                         through pointers; const, casts and sizeof; a typedef of any of these types but a const \
                         one, at file or block scope; subtracting two pointers into one array and ordering two \
                         pointers into one object (p - q, p < q); file-scope and static objects, whose initializers may be constant expressions \
                         such as 1 << 3, string literals or the names of functions; functions that return a number, \
                         an enumeration, a structure, or a pointer to one of these objects, to a pointer or to a function, called \
                         anywhere in an expression; pointers to functions, kept in variables, tables, parameters and structure members, called \
                         as fp(x), (*fp)(x), s.f(x) or p->f(x), compared and tested for null; assignments, compound \
                         assignments, ++ and -- anywhere in an expression, as in x = y = 0, *p++ = c or while ((c = \
                         next()) != 0), except in an initializer list and before a . that selects a member of their \
                         value; if and else, while, do and for, switch with its case and default labels anywhere in \
                         its block, break, continue, and goto with the labels it jumps to; printf, puts and putchar \
                         after #include <stdio.h>; and #define of a name as one number, character or string. Outside it are, among others: a declaration directly in a \
                         switch's block; union and _Bool; void and static functions; a pointer to a library function; \
                         a static pointer initialized to an object's address; arrays of structures or of arrays; and \
                         the other library functions. A program that uses one of them stops before its first step, \
                         and its note says so. One that needs more of the preprocessor (another header, a \
                         function-like macro, #if) stops there too, and its note has a button to the compiler's \
                         Preprocessor stage, which shows what bcc makes of it."
                    </p>
                    <p>
                        "C lets an expression store into an object at most once between two sequence points, and \
                         read it then only to compute the value stored. The engine checks every read and store \
                         against the object it actually reaches, through *, [] and -> as well: two accesses to one \
                         object with no sequence point between them, one a store, stop the run there as undefined \
                         behavior, and where a variable is named twice, as in i = i++, the engine refuses the \
                         program before its first step. A store that reads the object it stores into to find where \
                         to store, as a[a[0]] = 1 does when a[0] is 0, also stops: C99's wording does not settle it."
                    </p>
                    <p>
                        "The engine evaluates operands left to right, one of the orders C allows: "
                        <a href=format!("{N1256_URL}#page=79") target="_blank" rel="noopener">
                            "C99 §6.5p3"<span class="sr-only">" (opens the standard in a new tab)"</span>
                        </a>
                        " leaves the order unspecified, so a compiler may choose another. A call's body runs before or \
                         after the other operands as a whole; when the call and another operand use the same object \
                         and one of them changes it, as in g + f() or g += f() when f changes g, or two calls both \
                         print, the order can decide the result, so the engine stops wherever that could happen \
                         rather than choose."
                    </p>
                    <p>
                        "The compiler pipeline runs bcc itself, compiled to WebAssembly, in your browser. Only LLVM IR \
                         comes from the native bcc build: the browser build has no LLVM, so it is shown for the \
                         bundled lessons only. The WebAssembly view steps what bcc compiles in your browser; for a \
                         lesson, it can also step the module Clang (wasi-sdk) built ahead of time, to compare. "
                        <a href=README_URL>"Read the full notes"</a>"."
                    </p>
                </article>
            </div>
        </section>
    }
}

/// What the step did on the tape, for screen readers: the objects it
/// created, wrote, read or ended, and their values.
fn tape_summary(state: &Snapshot) -> Option<String> {
    let parts: Vec<String> = state
        .event
        .access
        .iter()
        .filter_map(|access| {
            let object = state
                .objects
                .iter()
                .find(|object| object.id == access.object)?;
            let value = crate::panels::value_text(&object.value)
                .unwrap_or_else(|| "indeterminate".to_owned());
            Some(match access.kind.as_str() {
                | "create" => format!("track {} begins, {value}", object.name),
                | "declare" => format!("{} declared, {value}", object.name),
                | "write" => format!("track {} now {value}", object.name),
                | "read" => format!("track {} read, {value}", object.name),
                | "end" => format!("track {} ends", object.name),
                | _ => return None,
            })
        })
        .collect();
    (!parts.is_empty()).then(|| format!("Tape: {}.", parts.join("; ")))
}

/// The abstract machine view: the tape, the cells under its playhead, and
/// below them the note on the step beside the expression and the objects.
/// The playhead's line drops through the cells into the note.
#[component]
fn MachineView(state: State) -> impl IntoView {
    let reel = Memo::new(move |_| state.history.with(|history| Reel::record(history)));
    view! {
        <div class="machine-view" style:--head-x=move || state.head.get().map_or_else(String::new, |x| format!("{x:.1}px"))>
            <Deck
                history=state.history
                cursor=state.cursor
                reel
                source=state.source.into()
                zoom=state.zoom
                running=state.running.into()
                head=state.head
                on_jump=Callback::new(move |index| state.jump(index))
            />
            <div class="head-row">
                <span class="hanger" aria-hidden="true" hidden=move || state.head.with(Option::is_none)></span>
                <Cells snapshot=state.current />
            </div>
            <div class="notes-row">
                <Narration
                    snapshot=state.current
                    recent=state.recent
                    group=state.group
                    on_jump=Callback::new(move |index| state.jump(index))
                    on_preprocessor=Callback::new(move |()| state.open_stage(Stage::Preprocessor))
                    placeholder=state.placeholder
                    running=state.running.into()
                    reel
                />
                <div class="machine-side">
                    <Schematic snapshot=state.current />
                    <Output snapshot=state.current />
                    <Datasheet snapshot=state.current history=state.history reel />
                </div>
            </div>
        </div>
    }
}

/// The machine's encoders: STEP scrubs through the recorded steps, SPEED
/// sets Run's pace, ZOOM the tape's scale.
#[component]
fn Encoders(state: State) -> impl IntoView {
    let recorded = move || state.history.with(Vec::len);
    let step_position = Signal::derive(move || {
        let count = recorded().max(2) - 1;
        crate::panels::px(state.cursor.get().unwrap_or(0)) / crate::panels::px(count)
    });
    let speed_index = move || {
        let delay = state.run_delay.get();
        RUN_DELAYS
            .iter()
            .position(|&known| known >= delay)
            .unwrap_or(RUN_DELAYS.len() - 1)
    };
    // ZOOM turns a quarter larger or smaller per detent, from a sixty-fourth
    // of the fitted scale (where the whole run fits, however long) to four
    // times it.
    let zoom_position = Signal::derive(move || (state.zoom.get().log2() + 6.0) / 8.0);
    view! {
        <div class="encoders" role="group" aria-label="Tape controls">
            <Knob
                label="Step"
                name="Step through the recorded run"
                colour="blue"
                position=step_position
                reading=Signal::derive(move || state.current.with(|snapshot| snapshot.as_ref().map_or_else(|| "–".to_owned(), |snapshot| snapshot.step.to_string())))
                disabled=Signal::derive(move || state.locked_tracked() || state.edited.get())
                on_turn=Callback::new(move |detents: i32| {
                    let count = state.history.with_untracked(Vec::len);
                    if let Some(cursor) = state.cursor.get_untracked().filter(|_| count > 0) {
                        let target = if detents < 0 {
                            cursor.saturating_sub(detents.unsigned_abs() as usize)
                        } else {
                            (cursor + detents.unsigned_abs() as usize).min(count - 1)
                        };
                        state.jump(target);
                    }
                })
            />
            <Knob
                label="Speed"
                name="Run speed"
                colour="ochre"
                position=Signal::derive(move || 1.0 - crate::panels::px(speed_index()) / crate::panels::px(RUN_DELAYS.len() - 1))
                reading=Signal::derive(move || format!("{} ms", state.run_delay.get()))
                on_turn=Callback::new(move |detents: i32| {
                    // A detent right is faster: a shorter pause.
                    let index = if detents > 0 {
                        speed_index().saturating_sub(1)
                    } else {
                        (speed_index() + 1).min(RUN_DELAYS.len() - 1)
                    };
                    state.run_delay.set(RUN_DELAYS[index]);
                })
            />
            <Knob
                label="Zoom"
                name="Tape zoom"
                colour="grey"
                position=zoom_position
                reading=Signal::derive(move || {
                    let zoom = state.zoom.get();
                    if (zoom - 1.0).abs() < 0.01 { "fit".to_owned() } else { format!("×{zoom:.2}") }
                })
                on_turn=Callback::new(move |detents: i32| {
                    state.zoom.update(|zoom| *zoom = (*zoom * 1.25_f64.powi(detents)).clamp(1.0 / 64.0, 4.0));
                })
            />
        </div>
    }
}

pub mod links;
#[cfg(test)]
mod tests;
pub mod wasm;
