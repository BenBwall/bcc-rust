//! Why a run stops without finishing: undefined behaviour, an unsupported
//! operation, a missing function, or a limit.
//!
//! An operation reports a [`Fault`], which does not know where it happened;
//! the main loop attaches the [`Location`] of the instruction it was
//! executing and returns a [`Trap`].

use std::fmt;

use crate::ir::{
    FuncId,
    GlobalId,
    Inst,
    Module,
};

/// Why the interpreter stopped a program before it finished.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Trap {
    /// The program did something the IR leaves undefined.
    UndefinedBehavior(UbKind, Location),
    /// The program used something the interpreter does not implement, such
    /// as an `f80` transcendental function.
    Unsupported(Unsupported, Location),
    /// The program called a declared function that neither the module nor
    /// the host defines.
    UnknownFunction(FuncId, Location),
    /// The program accessed an object declared but not defined by the module.
    UnknownGlobal(GlobalId, Location),
    /// The program ran more instructions than the step limit allows.
    StepLimit,
    /// The program nested more calls than the stack-depth limit allows.
    StackDepthLimit(Location),
    /// The entry function could not be called.
    BadEntry(EntryError),
}

impl Trap {
    /// The trap with its location spelled with the function's name, as
    /// `@main inst4` rather than `func0 inst4`.
    pub(crate) fn display_in<'t>(&'t self, module: &'t Module<'_>) -> impl fmt::Display + 't {
        fmt::from_fn(move |f| {
            let location = match *self {
                | Self::UndefinedBehavior(kind, location) => {
                    write!(f, "undefined behaviour: {kind}")?;
                    location
                },
                | Self::Unsupported(what, location) => {
                    write!(f, "unsupported by the interpreter: {what}")?;
                    location
                },
                | Self::UnknownFunction(callee, location) => {
                    write!(
                        f,
                        "call to @{}, which has no definition",
                        module.function(callee).name
                    )?;
                    location
                },
                | Self::StackDepthLimit(location) => {
                    f.write_str("the call stack exceeded its depth limit")?;
                    location
                },
                | Self::UnknownGlobal(global, location) => {
                    write!(
                        f,
                        "unresolved external object @{}",
                        module.global(global).name
                    )?;
                    location
                },
                | Self::StepLimit | Self::BadEntry(_) => return write!(f, "{self}"),
            };
            write!(
                f,
                " (in @{} at {})",
                module.function(location.func).name,
                location.inst
            )
        })
    }
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            | Self::UndefinedBehavior(kind, location) =>
                write!(f, "undefined behaviour: {kind} ({location})"),
            | Self::Unsupported(what, location) =>
                write!(f, "unsupported by the interpreter: {what} ({location})"),
            | Self::UnknownFunction(callee, location) =>
                write!(f, "call to {callee}, which has no definition ({location})"),
            | Self::StepLimit => f.write_str("the program exceeded its step limit"),
            | Self::UnknownGlobal(global, location) =>
                write!(f, "unresolved external object {global} ({location})"),
            | Self::StackDepthLimit(location) =>
                write!(f, "the call stack exceeded its depth limit ({location})"),
            | Self::BadEntry(error) => write!(f, "cannot run the entry function: {error}"),
        }
    }
}

/// The instruction that was executing when a run trapped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Location {
    pub(crate) func: FuncId,
    pub(crate) inst: Inst,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "in {} at {}", self.func, self.inst)
    }
}

