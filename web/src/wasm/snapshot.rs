//! What the engine Worker tells the page about a WebAssembly program, as
//! plain data. Values arrive rendered as text; the page decides only how to
//! lay them out.

use serde::{
    Deserialize,
    Serialize,
};

use crate::pipeline::CompileDiagnostic;

/// A module to step, with where its code came from.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasmModule {
    pub bytes:    Vec<u8>,
    /// Which C line each instruction came from. `None` lets the stepper read
    /// the module's own DWARF line table, if it has one.
    pub lines:    Option<Vec<WasmLine>>,
    /// Who compiled it, for the view's header.
    pub producer: String,
    /// The command that built it.
    pub command:  String,
    /// The compiler's own text format of the module, shown in place of the
    /// stepper's listing. `None` lists the module as the stepper prints it.
    pub text:     Option<WasmText>,
}

/// bcc's answer for one source compiled to WebAssembly: its module, or its
/// errors as it prints them when it refuses the program, and every
/// diagnostic placed in the editor, which draws them as squiggles.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct WasmCompiled {
    pub module:      Result<WasmModule, String>,
    pub diagnostics: Vec<CompileDiagnostic>,
}

/// A compiler's `.wat` for a module, and where each instruction is in it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct WasmText {
    pub text:         String,
    /// Each instruction's module offset and its 1-based line in `text`.
    pub instructions: Vec<WasmTextLine>,
    /// Each defined function's name and code range in the module.
    pub functions:    Vec<WasmFunction>,
}

/// The instruction at module byte `offset` is on line `line` of the text.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct WasmTextLine {
    pub offset: u32,
    pub line:   u32,
}

/// A defined function: its index (imports first), its name, and the module
/// offsets of its first instruction and just past its final `end`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct WasmFunction {
    pub index: u32,
    pub name:  String,
    pub start: u32,
    pub end:   u32,
}

/// From module byte `offset` on, code comes from C `line` (0: no line).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct WasmLine {
    pub offset: u32,
    pub line:   u32,
    pub column: u32,
}

/// How far a stepping request should run.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum WasmRun {
    Step,
    StepOver,
    StepOut,
    NextLine,
    RunToLine(u32),
    Continue,
}

/// A request to the Worker's WebAssembly session.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum WasmControl {
    Run(WasmRun),
    /// Show the state after this many instructions.
    GoTo(u64),
    Reset,
    /// Stop before the instructions at these module offsets.
    Breakpoints(Vec<u32>),
    /// Start again and run the C library's startup code, up to the first
    /// instruction of `main`.
    StartAtMain,
}

/// A loaded program: its listing, and the state before its first
/// instruction.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasmProgram {
    pub wat:        String,
    /// For each line of `wat`: the instruction's module offset, if the line
    /// shows one, and its C line.
    pub lines:      Vec<WatLine>,
    pub producer:   String,
    pub command:    String,
    /// Whether instructions map to C lines.
    pub has_lines:  bool,
    pub size:       usize,
    /// Imports the stepper does not provide.
    pub unresolved: Vec<String>,
    pub snapshot:   WasmSnapshot,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatLine {
    pub offset: Option<u32>,
    pub source: Option<u32>,
}

impl WasmProgram {
    /// The listing lines whose instructions came from C `line`.
    pub fn wat_lines_for(&self, line: usize) -> Vec<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, wat)| wat.source == u32::try_from(line).ok())
            .map(|(index, _)| index + 1)
            .collect()
    }

    /// The C line listing line `line` (1-based) came from.
    pub fn source_for(&self, line: usize) -> Option<usize> {
        line.checked_sub(1)
            .and_then(|index| self.lines.get(index))
            .and_then(|wat| wat.source)
            .map(|line| line as usize)
    }

    /// The listing line of the instruction at module `offset`.
    pub fn wat_line_of(&self, offset: u32) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| line.offset == Some(offset))
            .map(|index| index + 1)
    }
}

