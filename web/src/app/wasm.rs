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
    pipeline::CompileDiagnostic,
    wasm::{
        Precompiled,
        WasmBackEnd,
        WasmCompiled,
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
/// Where the learner's choice of compiler is kept between visits.
const COMPILER_KEY: &str = "bcc-wasm-compiler";
/// bcc's WebAssembly compilations kept for their squiggles and for loading.
const CHECKED_LIMIT: usize = 4;

/// Which compiler made a module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compiler {
    /// bcc, compiling in the browser.
    #[default]
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

    fn key(self) -> &'static str {
        match self {
            | Self::Bcc => "bcc",
            | Self::Clang => "clang",
        }
    }

    fn other(self) -> Self {
        match self {
            | Self::Bcc => Self::Clang,
            | Self::Clang => Self::Bcc,
        }
    }
}

/// Why bcc has no module for a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BccFailure {
    /// bcc refused the program; its diagnostics say why.
    Refused(String),
    /// The program is beyond what bcc can compile inside the browser (it
    /// ran out of a fixed memory region, nested too deeply, or hit an
    /// internal error), which compiling again would only repeat.
    Limit(String),
    /// The in-browser compiler did not finish (it timed out or its Worker
    /// stopped), which may not happen again, so compiling again may work.
    Stopped(String),
}

/// bcc's recent WebAssembly compilations of the editor's text, whose
/// diagnostics are the editor's squiggles in this view and whose modules a
/// load reuses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Checks {
    /// bcc's answer for each source, or why the compiler Worker gave none,
    /// oldest first.
    compiled: Vec<(String, Result<WasmCompiled, BccFailure>)>,
    /// The source being compiled for its squiggles.
    checking: Option<String>,
}

impl Checks {
    /// bcc's diagnostics for `text` compiled for WebAssembly, once known.
    pub fn diagnostics(&self, text: &str) -> &[CompileDiagnostic] {
        self.answer(text)
            .map_or(&[], |answer| answer.diagnostics.as_slice())
    }

    /// bcc's answer for `source`, if it is known.
    fn answer(&self, source: &str) -> Option<&WasmCompiled> {
        self.compiled
            .iter()
            .rev()
            .find(|(known, _)| known == source)
            .and_then(|(_, result)| result.as_ref().ok())
    }

    /// Whether `source` was compiled, whatever the outcome.
    fn known(&self, source: &str) -> bool {
        self.compiled.iter().any(|(known, _)| known == source)
    }

    /// Keeps `result` for `source` as the newest of at most
    /// [`CHECKED_LIMIT`] compilations; each holds its module and its source.
    fn remember(&mut self, source: &str, result: Result<WasmCompiled, BccFailure>) {
        self.compiled.retain(|(known, _)| known != source);
        self.compiled.push((source.to_owned(), result));
        if self.compiled.len() > CHECKED_LIMIT {
            _ = self.compiled.remove(0);
        }
    }
}

/// The modules a program has: bcc's, or why bcc has none, clang's for a
/// bundled lesson, and those the stepper could not load.
#[derive(Clone, Debug)]
pub struct Modules {
    pub bcc:        Result<WasmModule, BccFailure>,
    pub clang:      Option<WasmModule>,
    /// The compilers whose module the stepper refused, and its reason.
    pub unloadable: Vec<(Compiler, String)>,
}

/// What went wrong with bcc's module, for the view: a headline, the
/// details (bcc's diagnostics, laid out as bcc prints them, or a sentence),
/// and whether compiling again may help.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BccProblem {
    pub headline: &'static str,
    pub detail:   String,
    /// The detail is prose to wrap, not diagnostics.
    pub prose:    bool,
    pub retry:    bool,
}

impl Modules {
    /// The module to step when the learner prefers `preferred`: that one if
    /// the stepper can load it, else the other compiler's.
    pub fn pick(&self, preferred: Compiler) -> Option<(Compiler, &WasmModule)> {
        [preferred, preferred.other()]
            .into_iter()
            .find_map(|compiler| Some((compiler, self.loadable(compiler)?)))
    }

