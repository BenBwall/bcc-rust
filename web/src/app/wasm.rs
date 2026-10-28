//! The WebAssembly mode's page state and controls: which module is loaded
//! for which source, the stepper's latest snapshot, the steps visited, and
//! the breakpoints set in the listing.
//!
//! Each program gets up to two modules ([`Modules`]): bcc's, compiled live
//! in the compiler Worker, and, for a bundled lesson, the one clang built
//! ahead of time ([`Precompiled`]). The view steps the learner's choice of
//! [`Compiler`] (bcc unless they pick clang) and falls back to the other
//! when there is only one, as when bcc refuses a program for WebAssembly.
//! The engine Worker steps the chosen module. It keeps the execution and
//! re-executes deterministically to go back, so the page holds only what it
//! shows.

// `State` is the page's bundle of `Copy` signal handles, passed by value as
// everywhere in `app.rs`.
#![expect(clippy::large_types_passed_by_value, reason = "See above.")]

use gloo_timers::future::TimeoutFuture;
use leptos::{
    prelude::*,
    task::spawn_local,
};

use super::State;
use crate::{
    editor::Decoration,
    engine,
    wasm::{
        Precompiled,
        WasmBackEnd,
        WasmControl,
        WasmModule,
        WasmProgram,
        WasmRun,
        WasmSnapshot,
    },
};

/// How long Run waits between source lines, so each is visible.
const RUN_YIELD_MS: u32 = 320;
/// Lines Run takes before it pauses.
const RUN_LIMIT: usize = 500;
/// Steps kept for the narration's "Earlier steps".
const TRAIL: usize = 6;
/// Why neither compiler has a module for the program.
const NO_MODULE: &str = "bcc could not compile this program to WebAssembly, and it is not one of \
                         the lessons, so there is no module made ahead of time either. Fix the \
                         error below, or step through the program on the abstract machine.";

/// Which compiler made a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compiler {
    /// bcc, compiling in the browser.
    Bcc,
    /// clang from wasi-sdk, ahead of time (the lessons only).
    Clang,
}

impl Compiler {
    pub fn name(self) -> &'static str {
        match self {
            | Self::Bcc => "bcc",
            | Self::Clang => "Clang (wasi-sdk)",
        }
    }

    fn other(self) -> Self {
        match self {
            | Self::Bcc => Self::Clang,
            | Self::Clang => Self::Bcc,
        }
    }
}

/// The modules a program has: bcc's, or why bcc refused it, and clang's
/// for a bundled lesson.
#[derive(Clone, Debug)]
pub struct Modules {
    pub bcc:   Result<WasmModule, String>,
    pub clang: Option<WasmModule>,
}

impl Modules {
    /// The module to step when the learner prefers `preferred`: that one if
    /// it exists, else the other compiler's.
    pub fn pick(&self, preferred: Compiler) -> Option<(Compiler, &WasmModule)> {
        self.get(preferred)
            .map(|module| (preferred, module))
            .or_else(|| {
                self.get(preferred.other())
                    .map(|module| (preferred.other(), module))
            })
    }

    pub fn get(&self, compiler: Compiler) -> Option<&WasmModule> {
        match compiler {
            | Compiler::Bcc => self.bcc.as_ref().ok(),
            | Compiler::Clang => self.clang.as_ref(),
        }
    }

    /// The compilers with a module, bcc first.
    pub fn available(&self) -> Vec<Compiler> {
        [Compiler::Bcc, Compiler::Clang]
            .into_iter()
            .filter(|&compiler| self.get(compiler).is_some())
            .collect()
    }

    /// bcc's diagnostics, when it refused the program.
    pub fn refusal(&self) -> Option<&str> {
        self.bcc.as_ref().err().map(String::as_str)
    }
}

