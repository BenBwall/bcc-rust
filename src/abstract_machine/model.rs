//! Public observations and compact internal operations for the subset engine.

use super::clauses::Clause;

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
    /// What the last completed operation did, and why.
    pub event:           Event,
    /// The full expression the innermost activation is evaluating.
    pub evaluation:      Option<Evaluation>,
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

/// One completed operation as structured data: what kind of rule ran, on
/// which source, with which values, and the clauses of C99 that define it.
/// The title and explanation are plain-language renderings of the same data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The rule applied.
    pub kind:           EventKind,
    /// The exact source the operation evaluated (a subexpression, not a line).
    pub range:          SourceRange,
    /// A short headline, such as "Read `total`".
    pub title:          String,
    /// One plain sentence: what happened and why C says so. The
    /// explanation gives the details.
    pub summary:        String,
    /// A beginner-oriented paragraph naming the values involved.
    pub explanation:    String,
    /// Why the rule matters, or empty. Every event of a topic carries the
    /// same note; the site shows it the first time the learner sees it.
    pub note:           String,
    /// The rules applied, most specific first.
    pub clauses:        Vec<Clause>,
    /// Input values, in evaluation order.
    pub operands:       Vec<Operand>,
    /// The value or designation the operation produced.
    pub result:         Option<Operand>,
    /// Objects created, read, written or ended by this operation.
    pub objects:        Vec<usize>,
    /// Whether a full expression ended here (C99 §6.8p4).
    pub sequence_point: bool,
}

impl Event {
    pub(super) fn new(kind: EventKind, range: SourceRange, title: String) -> Self {
        Self {
            kind,
            range,
            title,
            summary: String::new(),
            explanation: String::new(),
            note: String::new(),
            clauses: Vec::new(),
            operands: Vec::new(),
            result: None,
            objects: Vec::new(),
            sequence_point: false,
        }
    }
}

/// The rule an operation applied. [`EventKind::category`] groups kinds for
/// colour coding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// The initial state: `main` is about to run.
    Start,
    /// Execution entered a block.
    EnterBlock,
    /// A block finished, ending its objects' lifetimes.
    LeaveBlock,
    /// A declaration created an object with an indeterminate value.
    Declare,
    /// An integer constant produced its value.
    Constant,
    /// An identifier evaluated to the lvalue designating its object.
    Designate,
    /// An lvalue was read for its stored value.
    LvalueConversion,
    /// A `char` value was promoted to `int`.
    IntegerPromotion,
    /// An array was converted to a pointer to its first element.
    ArrayDecay,
    /// `&` formed a pointer to an object.
    AddressOf,
    /// `*` designated the object a pointer points to.
    Dereference,
    /// `a[i]` designated an array element.
    Subscript,
    /// A unary `+`, `-` or `!` produced its value.
    Unary,
    /// An integer `+`, `-`, `*`, `/` or `%` produced its value.
    Arithmetic,
    /// A pointer moved by a number of elements.
    PointerArithmetic,
    /// A relational or equality operator produced 0 or 1.
    Comparison,
    /// An initializer stored an object's first value.
    Initialize,
    /// An assignment replaced an object's value.
    Assign,
    /// `++` or `--` updated an object.
    Increment,
    /// An expression statement's value was discarded.
    Discard,
    /// Control moved without a test (loop back, skip an else).
    Jump,
    /// A controlling expression chose the next statement.
    Branch,
    /// A function call created a new activation.
    Call,
    /// A function returned a value to its caller.
    Return,
    /// A diagnostic stopped execution.
    Fault,
}

impl EventKind {
    /// The kind's stable wire name, such as `lvalue_conversion`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            | Self::Start => "start",
            | Self::EnterBlock => "enter_block",
            | Self::LeaveBlock => "leave_block",
            | Self::Declare => "declare",
            | Self::Constant => "constant",
            | Self::Designate => "designate",
            | Self::LvalueConversion => "lvalue_conversion",
            | Self::IntegerPromotion => "integer_promotion",
            | Self::ArrayDecay => "array_decay",
            | Self::AddressOf => "address_of",
            | Self::Dereference => "dereference",
            | Self::Subscript => "subscript",
            | Self::Unary => "unary",
            | Self::Arithmetic => "arithmetic",
            | Self::PointerArithmetic => "pointer_arithmetic",
            | Self::Comparison => "comparison",
            | Self::Initialize => "initialize",
            | Self::Assign => "assign",
            | Self::Increment => "increment",
            | Self::Discard => "discard",
            | Self::Jump => "jump",
            | Self::Branch => "branch",
            | Self::Call => "call",
            | Self::Return => "return",
            | Self::Fault => "fault",
        }
    }

    /// `evaluate`, `convert`, `memory`, `lifetime`, `control`, `call` or
    /// `fault`.
    #[must_use]
    pub const fn category(self) -> &'static str {
        match self {
            | Self::Constant
            | Self::Unary
            | Self::Arithmetic
            | Self::PointerArithmetic
            | Self::Comparison => "evaluate",
            | Self::LvalueConversion | Self::IntegerPromotion | Self::ArrayDecay => "convert",
            | Self::Designate
            | Self::AddressOf
            | Self::Dereference
            | Self::Subscript
            | Self::Initialize
            | Self::Assign
            | Self::Increment => "memory",
            | Self::EnterBlock | Self::LeaveBlock | Self::Declare => "lifetime",
            | Self::Start | Self::Discard | Self::Jump | Self::Branch => "control",
            | Self::Call | Self::Return => "call",
            | Self::Fault => "fault",
        }
    }
}

