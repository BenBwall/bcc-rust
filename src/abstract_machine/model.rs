//! Public observations and compact internal operations for the subset engine.

/// A compiled bounded-subset program, independent of native compiler storage.
#[derive(Debug, Clone)]
pub struct Program {
    pub(super) source:    String,
    pub(super) functions: Vec<Function>,
    pub(super) entry:     usize,
}

/// An immutable observation at an execution boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// Completed operation count (initial state is zero).
    pub step:            usize,
    /// `ready`, `running`, `finished`, `undefined_behavior`, `error` or
    /// `limit`.
    pub status:          &'static str,
    /// The next operation, or the failing/last operation at termination.
    pub source_range:    SourceRange,
    /// The last completed operation's explanation.
    pub event:           String,
    /// Active calls, outermost first.
    pub frames:          Vec<Frame>,
    /// All allocated objects, retaining ended lifetimes for explanation.
    pub objects:         Vec<Object>,
    /// Next writes, if known; writes commit atomically at store operations.
    pub pending_effects: Vec<String>,
    /// Terminal diagnostic, when present.
    pub diagnostics:     Vec<Diagnostic>,
    /// The value returned by `main` after successful termination.
    pub return_value:    Option<Value>,
}

impl State {
    pub(super) fn terminal(&self) -> bool {
        !matches!(self.status, "ready" | "running")
    }
}

/// A source range in the submitted UTF-8 source (ASCII syntax only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceRange {
    /// Inclusive byte offset.
    pub start:      usize,
    /// Exclusive byte offset.
    pub end:        usize,
    /// One-based starting line.
    pub line:       usize,
    /// One-based starting byte column.
    pub column:     usize,
    /// One-based ending line.
    pub end_line:   usize,
    /// One-based ending byte column.
    pub end_column: usize,
}

/// An active C function invocation; IDs are never reused in a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Activation identity.
    pub id:       usize,
    /// Source function name.
    pub function: String,
    /// Zero-based call depth.
    pub depth:    usize,
}

/// One automatic C object. Array elements share their enclosing object ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// Stable allocation identity, independent of host addresses.
    pub id:               usize,
    /// Source identifier.
    pub name:             String,
    /// Declared subset type spelling.
    pub r#type:           String,
    /// Owning activation.
    pub frame_id:         usize,
    /// Always `automatic` in this subset.
    pub storage_duration: &'static str,
    /// `alive` or `ended`.
    pub lifetime:         &'static str,
    /// Stored scalar or aggregate contents.
    pub value:            Value,
}

/// A C value or the observable state of not-yet-initialized storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// No value has been stored yet (not implicitly zero).
    Indeterminate,
    /// Signed 32-bit `int`.
    Integer(i32),
    /// An `int *`, with offset measured in int elements; `None` means null.
    Pointer {
        /// Target object ID.
        target: Option<usize>,
        /// Element offset (one-past is representable).
        offset: i32,
    },
    /// Fixed-size int array contents.
    Array(Vec<Value>),
}

/// A structured diagnostic. Unsupported features and limits never claim UB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// `syntax`, `unsupported`, `undefined_behavior`, `indeterminate` or
    /// `limit`.
    pub kind:         &'static str,
    /// Human-readable explanation.
    pub message:      String,
    /// The rejected or failing operation.
    pub source_range: SourceRange,
    /// Verified C99 rule, when applicable.
    pub citation:     Option<&'static str>,
}

impl Diagnostic {
    pub(super) fn new(kind: &'static str, range: SourceRange, message: &str) -> Self {
        Self {
            kind,
            message: String::from(message),
            source_range: range,
            citation: None,
        }
    }

    pub(super) fn limit(range: SourceRange, message: &str) -> Self {
        Self::new("limit", range, message)
    }

    pub(super) fn ub(range: SourceRange, message: &str, citation: &'static str) -> Self {
        Self {
            citation: Some(citation),
            ..Self::new("undefined_behavior", range, message)
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Function {
    pub name:       String,
    pub parameters: Vec<(String, Type)>,
    pub code:       Vec<Instruction>,
}

#[derive(Debug, Clone)]
pub(super) struct Instruction {
    pub op:    Op,
    pub range: SourceRange,
}

#[derive(Debug, Clone)]
pub(super) enum Op {
    Enter,
    Leave,
    Declare(String, Type),
    Push(i32),
    Address(String),
    Load,
    Decay(String),
    AddressOf,
    Dereference,
    Index,
    Unary(&'static str),
    Binary(&'static str),
    Store,
    Increment(i32),
    Pop,
    Jump(usize),
    Branch(usize),
    Call(String, usize),
    Return,
    Fallthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Type {
    Int,
    Pointer,
    Array(usize),
}

pub(super) const MAX_SOURCE: usize = 32 * 1024;
pub(super) const MAX_DEPTH: usize = 64;
pub(super) const MAX_CODE: usize = 8192;
pub(super) const MAX_STEPS: usize = 2048;
pub(super) const MAX_FRAMES: usize = 32;
pub(super) const MAX_OBJECTS: usize = 128;
pub(super) const MAX_CELLS: usize = 512;
pub(super) const MAX_OPERANDS: usize = 1024;
pub(super) const MAX_TRACE_BYTES: usize = 8 * 1024 * 1024;