#[derive(Clone, Copy)]
pub struct WasmState {
    pub program:      RwSignal<Option<WasmProgram>>,
    /// The source text the program was loaded for.
    loaded:           RwSignal<Option<String>>,
    /// The compiler the learner wants to see; bcc until they choose.
    pub preferred:    RwSignal<Compiler>,
    /// Who compiled the module the view steps.
    pub compiler:     RwSignal<Option<Compiler>>,
    /// The compilers with a module for the loaded source.
    pub available:    RwSignal<Vec<Compiler>>,
    /// bcc's diagnostics, when it refused the loaded source.
    pub refusal:      RwSignal<Option<String>>,
    /// The loaded source's modules, so switching compiler needs no
    /// recompilation.
    modules:          StoredValue<Option<(String, Modules)>>,
    pub current:      RwSignal<Option<WasmSnapshot>>,
    /// Steps visited before the current one, oldest first: step, title,
    /// category.
    trail:            RwSignal<Vec<(u64, String, String)>>,
    /// Why there is no program to show.
    pub unavailable:  RwSignal<Option<String>>,
    pub busy:         RwSignal<bool>,
    /// A module is being loaded.
    loading:          RwSignal<bool>,
    /// Counts loads, so a run or an answer that began with an earlier
    /// program is dropped rather than applied to the new one.
    generation:       RwSignal<u64>,
    pub running:      RwSignal<bool>,
    pub pausing:      RwSignal<bool>,
    /// Module offsets with breakpoints.
    pub breakpoints:  RwSignal<Vec<u32>>,
    /// The listing line under the pointer.
    pub hover_wat:    RwSignal<usize>,
    pub reveal_wat:   RwSignal<Option<usize>>,
    pub memory_tab:   RwSignal<&'static str>,
    pub memory_words: RwSignal<bool>,
}

impl WasmState {
    pub fn new() -> Self {
        Self {
            program:      RwSignal::new(None),
            loaded:       RwSignal::new(None),
            preferred:    RwSignal::new(Compiler::Bcc),
            compiler:     RwSignal::new(None),
            available:    RwSignal::new(Vec::new()),
            refusal:      RwSignal::new(None),
            modules:      StoredValue::new(None),
            current:      RwSignal::new(None),
            trail:        RwSignal::new(Vec::new()),
            unavailable:  RwSignal::new(None),
            busy:         RwSignal::new(false),
            loading:      RwSignal::new(false),
            generation:   RwSignal::new(0),
            running:      RwSignal::new(false),
            pausing:      RwSignal::new(false),
            breakpoints:  RwSignal::new(Vec::new()),
            hover_wat:    RwSignal::new(0),
            reveal_wat:   RwSignal::new(None),
            memory_tab:   RwSignal::new("stack"),
            memory_words: RwSignal::new(false),
        }
    }

    /// Load the module for the program the page runs, from the compiler
    /// the learner prefers, unless it is loaded.
    pub(super) fn ensure_loaded(self, state: State) {
        let source = state.source.get_untracked();
        let preferred = self.preferred.get_untracked();
        let current = self
            .loaded
            .with_untracked(|loaded| loaded.as_ref() == Some(&source))
            && (self.compiler.get_untracked() == Some(preferred)
                || !self
                    .available
                    .with_untracked(|available| available.contains(&preferred)));
        if current || self.loading.get_untracked() || !state.has_engine.get_untracked() {
            return;
        }
        // A run of the previous program stops, and answers still on their
        // way for it are dropped.
        self.generation.update(|generation| *generation += 1);
        self.loading.set(true);
        spawn_local(async move {
            let modules = self.modules_for(state, &source).await;
            let result = match modules.pick(preferred) {
                | Some((compiler, module)) => {
                    let module = module.clone();
                    match state.engine() {
                        | Ok(engine) => engine
                            .wasm_load(module)
                            .await
                            .map(|program| (compiler, program)),
                        | Err(error) => Err(error),
                    }
                },
                | None => Err(NO_MODULE.to_owned()),
            };
            self.loaded.set(Some(source.clone()));
            self.available.set(modules.available());
            self.refusal.set(modules.refusal().map(str::to_owned));
            self.modules.set_value(Some((source, modules)));
            self.trail.set(Vec::new());
            self.breakpoints.set(Vec::new());
            match result {
                | Ok((compiler, program)) => {
                    let snapshot = program.snapshot.clone();
                    self.unavailable.set(None);
                    self.compiler.set(Some(compiler));
                    self.program.set(Some(program));
                    self.show(state, snapshot);
                },
                | Err(error) => {
                    self.compiler.set(None);
                    self.program.set(None);
                    self.current.set(None);
                    self.unavailable.set(Some(error));
                },
            }
            self.loading.set(false);
            // The program, or the compiler chosen, may have changed while
            // this one loaded.
            self.ensure_loaded(state);
        });
    }