/// A kind of undefined behaviour the interpreter detects.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum UbKind {
    /// Poison in a parameter or return position requiring a defined value.
    Noundef,
    /// `brif` on a poison condition.
    BranchOnPoison,
    /// `switch` on a poison value.
    SwitchOnPoison,
    /// An integer division or remainder by zero.
    DivisionByZero,
    /// An integer division or remainder by poison.
    DivisionByPoison,
    /// `sdiv` or `srem` of the minimum value by -1, or of poison by -1.
    SignedDivisionOverflow,
    /// A memory access, copy, fill or free through a poison pointer, or of a
    /// poison size.
    PoisonAddress,
    /// A memory access through the null pointer.
    NullDereference,
    /// A memory access through a pointer that names no object, such as one
    /// made from an integer.
    NoProvenance,
    /// A memory access through a pointer to an object that was freed or
    /// whose function returned.
    DanglingPointer,
    /// A memory access that reaches outside its object.
    OutOfBounds,
    /// A memory access whose address is not a multiple of its alignment.
    MisalignedAccess,
    /// A store, copy or fill into a `constant` global.
    WriteToConstant,
    /// A load or store through a pointer to a function or a `va_list`
    /// cursor.
    NotData,
    /// A `copy` without `may_overlap` whose source and destination overlap.
    OverlappingCopy,
    /// `free` of a pointer that is not the start of a live heap object.
    InvalidFree,
    /// Execution reached `unreachable`.
    Unreachable,
    /// `call_indirect` through a pointer that is not a function's address.
    CallThroughNonFunction,
    /// `indirect_br` to an address that is not the address of one of the
    /// blocks it lists. GNU extension: GCC "Labels as Values" leaves a jump
    /// to any other address undefined.
    IndirectBranch,
    /// `call_indirect` with a signature that differs from the callee's.
    SignatureMismatch,
    /// `va_start` in a function that is not variadic.
    VaStartOutsideVariadic,
    /// A `va_list` operation on memory that holds no live cursor, or on a
    /// cursor whose function has returned.
    InvalidVaList,
    /// `va_arg` after the last variadic argument.
    VaArgPastEnd,
    /// `va_arg` of a type other than the argument's.
    VaArgTypeMismatch,
    /// A host function read poison where it needed a value, such as a
    /// `printf` argument or a string byte.
    PoisonInHostCall,
    /// `printf` ran out of arguments for its conversions.
    MissingFormatArgument,
    /// A stream argument that is not an open `FILE`, such as one already
    /// closed.
    /// C99: §7.19.3 paragraph 4, p. 264; PDF p. 276.
    InvalidStream,
    /// A comparison function of `qsort` or `bsearch` whose type is not
    /// `int (const void *, const void *)`, or one the host defines.
    /// C99: §7.20.5 paragraph 2, p. 318; PDF p. 330.
    InvalidComparison,
    /// `longjmp` with a buffer that holds no environment `setjmp` saved,
    /// or one whose function has returned since.
    /// C99: §7.13.2.1 paragraph 2, p. 244; PDF p. 256.
    InvalidJmpBuf,
}

impl fmt::Display for UbKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            | Self::Noundef => "poison reached a noundef parameter or return",
            | Self::BranchOnPoison => "branch on poison",
            | Self::SwitchOnPoison => "switch on poison",
            | Self::DivisionByZero => "division by zero",
            | Self::DivisionByPoison => "division by poison",
            | Self::SignedDivisionOverflow => "signed division overflow",
            | Self::PoisonAddress => "poison used as an address or size",
            | Self::NullDereference => "null pointer dereference",
            | Self::NoProvenance => "access through a pointer that names no object",
            | Self::DanglingPointer => "access to a freed object",
            | Self::OutOfBounds => "out-of-bounds access",
            | Self::MisalignedAccess => "misaligned access",
            | Self::WriteToConstant => "write to a constant global",
            | Self::NotData => "data access to a function, label or va_list cursor",
            | Self::OverlappingCopy => "overlapping copy",
            | Self::InvalidFree => "free of a pointer that is not a live heap allocation",
            | Self::Unreachable => "reached unreachable",
            | Self::CallThroughNonFunction => "indirect call through a non-function pointer",
            | Self::IndirectBranch =>
                "computed goto to an address that is not a label of the executing function",
            | Self::SignatureMismatch => "indirect call with the wrong signature",
            | Self::VaStartOutsideVariadic => "va_start in a function that is not variadic",
            | Self::InvalidVaList => "va_list operation on an invalid va_list",
            | Self::VaArgPastEnd => "va_arg past the last argument",
            | Self::VaArgTypeMismatch => "va_arg of the wrong type",
            | Self::PoisonInHostCall => "poison passed to a host function",
            | Self::MissingFormatArgument => "printf has too few arguments",
            | Self::InvalidStream => "a stream argument that is not an open FILE",
            | Self::InvalidComparison => "a qsort or bsearch comparison function of the wrong type",
            | Self::InvalidJmpBuf =>
                "longjmp to an environment that no active setjmp invocation saved",
        })
    }
}