    /// `compiler`'s module, unless there is none or the stepper refused it.
    pub fn loadable(&self, compiler: Compiler) -> Option<&WasmModule> {
        let module = match compiler {
            | Compiler::Bcc => self.bcc.as_ref().ok(),
            | Compiler::Clang => self.clang.as_ref(),
        };
        module.filter(|_| !self.unloadable.iter().any(|(known, _)| *known == compiler))
    }

    /// The compilers with a module to step, bcc first.
    pub fn available(&self) -> Vec<Compiler> {
        [Compiler::Bcc, Compiler::Clang]
            .into_iter()
            .filter(|&compiler| self.loadable(compiler).is_some())
            .collect()
    }

    /// The compilers the switch offers: those with a module, and bcc when
    /// compiling again may give it one.
    pub fn offered(&self) -> Vec<Compiler> {
        let mut offered = self.available();
        if matches!(self.bcc, Err(BccFailure::Stopped(_))) {
            offered.insert(0, Compiler::Bcc);
        }
        offered
    }

    /// Why bcc's module cannot be stepped, if it cannot.
    pub fn bcc_problem(&self) -> Option<BccProblem> {
        match &self.bcc {
            | Err(BccFailure::Refused(diagnostics)) => Some(BccProblem {
                headline: "bcc can't compile this program to WebAssembly. bcc said:",
                detail:   diagnostics.clone(),
                prose:    false,
                retry:    false,
            }),
            | Err(BccFailure::Limit(reason)) => Some(BccProblem {
                headline: "bcc can't compile this program inside the browser:",
                detail:   reason.clone(),
                prose:    true,
                retry:    false,
            }),
            | Err(BccFailure::Stopped(error)) => Some(BccProblem {
                headline: "bcc's in-browser compiler stopped before it finished. That is a limit \
                           of compiling inside this browser tab, not a mistake in the program:",
                detail:   error.clone(),
                prose:    true,
                retry:    true,
            }),
            | Ok(_) => self
                .unloadable
                .iter()
                .find(|(compiler, _)| *compiler == Compiler::Bcc)
                .map(|(_, error)| BccProblem {
                    headline: "bcc compiled this program, but the stepper can't run its module:",
                    detail:   error.clone(),
                    prose:    true,
                    retry:    false,
                }),
        }
    }

    /// Why there is nothing to step, when [`Modules::pick`] finds nothing.
    pub fn nothing_to_step(&self) -> String {
        let clang = match (
            &self.clang,
            self.unloadable.iter().find(|(c, _)| *c == Compiler::Clang),
        ) {
            | (Some(_), Some((_, error))) =>
                format!(" Clang's module for this lesson didn't load either: {error}"),
            | (Some(_), None) => String::new(),
            | (None, _) => " It is not one of the lessons, so there is no module made ahead of \
                            time either."
                .to_owned(),
        };
        let advice = match &self.bcc {
            | Err(BccFailure::Refused(_)) =>
                " Fix the error below, or step through the program on the abstract machine.",
            | Err(BccFailure::Limit(_)) =>
                " Make the program smaller or simpler, or compile it with the native bcc.",
            | Err(BccFailure::Stopped(_)) =>
                " Compile again, or make the program smaller or simpler.",
            | Ok(_) => " Change the program, or step through it on the abstract machine.",
        };
        let headline = match &self.bcc {
            | Err(BccFailure::Refused(_)) => "bcc could not compile this program to WebAssembly.",
            | Err(BccFailure::Limit(_)) => "bcc could not compile this program inside the browser.",
            | Err(BccFailure::Stopped(_)) => "bcc's in-browser compiler did not finish.",
            | Ok(_) => "bcc compiled this program, but the stepper can't run its module.",
        };
        format!("{headline}{clang}{advice}")
    }
}

/// Which compiler's module the view steps, and the modules it chooses
/// from.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    /// The compiler the learner wants to see; bcc until they choose.
    pub preferred: Compiler,
    /// Who compiled the module the view steps.
    pub compiler:  Option<Compiler>,
    /// The loaded source and its modules, so switching compiler needs no
    /// recompilation.
    modules:       Option<(String, Modules)>,
}