    /// `source`'s modules: bcc compiles it in the compiler Worker, and a
    /// lesson also has clang's.
    async fn modules_for(self, state: State, source: &str) -> Modules {
        if let Some(modules) = self.modules.with_value(|modules| {
            modules
                .as_ref()
                .filter(|(known, _)| known == source)
                .map(|(_, modules)| modules.clone())
        }) {
            return modules;
        }
        let bcc = match state.compiler().await {
            | Ok(compiler) => {
                let result = compiler.compile_wasm(source).await;
                // A compiler that timed out or trapped is gone; the next
                // compilation starts a fresh one.
                if !compiler.connected() {
                    state.compiler.set_value(None);
                }
                result.unwrap_or_else(|error| {
                    Err(
                        if error == engine::COMPILER.timed_out()
                            || error == engine::COMPILER.stopped()
                        {
                            error
                        } else {
                            format!("bcc's in-browser compiler crashed ({error}).")
                        },
                    )
                })
            },
            | Err(error) => Err(error),
        };
        Modules {
            bcc,
            clang: Precompiled.compile(source).ok(),
        }
    }

    /// Step `compiler`'s module from now on, if the program has one.
    pub fn choose(self, state: State, compiler: Compiler) {
        if self.preferred.get_untracked() == compiler
            && self.compiler.get_untracked() == Some(compiler)
        {
            return;
        }
        self.preferred.set(compiler);
        if self
            .available
            .with_untracked(|available| available.contains(&compiler))
        {
            state.announcement.set(format!(
                "Loading the module {} compiled. Stepping starts again from the first instruction.",
                compiler.name()
            ));
        }
        self.ensure_loaded(state);
    }

    /// Send `control` to the Worker and show the answer, unless another
    /// program was loaded meanwhile.
    async fn request(self, state: State, control: WasmControl) -> Result<(), String> {
        let generation = self.generation.get_untracked();
        let snapshot = state.engine()?.wasm(control).await?;
        if self.generation.get_untracked() == generation {
            self.show(state, snapshot);
        }
        Ok(())
    }

    fn show(self, state: State, snapshot: WasmSnapshot) {
        let step = snapshot.step;
        self.trail.update(|trail| {
            trail.retain(|(seen, ..)| *seen < step);
            if let Some(current) = self
                .current
                .get_untracked()
                .filter(|current| current.step < step)
                && trail.last().is_none_or(|(seen, ..)| *seen != current.step)
            {
                trail.push((
                    current.step,
                    current.narration.title.clone(),
                    current.narration.category.clone(),
                ));
            }
            let excess = trail.len().saturating_sub(TRAIL);
            trail.drain(..excess);
        });
        let place = snapshot.next.or(snapshot.last);
        self.reveal_wat.set(place.map(|place| place.wat_line));
        state.reveal.set(place.and_then(|place| place.source));
        let mut announcement = format!(
            "Step {}. {}. {}",
            snapshot.step,
            snapshot.narration.title.replace('`', ""),
            snapshot.narration.explanation.replace('`', "")
        );
        if snapshot.stopped.as_deref() == Some("breakpoint") {
            announcement.insert_str(0, "Stopped at a breakpoint. ");
        }
        state.announcement.set(announcement);
        self.current.set(Some(snapshot));
    }

    fn act<F>(self, state: State, operation: impl FnOnce() -> F + 'static)
    where
        F: Future<Output = Result<(), String>> + 'static,
    {
        if self.locked() {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            if let Err(error) = operation().await {
                self.fail(state, &error);
            }
            self.busy.set(false);
        });
    }

    fn fail(self, state: State, error: &str) {
        self.unavailable.set(Some(format!(
            "The stepper stopped: {error} Reset, or choose a lesson, to start again."
        )));
        state.announcement.set(error.to_owned());
        if !state.engine_connected() {
            state.drop_engine();
            state.show_unavailable(error);
            self.loaded.set(None);
        }
    }

    pub(super) fn run_command(self, state: State, run: WasmRun) {
        if !self.can_step_untracked() {
            return;
        }
        self.act(state, move || self.request(state, WasmControl::Run(run)));
    }

    pub(super) fn back(self, state: State) {
        let Some(step) = self
            .current
            .with_untracked(|current| current.as_ref().map(|c| c.step))
        else {
            return;
        };
        if step == 0 {
            return;
        }
        self.act(state, move || {
            self.request(state, WasmControl::GoTo(step - 1))
        });
    }