/// Something the interpreter does not implement.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Unsupported {
    /// An `f80` or `f128` library function the host does not compute
    /// exactly.
    WideFloat,
    /// An object larger than [`Limits::max_object_size`] or than memory.
    ///
    /// [`Limits::max_object_size`]: super::Limits::max_object_size
    ObjectSize,
    /// More objects or addresses than the interpreter can number.
    AddressSpace,
    /// A `printf` or `scanf` conversion other than those the default host
    /// implements.
    FormatConversion,
    /// A field width over the host's bounded padding limit.
    FormatWidth,
    /// A post-ABI `va_start` in a module without a supported target.
    VaListTarget,
    /// A function that may return twice other than the `setjmp` family,
    /// such as `vfork` or one declared GNU `returns_twice`.
    ReturnsTwice,
    /// Inline assembly, whose machine code the interpreter cannot run.
    /// GNU extension: GCC manual, "Extended Asm".
    InlineAsm,
    /// A `longjmp` out of a function `qsort` or `bsearch` called.
    CallbackEscape,
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            | Self::WideFloat => "an f80 or f128 library function",
            | Self::ObjectSize => "an object larger than the size limit",
            | Self::AddressSpace => "more objects than the interpreter can address",
            | Self::FormatConversion => "a printf or scanf conversion the host does not implement",
            | Self::FormatWidth => "a printf field width larger than the size limit",
            | Self::VaListTarget => "a target va_list in a module without a known target",
            | Self::ReturnsTwice => "a returns-twice function other than setjmp",
            | Self::InlineAsm =>
                "inline assembly, which only compiled code can run; compile it with -o instead",
            | Self::CallbackEscape => "a longjmp out of a qsort or bsearch comparison",
        })
    }
}

/// Why the entry function could not be called.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EntryError {
    /// No function has the entry's name.
    NotFound,
    /// The entry function is declared but not defined.
    NotDefined,
    /// The arguments do not match the entry's parameters in number or kind.
    Arguments,
    /// A global is larger than the interpreter allows an object to be.
    GlobalTooLarge,
    /// The module has file-scope assembly, which may define symbols or
    /// code the interpreter cannot see.
    ModuleAsm,
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            | Self::NotFound => "no function has its name",
            | Self::NotDefined => "it has no body",
            | Self::Arguments => "the arguments do not match its parameters",
            | Self::GlobalTooLarge => "a global is larger than the interpreter allows",
            | Self::ModuleAsm =>
                "the module has file-scope assembly, which the interpreter cannot run; compile it \
                 with -o instead",
        })
    }
}

/// A trap before the main loop knows where it happened; `Machine::trap`
/// adds the location.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fault {
    Ub(UbKind),
    Unsupported(Unsupported),
    /// The host does not provide the function the instruction calls.
    UnknownFunction,
    /// The module declares the object but supplies no storage for it.
    UnknownGlobal(GlobalId),
    StackDepthLimit,
    /// A trap inside a function the C library called back, such as a
    /// `qsort` comparison, which already knows its location.
    Nested(Trap),
}

impl From<UbKind> for Fault {
    fn from(kind: UbKind) -> Self {
        Self::Ub(kind)
    }
}