impl Choice {
    /// The compilers the switch offers for the loaded source.
    pub fn offered(&self) -> Vec<Compiler> {
        self.modules
            .as_ref()
            .map(|(_, modules)| modules.offered())
            .unwrap_or_default()
    }

    /// Why bcc's module for the loaded source cannot be stepped.
    pub fn bcc_problem(&self) -> Option<BccProblem> {
        self.modules
            .as_ref()
            .and_then(|(_, modules)| modules.bcc_problem())
    }

    /// Whether the view shows what it should for `source`, given the source
    /// the engine Worker has a module for (`loaded`; `None` after the engine
    /// restarted): the module of the compiler the learner prefers, or the
    /// only one there is, or nothing when there is nothing to step. A module
    /// the stepper refused counts as missing, so a failed load is not tried
    /// again.
    fn settled(&self, source: &str, loaded: Option<&str>) -> bool {
        loaded == Some(source)
            && self.modules_of(source).is_some_and(|modules| {
                let available = modules.available();
                self.compiler == Some(self.preferred)
                    || (!available.contains(&self.preferred)
                        && self.compiler == available.first().copied())
            })
    }

    /// `source`'s modules for a new load, unless the browser stopped bcc's
    /// compilation, which is then tried again.
    fn reusable(&self, source: &str) -> Option<&Modules> {
        self.modules_of(source)
            .filter(|modules| !matches!(modules.bcc, Err(BccFailure::Stopped(_))))
    }

    /// `source`'s modules, if they are known.
    fn modules_of(&self, source: &str) -> Option<&Modules> {
        self.modules
            .as_ref()
            .filter(|(known, _)| known == source)
            .map(|(_, modules)| modules)
    }
}

#[derive(Clone, Copy)]
pub struct WasmState {
    pub program:      RwSignal<Option<WasmProgram>>,
    /// The source text the program was loaded for.
    loaded:           RwSignal<Option<String>>,
    pub choice:       RwSignal<Choice>,
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
            choice:       RwSignal::new(Choice::default()),
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

    /// Whether the view shows what it should for `source` (see
    /// [`Choice::settled`]).
    fn settled(self, source: &str) -> bool {
        let loaded = self.loaded.get_untracked();
        self.choice
            .with_untracked(|choice| choice.settled(source, loaded.as_deref()))
    }

    /// Load the module for the program the page runs, from the compiler
    /// the learner prefers, unless the view already shows what it should.
    /// A module the stepper refuses is marked unloadable and the other
    /// compiler's is tried, so a refusal is shown once rather than retried.
    pub(super) fn ensure_loaded(self, state: State) {
        let source = state.source.get_untracked();
        if self.settled(&source)
            || self.loading.get_untracked()
            || !state.has_engine.get_untracked()
        {
            return;
        }
        let preferred = self.choice.with_untracked(|choice| choice.preferred);
        // A run of the previous program stops, and answers still on their
        // way for it are dropped.
        self.generation.update(|generation| *generation += 1);
        self.loading.set(true);
        // While it loads, the view says so rather than why the previous
        // program had no module.
        self.unavailable.set(None);
        spawn_local(async move {
            let mut modules = self.modules_for(state, &source).await;
            let mut loaded = None;
            while let Some((compiler, module)) = modules.pick(preferred) {
                let module = module.clone();
                let result = match state.engine() {
                    | Ok(engine) => engine.wasm_load(module).await,
                    | Err(error) => Err(error),
                };
                match result {
                    | Ok(program) => {
                        loaded = Some((compiler, program));
                        break;
                    },
                    // The engine itself stopped: the page starts a new one,
                    // which loads this program again.
                    | Err(error) if !state.engine_connected() => {
                        self.loading.set(false);
                        self.fail(state, &error);
                        return;
                    },
                    | Err(error) => modules.unloadable.push((compiler, error)),
                }
            }
            self.loaded.set(Some(source.clone()));
            self.trail.set(Vec::new());
            self.breakpoints.set(Vec::new());
            let compiler = loaded.as_ref().map(|(compiler, _)| *compiler);
            match loaded {
                | Some((_, program)) => {
                    let snapshot = program.snapshot.clone();
                    self.unavailable.set(None);
                    self.program.set(Some(program));
                    self.show(state, snapshot);
                },
                | None => {
                    self.program.set(None);
                    self.current.set(None);
                    self.unavailable.set(Some(modules.nothing_to_step()));
                },
            }
            self.choice.update(|choice| {
                choice.compiler = compiler;
                choice.modules = Some((source, modules));
            });
            self.loading.set(false);
            // The program, or the compiler chosen, may have changed while
            // this one loaded, and the editor's text may need its squiggles.
            self.ensure_loaded(state);
            state.pump();
        });
    }