    pub(super) fn jump(self, state: State, step: u64) {
        self.act(state, move || self.request(state, WasmControl::GoTo(step)));
    }

    pub(super) fn reset(self, state: State) {
        self.trail.set(Vec::new());
        self.act(state, move || self.request(state, WasmControl::Reset));
    }

    /// Run line by line until the program stops, a breakpoint, or Pause.
    pub(super) fn toggle_run(self, state: State) {
        if self.running.get_untracked() {
            self.pausing.set(true);
            return;
        }
        if self.locked() || !self.can_step_untracked() {
            return;
        }
        self.running.set(true);
        self.pausing.set(false);
        let by_line = self
            .program
            .with_untracked(|program| program.as_ref().is_some_and(|program| program.has_lines));
        let generation = self.generation.get_untracked();
        spawn_local(async move {
            let mut taken = 0;
            while !self.pausing.get_untracked()
                && self.generation.get_untracked() == generation
                && self.can_step_untracked()
                && taken < RUN_LIMIT
            {
                let run = if by_line {
                    WasmRun::NextLine
                } else {
                    WasmRun::Step
                };
                if let Err(error) = self.request(state, WasmControl::Run(run)).await {
                    self.fail(state, &error);
                    break;
                }
                taken += 1;
                if self
                    .current
                    .with_untracked(|current| current.as_ref().is_some_and(|c| c.stopped.is_some()))
                {
                    break;
                }
                TimeoutFuture::new(RUN_YIELD_MS).await;
            }
            if taken == RUN_LIMIT && self.can_step_untracked() {
                state.announcement.set(format!(
                    "Paused after {RUN_LIMIT} lines. Run again to continue."
                ));
            }
            self.running.set(false);
            self.pausing.set(false);
        });
    }

    /// Toggle a breakpoint on listing line `line`, if it shows an
    /// instruction.
    pub(super) fn toggle_breakpoint(self, state: State, line: usize) {
        let Some(offset) = self.program.with_untracked(|program| {
            program
                .as_ref()
                .and_then(|program| program.lines.get(line.checked_sub(1)?))
                .and_then(|line| line.offset)
        }) else {
            return;
        };
        self.breakpoints.update(|breakpoints| {
            if let Some(index) = breakpoints.iter().position(|&known| known == offset) {
                breakpoints.remove(index);
            } else {
                breakpoints.push(offset);
            }
        });
        let offsets = self.breakpoints.get_untracked();
        state.announcement.set(format!(
            "{} breakpoint{} set.",
            offsets.len(),
            if offsets.len() == 1 { "" } else { "s" }
        ));
        spawn_local(async move {
            if let Ok(engine) = state.engine() {
                _ = engine.wasm(WasmControl::Breakpoints(offsets)).await;
            }
        });
    }

    pub fn locked(self) -> bool {
        self.busy.get_untracked() || self.running.get_untracked() || self.loading.get_untracked()
    }

    pub fn locked_tracked(self) -> bool {
        self.busy.get() || self.running.get() || self.loading.get()
    }

    pub fn can_step(self) -> bool {
        self.current
            .with(|current| current.as_ref().is_some_and(WasmSnapshot::can_step))
    }

    fn can_step_untracked(self) -> bool {
        untrack(move || self.can_step())
    }

    /// Whether the WebAssembly view shows a program other than the
    /// editor's text.
    pub fn stale(self, text: &str) -> bool {
        self.loaded
            .with(|loaded| loaded.as_ref().is_some_and(|loaded| loaded != text))
    }

    /// Steps before the current one, newest first, for the narration.
    pub fn recent(self) -> Vec<(u64, String, String)> {
        self.trail
            .with(|trail| trail.iter().rev().take(4).cloned().collect())
    }

    /// The source editor's marks in this mode: the C line of the next
    /// instruction and of the last one, and the lines linked to the hovered
    /// listing line.
    pub fn source_marks(self) -> (Vec<Decoration>, Vec<Decoration>) {
        let mut execution = Vec::new();
        self.current.with(|current| {
            let Some(current) = current else {
                return;
            };
            let next = current.next.and_then(|place| place.source);
            let last = current.last.and_then(|place| place.source);
            if let Some(line) = next {
                execution.push(Decoration {
                    glyph_class: Some("wasm-arrow".into()),
                    ..Decoration::line(line, "exec-line wasm-c-line")
                });
            }
            if let Some(line) = last.filter(|&line| Some(line) != next) {
                execution.push(Decoration::line(line, "wasm-c-previous"));
            }
            if current.status == "trapped"
                && let Some(line) = last
            {
                execution.push(Decoration::line(line, "exec-line-fault"));
            }
        });
        let hovered = self.hover_wat.get();
        let links = if hovered > 0 {
            self.program.with(|program| {
                program
                    .as_ref()
                    .and_then(|program| program.source_for(hovered))
                    .map(|line| vec![Decoration::line(line, "link-line")])
                    .unwrap_or_default()
            })
        } else {
            Vec::new()
        };
        (execution, links)
    }

