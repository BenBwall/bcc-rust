//! Public observations and compact internal operations for the subset engine.

use std::sync::Arc;

use super::clauses::Clause;

/// A compiled bounded-subset program, independent of native compiler storage.
#[derive(Debug, Clone)]
pub struct Program {
    pub(super) source:     String,
    pub(super) functions:  Vec<Function>,
    pub(super) entry:      usize,
    /// Structure types, in definition order; [`Value::Struct`] and
    /// [`Operand::Address`] refer to them by index.
    pub(super) structures: Arc<[Structure]>,
    /// Objects with static storage duration, in declaration order: file-scope
    /// objects and block-scope `static` ones. They are initialized before
    /// `main` runs and take the first object IDs. String literals are
    /// among them, in source order.
    pub(super) statics:    Vec<Static>,
    /// The preprocessing directives the machine runs itself, in source
    /// order: each `#include <stdio.h>`. They run before the static objects
    /// are initialized, as translation phase 4 comes before execution.
    pub(super) directives: Vec<SourceRange>,
}

/// A structure type and its layout on x86-64: each member at an offset
/// aligned for its type, with padding where the next member needs it and at
/// the end, so that an array of the structure keeps every member aligned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    /// The tag: `point` in `struct point`.
    pub name:    String,
    /// Size in bytes, including padding.
    pub size:    usize,
    /// Alignment in bytes: that of its most aligned member.
    pub align:   usize,
    /// Members, in declaration order.
    pub members: Vec<Member>,
}

/// One member of a [`Structure`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::partial_pub_fields,
    reason = "The page reads the layout; the type is the engine's own."
)]
pub struct Member {
    /// The member's identifier.
    pub name:      String,
    /// Its declared type, spelled as in C (`int`, `int[3]`, `struct point`).
    pub r#type:    String,
    /// Byte offset from the start of the structure.
    pub offset:    usize,
    /// Size in bytes.
    pub size:      usize,
    /// Unnamed padding bytes after it, before the next member or the end.
    pub padding:   usize,
    /// The member's first scalar (leaf) within the structure, the unit the
    /// machine addresses: [`Operand::Address`] offsets count leaves.
    pub first:     usize,
    pub(super) ty: Type,
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
    /// Every byte the program has written to standard output so far.
    pub output:          Vec<u8>,
    /// How many bytes at the end of `output` the last operation wrote.
    pub written:         usize,
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
    /// Translation phase 4 ran a preprocessing directive: `#include
    /// <stdio.h>` declared the library's output functions.
    Directive,
    /// Before `main`, an object with static storage duration received its
    /// initial value: zero, or its constant initializer's.
    StaticStorage,
    /// Execution reached the declaration of a block-scope `static` object,
    /// which already exists and does nothing here.
    StaticLocal,
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
    /// `s.m` or `p->m` designated a structure member.
    Member,
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
    /// A call to a library function (`printf`, `puts`, `putchar`) ran to
    /// completion in one step, writing to standard output.
    LibraryCall,
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
            | Self::Directive => "directive",
            | Self::StaticStorage => "static_storage",
            | Self::StaticLocal => "static_local",
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
            | Self::Member => "member",
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
            | Self::LibraryCall => "library_call",
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
            | Self::Member
            | Self::Initialize
            | Self::Assign
            | Self::Increment => "memory",
            | Self::EnterBlock
            | Self::LeaveBlock
            | Self::Declare
            | Self::StaticStorage
            | Self::StaticLocal => "lifetime",
            | Self::Start | Self::Directive | Self::Discard | Self::Jump | Self::Branch =>
                "control",
            | Self::Call | Self::LibraryCall | Self::Return => "call",
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
    /// An lvalue designating the subobject at scalar `offset` of object
    /// `target`.
    Address {
        /// The designated object's ID.
        target:    usize,
        /// The scalar (leaf) where the designated subobject starts: an array
        /// element's index, a member's first leaf, 0 for a whole object.
        offset:    i32,
        /// The structure type, when the lvalue designates a whole structure
        /// rather than one scalar.
        structure: Option<usize>,
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

/// One C object. Array elements and structure members share their enclosing
/// object ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// Stable allocation identity, independent of host addresses.
    pub id:               usize,
    /// Source identifier.
    pub name:             String,
    /// Declared subset type spelling.
    pub r#type:           String,
    /// Owning activation, or 0 for an object with static storage duration,
    /// which belongs to no call.
    pub frame_id:         usize,
    /// `automatic` or `static`.
    pub storage_duration: &'static str,
    /// The function a block-scope `static` object is declared in; `None` for
    /// every other object.
    pub function:         Option<String>,
    /// `alive` or `ended`.
    pub lifetime:         &'static str,
    /// Whether this object is a function parameter.
    pub parameter:        bool,
    /// Whether this object is the array a string literal denotes, which
    /// the program must not modify (C99 §6.4.5p6). Its name is the
    /// literal as written.
    pub literal:          bool,
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
    /// An `int *` or a pointer to a structure; a `None` target means null.
    Pointer {
        /// Target object ID.
        target:    Option<usize>,
        /// The scalar (leaf) it points at (one past the end is
        /// representable).
        offset:    i32,
        /// The array the pointer moves in, as the leaf of its element 0: an
        /// object that is not an array element counts as an array of one
        /// (C99 §6.5.6p7).
        first:     i32,
        /// That array's length in elements.
        length:    i32,
        /// The structure type a pointer to a structure points to.
        structure: Option<usize>,
    },
    /// Fixed-size int array contents.
    Array(Vec<Value>),
    /// A structure's members' values, in declaration order.
    Struct {
        /// Index of its type in the program's structures.
        structure: usize,
        /// Member values, in declaration order.
        members:   Vec<Value>,
    },
}

impl Value {
    /// The null pointer.
    pub(super) const fn null() -> Self {
        Self::Pointer {
            target:    None,
            offset:    0,
            first:     0,
            length:    0,
            structure: None,
        }
    }