    /// Compile the editor's text with bcc for WebAssembly once typing
    /// pauses, unless it is known, so the editor shows the diagnostics of
    /// the compilation this view runs: an x86-64 compilation has no C
    /// library headers, while bcc's WASI target has its own. Text the view
    /// is loading is left to the load, which compiles it anyway.
    pub(super) fn check(self, state: State) {
        let text = state.editor_text.get_untracked();
        let (known, checking) = state
            .wasm_checks
            .with_untracked(|checks| (checks.known(&text), checks.checking.is_some()));
        let loading =
            self.loading.get_untracked() && state.source.with_untracked(|source| *source == text);
        if known || checking || loading || crate::pipeline::too_large(&text).is_some() {
            return;
        }
        state
            .wasm_checks
            .update(|checks| checks.checking = Some(text.clone()));
        spawn_local(async move {
            let result = compile_wasm(state, &text).await;
            state.wasm_checks.update(|checks| {
                checks.checking = None;
                checks.remember(&text, result);
            });
            state.pump();
        });
    }

    /// `source`'s modules: bcc compiles it in the compiler Worker, and a
    /// lesson also has clang's. Known modules are reused, unless the
    /// browser stopped bcc's compilation, which is tried again.
    async fn modules_for(self, state: State, source: &str) -> Modules {
        let known = self
            .choice
            .with_untracked(|choice| choice.reusable(source).cloned());
        if let Some(modules) = known {
            return modules;
        }
        // The squiggles' compilation of the same text gives the module too.
        let checked = state
            .wasm_checks
            .with_untracked(|checks| checks.answer(source).cloned());
        let bcc = match checked {
            | Some(answer) => answer.module.map_err(BccFailure::Refused),
            | None => match compile_wasm(state, source).await {
                | Ok(answer) => {
                    let module = answer.module.clone().map_err(BccFailure::Refused);
                    state
                        .wasm_checks
                        .update(|checks| checks.remember(source, Ok(answer)));
                    module
                },
                | Err(failure) => Err(failure),
            },
        };
        Modules {
            bcc,
            clang: Precompiled.compile(source).ok(),
            unloadable: Vec::new(),
        }
    }