    /// The listing's marks: the instruction just executed, the next one,
    /// breakpoints, and the lines linked to the hovered C line.
    pub fn listing_marks(self, hover_source: usize) -> Vec<(&'static str, Vec<Decoration>)> {
        let mut execution = Vec::new();
        self.current.with(|current| {
            let Some(current) = current else {
                return;
            };
            let fault = current.status == "trapped";
            if let Some(last) = current.last {
                let class = if fault {
                    "exec-line-fault wasm-fault".to_owned()
                } else {
                    format!("wasm-last cat-{}", current.narration.category)
                };
                execution.push(Decoration::line(last.wat_line, &class));
            }
            if let Some(next) = current.next {
                execution.push(Decoration {
                    margin_class: Some("wasm-next-glyph".into()),
                    ..Decoration::line(next.wat_line, "wasm-next")
                });
            }
        });
        let breakpoints = self.program.with(|program| {
            let Some(program) = program else {
                return Vec::new();
            };
            self.breakpoints.with(|breakpoints| {
                breakpoints
                    .iter()
                    .filter_map(|&offset| program.wat_line_of(offset))
                    .map(|line| Decoration {
                        number_class: Some("wasm-breakpoint".into()),
                        ..Decoration::line(line, "wasm-breakpoint-line")
                    })
                    .collect()
            })
        });
        let links = if hover_source > 0 {
            self.program.with(|program| {
                program
                    .as_ref()
                    .map(|program| {
                        program
                            .wat_lines_for(hover_source)
                            .into_iter()
                            .map(|line| Decoration::line(line, "link-line"))
                            .collect()
                    })
                    .unwrap_or_default()
            })
        } else {
            Vec::new()
        };
        vec![
            ("execution", execution),
            ("breakpoints", breakpoints),
            ("links", links),
        ]
    }
}

/// The WebAssembly mode's controls.
#[component]
pub(super) fn WasmTransport(state: State, hidden: Signal<bool>) -> impl IntoView {
    let wasm = state.wasm;
    let ready = move || !wasm.locked_tracked() && wasm.can_step();
    let button = move |label: &'static str,
                       title: &'static str,
                       class: &'static str,
                       run: WasmRun,
                       icon: &'static str| {
        view! {
            <button
                type="button"
                class=format!("button {class}")
                title=title
                disabled=move || !ready()
                on:click=move |_| wasm.run_command(state, run)
            >
                {(!icon.is_empty()).then(|| view! {
                    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d=icon /></svg>
                })}
                {label}
            </button>
        }
    };
    view! {
        <div class="transport wasm-transport" role="group" aria-label="WebAssembly controls" hidden=move || hidden.get()>
            <button
                type="button"
                class="button ghost"
                title="Reset to the first instruction (Shift+R)"
                disabled=move || wasm.locked_tracked() || wasm.current.with(Option::is_none)
                on:click=move |_| wasm.reset(state)
            >
                <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M4 10a6 6 0 1 0 1.8-4.3M4 3.5v3.2h3.2" /></svg>
                "Reset"
            </button>
            <button
                type="button"
                class="button"
                title="Back one instruction (←)"
                disabled=move || wasm.locked_tracked() || wasm.current.with(|c| c.as_ref().is_none_or(|c| c.step == 0))
                on:click=move |_| wasm.back(state)
            >
                <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M12 5l-5 5 5 5" /></svg>
                "Back"
            </button>
            {button("Step", "Execute one instruction (→)", "primary", WasmRun::Step, "")}
            {button("Over", "Step over: run a call to its return (O)", "", WasmRun::StepOver, "M4 13a6 6 0 0 1 11-3M15 6v4h-4")}
            {button("Out", "Step out: run until this function returns (U)", "", WasmRun::StepOut, "M10 15V5M6 9l4-4 4 4")}
            {button("Next line", "Run to the next C source line (N)", "", WasmRun::NextLine, "M4 6h12M4 10h8M4 14h12")}
            <button
                type="button"
                class="button"
                title=move || if wasm.running.get() { "Pause (R)" } else { "Run line by line until the program ends or reaches a breakpoint (R)" }
                aria-pressed=move || wasm.running.get().to_string()
                disabled=move || wasm.busy.get() || wasm.loading.get() || (wasm.running.get() && wasm.pausing.get()) || (!wasm.running.get() && !wasm.can_step())
                on:click=move |_| wasm.toggle_run(state)
            >
                {move || if wasm.running.get() {
                    view! { <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M7 5v10M13 5v10" /></svg> }.into_any()
                } else {
                    view! { <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path class="fill" d="M6 4.5v11l9-5.5z" /></svg> }.into_any()
                }}
                {move || match (wasm.running.get(), wasm.pausing.get()) {
                    | (true, true) => "Pausing…",
                    | (true, false) => "Pause",
                    | (false, _) => "Run",
                }}
            </button>
            <button
                type="button"
                class="button ghost"
                hidden=move || state.target_line.get().is_none()
                disabled=move || !ready()
                on:click=move |_| {
                    if let Some(line) = state.target_line.get_untracked().and_then(|line| u32::try_from(line).ok()) {
                        wasm.run_command(state, WasmRun::RunToLine(line));
                    }
                }
            >
                {move || format!("Run to line {}", state.target_line.get().unwrap_or_default())}
            </button>
        </div>
    }
}