/// An evaluated operand: a value, or the object (element) an lvalue
/// designates before any read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// An rvalue.
    Value(Value),
    /// An lvalue designating element `offset` of object `target`.
    Address {
        /// The designated object's ID.
        target: usize,
        /// The element within it (0 for a scalar).
        offset: i32,
    },
}

/// Progress through one full expression: the static tree from
/// [`Program::expression`] plus the values its nodes have produced so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    /// Index of the function in [`Program::functions`] order.
    pub function: usize,
    /// The root node of the full expression.
    pub root:     usize,
    /// Node values in completion order; a later entry replaces an earlier one.
    pub values:   Vec<(usize, Operand)>,
    /// The node the last completed operation evaluated.
    pub last:     Option<usize>,
    /// A call in progress from this full expression.
    pub calling:  Option<usize>,
}

/// One node of a statement's expression tree. Children precede their parent,
/// so a tree occupies the node range `first..=root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionNode {
    /// Operator, identifier, constant, or statement form (`=`, `return`).
    pub label:    String,
    /// The source the node covers.
    pub range:    SourceRange,
    /// Operand nodes, in source order.
    pub children: Vec<usize>,
    /// The first node of this node's subtree.
    pub first:    usize,
    /// The root of the full expression containing this node.
    pub root:     usize,
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
    /// The call expression this activation is waiting on, when it has
    /// called another function.
    pub waiting:  Option<SourceRange>,
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
    /// Whether this object is a function parameter.
    pub parameter:        bool,
    /// Stored scalar or aggregate contents.
    pub value:            Value,
}

/// A C value or the observable state of not-yet-initialized storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// No value has been stored yet (not implicitly zero).
    Indeterminate,
    /// Signed 32-bit `int`, or a `char` value in -128..=127.
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
    /// The same rule with its title and a plain-language gloss.
    pub clause:       Option<&'static Clause>,
}

impl Diagnostic {
    pub(super) fn new(kind: &'static str, range: SourceRange, message: &str) -> Self {
        Self {
            kind,
            message: String::from(message),
            source_range: range,
            citation: None,
            clause: None,
        }
    }

    pub(super) fn limit(range: SourceRange, message: &str) -> Self {
        Self::new("limit", range, message)
    }

    pub(super) fn ub(range: SourceRange, message: &str, clause: &'static Clause) -> Self {
        Self {
            citation: Some(clause.citation),
            clause: Some(clause),
            ..Self::new("undefined_behavior", range, message)
        }
    }

    pub(super) fn indeterminate(
        range: SourceRange,
        message: &str,
        clause: &'static Clause,
    ) -> Self {
        Self {
            citation: Some(clause.citation),
            clause: Some(clause),
            ..Self::new("indeterminate", range, message)
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Function {
    pub name:       String,
    pub parameters: Vec<(String, Type)>,
    pub code:       Vec<Instruction>,
    pub nodes:      Vec<ExpressionNode>,
}

#[derive(Debug, Clone)]
pub(super) struct Instruction {
    pub op:    Op,
    pub range: SourceRange,
    /// The expression-tree node this operation evaluates, if any.
    pub node:  Option<usize>,
}

#[derive(Debug, Clone)]
pub(super) enum Op {
    Enter,
    Leave,
    Declare(String, Type),
    Push(i32),
    /// The implicit zero of an array element without an initializer.
    Zero,
    Address(String),
    /// Element `index` of the array being initialized.
    Element(String, i32),
    Load,
    /// The integer promotion of a loaded `char` to `int`.
    Promote,
    Decay(String),
    AddressOf,
    Dereference,
    Index,
    Unary(&'static str),
    Binary(&'static str),
    /// `&E1[E2]`: `E1 + E2` without the indirection (C99 §6.5.3.2p3).
    ElementAddress,
    Store {
        /// An initializer rather than an assignment expression.
        initializer:     bool,
        /// Whether this store ends a full expression (C99 §6.8p4).
        full_expression: bool,
    },
    Increment {
        delta:  i32,
        prefix: bool,
    },
    Pop,
    Jump(usize),
    /// Skip to the target when the condition of the named statement is zero.
    Branch(usize, &'static str),
    Call(String, usize),
    Return,
    Fallthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Type {
    Int,
    /// Plain `char`, signed with 8 bits as on x86-64 (implementation-defined;
    /// C99 §6.2.5p15).
    Char,
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