    /// Step `compiler`'s module from now on, if the program has one, and
    /// remember the choice for later visits. Choosing bcc after the
    /// browser stopped its compilation compiles again.
    pub fn choose(self, state: State, compiler: Compiler) {
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            _ = storage.set_item(COMPILER_KEY, compiler.key());
        }
        let retry = compiler == Compiler::Bcc
            && self
                .choice
                .with_untracked(|choice| choice.bcc_problem().is_some_and(|problem| problem.retry));
        let (shown, offered) = self.choice.with_untracked(|choice| {
            (
                choice.preferred == compiler && choice.compiler == Some(compiler),
                choice.offered().contains(&compiler),
            )
        });
        if shown && !retry {
            return;
        }
        self.choice.update(|choice| {
            choice.preferred = compiler;
            if retry {
                // Forget the modules, so the next load compiles again.
                choice.compiler = None;
                choice.modules = None;
            }
        });
        if retry {
            state
                .announcement
                .set("Compiling the program with bcc again.".to_owned());
        } else if offered && !self.settled(&state.source.get_untracked()) {
            state.announcement.set(format!(
                "Loading the module {} compiled. Stepping starts again from the first instruction.",
                compiler.name()
            ));
        }
        self.ensure_loaded(state);
    }

    /// The compiler chosen on an earlier visit, if any. Only the browser
    /// keeps one, so the page calls this once it runs there.
    pub(super) fn restore_choice(self) {
        let saved = web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .and_then(|storage| storage.get_item(COMPILER_KEY).ok().flatten());
        let compiler = match saved.as_deref() {
            | Some("clang") => Compiler::Clang,
            | _ => Compiler::Bcc,
        };
        self.choice.update(|choice| choice.preferred = compiler);
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
        }
    }

    /// The engine Worker is gone, and with it the loaded module: the next
    /// engine loads the program again (with the modules already compiled).
    pub(super) fn engine_stopped(self) {
        self.loaded.set(None);
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

    /// Who compiled the module the view steps.
    pub fn compiler(self) -> Option<Compiler> {
        self.choice.with(|choice| choice.compiler)
    }

    /// Why bcc's module for the loaded source cannot be stepped.
    pub fn bcc_problem(self) -> Option<BccProblem> {
        self.choice.with(Choice::bcc_problem)
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

/// `source` compiled by bcc for WebAssembly in the compiler Worker, which
/// is started if it is not running, or why the Worker gave no answer.
async fn compile_wasm(state: State, source: &str) -> Result<WasmCompiled, BccFailure> {
    let compiler = state.compiler().await.map_err(BccFailure::Stopped)?;
    let result = compiler.compile_wasm(source).await;
    // A compiler that timed out or trapped is gone; the next compilation
    // starts a fresh one.
    if !compiler.connected() {
        state.compiler.set_value(None);
    }
    result.map_err(failure)
}

/// What a compiler Worker error means for bcc's module: running out of
/// memory, nesting too deeply or an internal error happen again on every
/// try, while a timeout or a stopped Worker may not.
fn failure(error: String) -> BccFailure {
    if error == engine::OUT_OF_MEMORY
        || error == engine::TOO_DEEP
        || error.starts_with(engine::INTERNAL_ERROR)
    {
        BccFailure::Limit(error)
    } else {
        BccFailure::Stopped(super::compiler_failure(error))
    }
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

    fn modules(bcc: Result<WasmModule, BccFailure>, clang: bool) -> Modules {
        Modules {
            bcc,
            clang: clang.then(|| module("clang")),
            unloadable: Vec::new(),
        }
    }

    /// The engine Worker has the module for "source".
    const LOADED: Option<&str> = Some("source");

    fn choice(preferred: Compiler, compiler: Option<Compiler>, modules: Modules) -> Choice {
        Choice {
            preferred,
            compiler,
            modules: Some(("source".into(), modules)),
        }
    }

    #[test]
    fn checks_keep_the_newest_compilations_and_their_diagnostics() {
        let compiled = |message: &str| WasmCompiled {
            module:      Ok(module("bcc")),
            diagnostics: vec![CompileDiagnostic {
                severity: "warning".into(),
                message: message.into(),
                ..CompileDiagnostic::default()
            }],
        };
        let mut checks = Checks::default();
        checks.remember("a", Ok(compiled("first")));
        checks.remember("b", Err(BccFailure::Stopped("timed out".into())));
        assert_eq!(checks.diagnostics("a")[0].message, "first");
        // A failed compilation is known, so it is not retried while the
        // text stays, but it has no answer to reuse.
        assert!(checks.known("b") && checks.answer("b").is_none());
        assert!(checks.diagnostics("b").is_empty() && checks.diagnostics("c").is_empty());
        checks.remember("a", Ok(compiled("again")));
        assert_eq!(checks.diagnostics("a")[0].message, "again");
        for source in ["c", "d", "e", "f"] {
            checks.remember(source, Ok(compiled(source)));
        }
        assert!(!checks.known("a") && !checks.known("b"));
        assert_eq!(checks.compiled.len(), CHECKED_LIMIT);
    }

    #[test]
    fn a_lesson_offers_both_compilers_and_steps_the_one_chosen() {
        let both = modules(Ok(module("bcc")), true);
        assert_eq!(both.available(), [Compiler::Bcc, Compiler::Clang]);
        assert_eq!(both.offered(), [Compiler::Bcc, Compiler::Clang]);
        let (compiler, picked) = both.pick(Compiler::Bcc).unwrap();
        assert_eq!((compiler, picked.producer.as_str()), (Compiler::Bcc, "bcc"));
        let (compiler, picked) = both.pick(Compiler::Clang).unwrap();
        assert_eq!(
            (compiler, picked.producer.as_str()),
            (Compiler::Clang, "clang")
        );
        assert_eq!(both.bcc_problem(), None);
        // Showing bcc's module is settled; preferring clang then is not.
        let shown = choice(Compiler::Bcc, Some(Compiler::Bcc), both);
        assert!(shown.settled("source", LOADED));
        assert!(!shown.settled("other source", Some("other source")));
        let switched = Choice {
            preferred: Compiler::Clang,
            ..shown
        };
        assert!(!switched.settled("source", LOADED));
    }

    /// The review's second regression: an engine trap or timeout starts a
    /// new engine Worker, which has no module, so the same program and
    /// choice must load again (Retry engine) instead of counting as shown.
    #[test]
    fn after_an_engine_restart_nothing_is_settled() {
        let shown = choice(
            Compiler::Bcc,
            Some(Compiler::Bcc),
            modules(Ok(module("bcc")), true),
        );
        assert!(shown.settled("source", LOADED));
        assert!(!shown.settled("source", None));
        // The modules are kept, so the new engine gets them without
        // compiling again.
        assert!(shown.reusable("source").is_some());
        let refused = choice(
            Compiler::Bcc,
            None,
            modules(Err(BccFailure::Refused("error".into())), false),
        );
        assert!(!refused.settled("source", None));
    }

    #[test]
    fn edited_code_has_only_bccs_module_whatever_was_chosen() {
        let edited = modules(Ok(module("bcc")), false);
        assert_eq!(edited.available(), [Compiler::Bcc]);
        assert_eq!(edited.pick(Compiler::Clang).unwrap().0, Compiler::Bcc);
        assert!(choice(Compiler::Clang, Some(Compiler::Bcc), edited).settled("source", LOADED));
    }

    #[test]
    fn a_refused_lesson_falls_back_to_clang_and_keeps_bccs_diagnostic() {
        let refused = modules(
            Err(BccFailure::Refused(
                "error: setjmp is not supported on WebAssembly".into(),
            )),
            true,
        );
        assert_eq!(refused.available(), [Compiler::Clang]);
        assert_eq!(refused.offered(), [Compiler::Clang]);
        assert_eq!(refused.pick(Compiler::Bcc).unwrap().0, Compiler::Clang);
        let problem = refused.bcc_problem().unwrap();
        assert!(problem.detail.contains("setjmp") && !problem.retry && !problem.prose);
        let shown = choice(Compiler::Bcc, Some(Compiler::Clang), refused.clone());
        assert!(shown.settled("source", LOADED));
        // A refusal is kept: the same source is not compiled again.
        assert!(shown.reusable("source").is_some());
        let edited = Modules {
            clang: None,
            ..refused
        };
        assert!(edited.pick(Compiler::Bcc).is_none());
        assert_eq!(edited.available(), []);
        assert!(choice(Compiler::Bcc, None, edited.clone()).settled("source", LOADED));
        let why = edited.nothing_to_step();
        assert!(why.contains("Fix the error below"), "{why}");
        assert!(why.contains("not one of the lessons"), "{why}");
    }

    /// The review's reload loop: bcc compiles `static char big[20 << 20]`,
    /// but the stepper refuses its memory. That module is marked
    /// unloadable, so the view settles (no second load) on clang's module
    /// for a lesson, or on nothing, and says why.
    #[test]
    fn a_module_the_stepper_refuses_is_loaded_once_and_falls_back() {
        let mut lesson = modules(Ok(module("bcc")), true);
        lesson.unloadable.push((
            Compiler::Bcc,
            "the program needs 338 pages of memory".into(),
        ));
        assert_eq!(lesson.available(), [Compiler::Clang]);
        assert_eq!(lesson.pick(Compiler::Bcc).unwrap().0, Compiler::Clang);
        let problem = lesson.bcc_problem().unwrap();
        assert!(problem.headline.contains("the stepper can't run"));
        assert!(problem.detail.contains("338 pages") && problem.prose);
        assert!(
            choice(Compiler::Bcc, Some(Compiler::Clang), lesson.clone()).settled("source", LOADED)
        );
        let edited = Modules {
            clang: None,
            ..lesson
        };
        assert!(edited.pick(Compiler::Bcc).is_none());
        assert!(choice(Compiler::Bcc, None, edited.clone()).settled("source", LOADED));
        let why = edited.nothing_to_step();
        assert!(why.contains("the stepper can't run its module"), "{why}");
        assert!(!why.contains("Fix the error"), "{why}");
    }

    /// Running out of the compiler's memory (a macro that expands
    /// enormously), nesting too deeply or an internal error repeat on every
    /// try: a limit of compiling in the browser, not retried, and worded so.
    /// A timeout or a stopped Worker may not repeat.
    #[test]
    fn deterministic_compiler_failures_are_limits_and_others_retry() {
        for error in [
            engine::OUT_OF_MEMORY.to_owned(),
            engine::TOO_DEEP.to_owned(),
            format!("{}index out of bounds", engine::INTERNAL_ERROR),
        ] {
            let limit = failure(error.clone());
            assert_eq!(limit, BccFailure::Limit(error.clone()));
            let edited = modules(Err(limit), false);
            let problem = edited.bcc_problem().unwrap();
            assert!(!problem.retry && problem.prose, "{error}");
            assert!(problem.headline.contains("inside the browser"));
            assert_eq!(edited.offered(), []);
            let why = edited.nothing_to_step();
            assert!(
                why.contains("smaller or simpler") && !why.contains("Compile again"),
                "{why}"
            );
            let shown = choice(Compiler::Bcc, None, edited);
            assert!(
                shown.reusable("source").is_some(),
                "kept, not compiled again"
            );
        }
        for error in [
            engine::COMPILER.timed_out().to_owned(),
            engine::COMPILER.stopped().to_owned(),
            "RuntimeError: unreachable".to_owned(),
        ] {
            assert!(matches!(failure(error), BccFailure::Stopped(_)));
        }
        assert!(matches!(
            failure("trap".into()),
            BccFailure::Stopped(reason) if reason.contains("crashed")
        ));
    }

    /// A compilation the browser stopped (a timeout, a stopped Worker) is not
    /// the program's fault: the view says so, settles without retrying on
    /// its own, keeps bcc in a lesson's switch, and compiles again on the
    /// next load.
    #[test]
    fn a_compilation_the_browser_stopped_is_tried_again_on_request() {
        let stopped = modules(
            Err(BccFailure::Stopped(
                "bcc's in-browser compiler took longer than 10 seconds".into(),
            )),
            true,
        );
        assert_eq!(stopped.available(), [Compiler::Clang]);
        assert_eq!(stopped.offered(), [Compiler::Bcc, Compiler::Clang]);
        assert!(stopped.bcc_problem().unwrap().retry);
        let shown = choice(Compiler::Bcc, Some(Compiler::Clang), stopped.clone());
        assert!(shown.settled("source", LOADED));
        assert!(shown.reusable("source").is_none(), "compiled again");
        // Compiling again forgets the modules, so nothing is settled until
        // bcc has compiled the program anew, a lesson or not.
        let retrying = Choice {
            compiler: None,
            modules: None,
            ..shown
        };
        assert!(!retrying.settled("source", LOADED));
        let edited = Modules {
            clang: None,
            ..stopped
        };
        let why = edited.nothing_to_step();
        assert!(
            why.contains("did not finish") && why.contains("Compile again"),
            "{why}"
        );
        assert!(!why.contains("Fix the error"), "{why}");
    }
}