/// The state after some number of instructions.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasmSnapshot {
    pub step:      u64,
    /// `running`, `finished`, `exited`, `trapped` or `limit`.
    pub status:    String,
    /// Why the last request stopped early: `breakpoint` or `budget`.
    pub stopped:   Option<String>,
    /// The instruction that runs next.
    pub next:      Option<Place>,
    /// The instruction the last step executed.
    pub last:      Option<Place>,
    pub narration: WasmNarration,
    /// The current call's operands, deepest first.
    pub stack:     Vec<Operand>,
    /// Operands that belong to the callers, below the current call's.
    pub below:     usize,
    pub locals:    Vec<Slot>,
    /// Active calls, innermost first.
    pub frames:    Vec<CallFrame>,
    /// The current call's entered blocks, innermost (label 0) first.
    pub labels:    Vec<BlockLabel>,
    pub globals:   Vec<Slot>,
    pub memory:    MemoryView,
    pub output:    Vec<Output>,
    /// The entry point's results, once it returned.
    pub results:   Vec<Operand>,
    /// The status the program passed to WASI's `proc_exit`, once it exited.
    pub exit:      Option<i32>,
}

impl WasmSnapshot {
    pub fn can_step(&self) -> bool {
        self.status == "running"
    }
}

/// An instruction's place in the listing and the C source.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub offset:   u32,
    pub wat_line: usize,
    pub source:   Option<usize>,
}

/// What the last step did, for programmers new to WebAssembly.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasmNarration {
    /// `evaluate`, `convert` (locals and globals), `memory`, `lifetime` (the
    /// shadow stack), `control`, `call` or `fault`; it selects the colour.
    pub category:    String,
    /// The badge: `Compute`, `Local`, `Memory`, ….
    pub label:       String,
    /// The instruction as the listing shows it.
    pub instruction: String,
    pub title:       String,
    pub explanation: String,
    pub concepts:    Vec<Concept>,
}

/// An idea a step relies on, with the part of the specification that
/// defines it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Concept {
    pub title:    String,
    pub gloss:    String,
    /// Like `Spec §4.4.8`.
    pub section:  String,
    pub citation: String,
}

/// A typed value.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operand {
    #[serde(rename = "type")]
    pub ty:    String,
    pub text:  String,
    /// Pushed by the last step.
    pub fresh: bool,
}

/// A local or a global.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub index:   u32,
    pub name:    Option<String>,
    #[serde(rename = "type")]
    pub ty:      String,
    pub text:    String,
    /// `param`, `local`, `mut` (a mutable global) or `const`.
    pub role:    String,
    /// `read`, `write`, or empty.
    pub touched: String,
    /// For a global: what toolchains use it for.
    pub note:    Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallFrame {
    pub name:          String,
    /// Where the call is now: the next instruction, or the call it waits on.
    pub wat_line:      usize,
    pub source:        Option<usize>,
    /// The shadow stack pointer when the call began.
    pub stack_pointer: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockLabel {
    /// `block`, `loop`, `if`, `else` or `function`.
    pub kind:  String,
    /// Its `@N` name in the listing (nesting depth), 0 for the function.
    pub name:  usize,
    pub start: usize,
    pub end:   usize,
    /// How many values a branch to it carries.
    pub arity: usize,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryView {
    pub pages:         u32,
    pub stack_pointer: Option<u32>,
    /// The shadow stack's top (its pointer's initial value).
    pub stack_top:     Option<u32>,
    pub windows:       Vec<MemoryWindow>,
    /// The bytes the last step read or wrote.
    pub access:        Option<Access>,
    /// Each call's part of the shadow stack, innermost first.
    pub bands:         Vec<StackBand>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryWindow {
    /// `stack`, `data` or `access`.
    pub key:   String,
    pub title: String,
    pub start: u32,
    pub bytes: Vec<u8>,
    pub note:  String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Access {
    pub address: u64,
    pub size:    u64,
    pub write:   bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct StackBand {
    pub name:  String,
    pub start: u32,
    pub end:   u32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Output {
    /// `stdout` or `stderr`.
    pub stream: String,
    pub text:   String,
}