    /// A pointer to the scalar `offset` of `target`, moving in the array of
    /// `length` elements whose element 0 is the scalar `first`.
    pub(super) const fn pointer(target: usize, offset: i32, first: i32, length: i32) -> Self {
        Self::Pointer {
            target: Some(target),
            offset,
            first,
            length,
            structure: None,
        }
    }

    /// Whether `self` and `other` are pointers to the same place, or both
    /// null: what `==` compares, whatever array each moves in.
    pub(super) fn same_address(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (
                Self::Pointer { target: a, offset: x, .. },
                Self::Pointer { target: b, offset: y, .. },
            ) if a == b && (a.is_none() || x == y)
        )
    }
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
    /// What made the behavior undefined, for undefined behavior.
    pub undefined:    Option<Undefined>,
}

/// What made a program's behavior undefined: the narration titles the
/// stop by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undefined {
    /// A `+`, `-`, `*` or unary `-` result outside int (C99 §6.5p5).
    SignedOverflow,
    /// `INT_MIN / -1` or `INT_MIN % -1`: the quotient does not fit.
    QuotientOverflow,
    /// `/` or `%` by zero (C99 §6.5.5p5).
    DivisionByZero,
    /// `*` applied to a null pointer (C99 §6.5.3.2p4).
    NullIndirection,
    /// Pointer arithmetic on a null pointer, which points into no array.
    NullArithmetic,
    /// `p[i]` on a null pointer: `*(p + i)`'s addition has no meaning.
    NullSubscript,
    /// An access to an object whose lifetime has ended (C99 §6.2.4p2).
    AfterLifetime,
    /// An access one past the end of an array (C99 §6.5.6p8).
    OnePastEnd,
    /// Pointer arithmetic that leaves the array and its one-past position.
    PointerOutside,
    /// A store into the array of a string literal (C99 §6.4.5p6).
    StringLiteralWrite,
    /// A library function read a string that has no null character before
    /// the end of its array.
    Unterminated,
    /// A null pointer where a library function needs a string (C99
    /// §7.1.4p1).
    NullString,
    /// A `printf` conversion with no argument left for it (C99
    /// §7.19.6.1p2).
    MissingArgument,
    /// A `printf` argument of the wrong type for its conversion (C99
    /// §7.19.6.1p9).
    WrongArgument,
    /// An invalid `printf` conversion specification (C99 §7.19.6.1p9,
    /// §7.19.6.1p6).
    InvalidConversion,
}

impl Diagnostic {
    pub(super) fn new(kind: &'static str, range: SourceRange, message: &str) -> Self {
        Self {
            kind,
            message: String::from(message),
            source_range: range,
            citation: None,
            clause: None,
            undefined: None,
        }
    }

    pub(super) fn limit(range: SourceRange, message: &str) -> Self {
        Self::new("limit", range, message)
    }