/// The status pill's text and kind in this mode.
pub fn status(wasm: WasmState) -> (String, String) {
    wasm.current.with(|current| match current {
        | None if wasm.unavailable.with(Option::is_some) => ("No module".into(), "idle".into()),
        | None => ("Loading…".into(), "idle".into()),
        | Some(current) => {
            let label = match current.status.as_str() {
                | "running" if current.stopped.as_deref() == Some("breakpoint") => "Breakpoint",
                | "running" if current.step == 0 => "Ready",
                | "running" => "Paused",
                | "finished" => "Finished",
                | "exited" => "Exited",
                | "trapped" => "Trapped",
                | _ => "Instruction limit",
            };
            let kind = match current.status.as_str() {
                | "running" => "running",
                | "finished" | "exited" => "finished",
                | _ => "error",
            };
            (label.to_owned(), kind.to_owned())
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(producer: &str) -> WasmModule {
        WasmModule {
            producer: producer.into(),
            ..WasmModule::default()
        }
    }

    #[test]
    fn a_lesson_offers_both_compilers_and_steps_the_one_chosen() {
        let modules = Modules {
            bcc:   Ok(module("bcc")),
            clang: Some(module("clang")),
        };
        assert_eq!(modules.available(), [Compiler::Bcc, Compiler::Clang]);
        let (compiler, picked) = modules.pick(Compiler::Bcc).unwrap();
        assert_eq!((compiler, picked.producer.as_str()), (Compiler::Bcc, "bcc"));
        let (compiler, picked) = modules.pick(Compiler::Clang).unwrap();
        assert_eq!(
            (compiler, picked.producer.as_str()),
            (Compiler::Clang, "clang")
        );
        assert_eq!(modules.refusal(), None);
    }

    #[test]
    fn edited_code_has_only_bccs_module_whatever_was_chosen() {
        let modules = Modules {
            bcc:   Ok(module("bcc")),
            clang: None,
        };
        assert_eq!(modules.available(), [Compiler::Bcc]);
        assert_eq!(modules.pick(Compiler::Clang).unwrap().0, Compiler::Bcc);
    }

    #[test]
    fn a_refused_lesson_falls_back_to_clang_and_keeps_bccs_diagnostic() {
        let modules = Modules {
            bcc:   Err("error: setjmp is not supported on WebAssembly".into()),
            clang: Some(module("clang")),
        };
        assert_eq!(modules.available(), [Compiler::Clang]);
        assert_eq!(modules.pick(Compiler::Bcc).unwrap().0, Compiler::Clang);
        assert!(modules.refusal().unwrap().contains("setjmp"));
        let refused = Modules {
            clang: None,
            ..modules
        };
        assert!(refused.pick(Compiler::Bcc).is_none());
        assert!(refused.available().is_empty());
    }
}