    pub(super) fn ub(
        range: SourceRange,
        message: &str,
        clause: &'static Clause,
        undefined: Undefined,
    ) -> Self {
        Self {
            citation: Some(clause.citation),
            clause: Some(clause),
            undefined: Some(undefined),
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
    /// `int` or a structure.
    pub returns:    Type,
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
    /// Reaching the declaration of block-scope static `index`: nothing to do.
    StaticLocal(usize),
    Push(i32),
    /// The implicit zero of an array element or member without an
    /// initializer.
    Zero,
    Address(Place),
    /// The subobject at scalar `offset` of the object `name` being
    /// initialized (`values[2]`, `p.x`); `structure` when it is a whole
    /// structure.
    Element {
        name:      String,
        offset:    i32,
        structure: Option<usize>,
    },
    Load,
    /// Lvalue conversion of a whole structure: a copy of every member.
    LoadStruct(usize),
    /// `.m` (or `->m` when `arrow`) selecting the member that starts at
    /// scalar `offset` of the structure; `structure` when the member is a
    /// structure itself, `array` (its length) when it is an int array, which
    /// decays to a pointer to its first element at once.
    Member {
        name:      String,
        offset:    i32,
        arrow:     bool,
        structure: Option<usize>,
        array:     Option<usize>,
    },
    /// The integer promotion of a loaded `char` to `int`.
    Promote,
    /// The default argument promotion of a `char` passed through a
    /// variadic function's `...` (C99 §6.5.2.2p6-7).
    DefaultPromote,
    Decay(Place),
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
        /// Whether the stored value is a char's, not an int's, so storing it
        /// in a char converts nothing.
        char_value:      bool,
        /// The structure type, for a store that copies a whole structure.
        structure:       Option<usize>,
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
    /// A call to a library function with the arguments' types, after the
    /// default argument promotions for those passed through `...`.
    Library(Library, Vec<Type>),
    /// Initialize the char array the address on the stack designates from
    /// a string literal's characters (C99 §6.7.8p14): `bytes` holds them
    /// without the null, `length` is the array's.
    InitializeString {
        bytes:           Vec<u8>,
        length:          usize,
        /// Whether this ends a full expression (C99 §6.8p4).
        full_expression: bool,
    },
    Return,
    Fallthrough,
}

/// A function `<stdio.h>` declares that the machine runs itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Library {
    /// `int printf(const char *format, ...)`.
    Printf,
    /// `int puts(const char *s)`.
    Puts,
    /// `int putchar(int c)`.
    Putchar,
}

impl Library {
    /// The function `name` names, when `<stdio.h>` declares it.
    pub(super) fn named(name: &str) -> Option<Self> {
        match name {
            | "printf" => Some(Self::Printf),
            | "puts" => Some(Self::Puts),
            | "putchar" => Some(Self::Putchar),
            | _ => None,
        }
    }

    pub(super) const fn name(self) -> &'static str {
        match self {
            | Self::Printf => "printf",
            | Self::Puts => "puts",
            | Self::Putchar => "putchar",
        }
    }
}

/// Where a name's object is found when it is evaluated.
#[derive(Debug, Clone)]
pub(super) enum Place {
    /// An automatic object of the current call, by name.
    Local(String),
    /// Static object `index` of [`Program::statics`].
    Static(usize),
}

/// An object with static storage duration and its initial value, fixed
/// before the program starts.
#[derive(Debug, Clone)]
pub(super) struct Static {
    pub name:     String,
    pub ty:       Type,
    /// The function a block-scope static is declared in.
    pub function: Option<String>,
    /// Its value before `main` runs: the constant initializer's, with zero
    /// for every scalar it does not cover.
    pub value:    Value,
    /// Whether the declaration has an initializer.
    pub explicit: bool,
    /// Whether this is the array of a string literal, named as written.
    pub literal:  bool,
    pub range:    SourceRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Type {
    Int,
    /// Plain `char`, signed with 8 bits as on x86-64 (implementation-defined;
    /// C99 §6.2.5p15).
    Char,
    /// `int *`.
    Pointer,
    /// `int[n]`.
    Array(usize),
    /// A structure, by index into [`Program::structures`].
    Struct(usize),
    /// A pointer to a structure.
    StructPointer(usize),
    /// `char[n]`.
    CharArray(usize),
    /// `char *`, or `const char *` when set.
    CharPointer(bool),
}

impl Type {
    /// Whether values of this type are pointers.
    pub(super) const fn is_pointer(self) -> bool {
        matches!(
            self,
            Self::Pointer | Self::StructPointer(_) | Self::CharPointer(_)
        )
    }

    /// Whether this is an array type, which a name of it decays from.
    pub(super) const fn is_array(self) -> bool {
        matches!(self, Self::Array(_) | Self::CharArray(_))
    }

    /// The pointer an array of this type decays to.
    pub(super) const fn decayed(self) -> Self {
        match self {
            | Self::Array(_) => Self::Pointer,
            | Self::CharArray(_) => Self::CharPointer(false),
            | other => other,
        }
    }
}

pub(super) const MAX_SOURCE: usize = 32 * 1024;
pub(super) const MAX_DEPTH: usize = 64;
pub(super) const MAX_CODE: usize = 8192;
pub(super) const MAX_STEPS: usize = 2048;
pub(super) const MAX_FRAMES: usize = 32;
pub(super) const MAX_OBJECTS: usize = 128;
pub(super) const MAX_STRUCTURES: usize = 32;
pub(super) const MAX_MEMBERS: usize = 32;
pub(super) const MAX_CELLS: usize = 512;
pub(super) const MAX_OPERANDS: usize = 1024;
pub(super) const MAX_TRACE_BYTES: usize = 8 * 1024 * 1024;
/// The most a program may write to standard output.
pub(super) const MAX_OUTPUT: usize = 4096;
/// The most arguments a library call takes.
pub(super) const MAX_ARGUMENTS: usize = 16;
