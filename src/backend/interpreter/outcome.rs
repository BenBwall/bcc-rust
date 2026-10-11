//! How a run ends, and the limits that stop a runaway program.

use super::value::RuntimeValue;

/// A run that finished without trapping.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Outcome {
    pub(crate) termination: Termination,
    /// The instructions executed.
    pub(crate) steps:       u64,
}

/// How a program ended.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Termination {
    /// The entry function returned, with its result if it has one.
    Returned(Option<RuntimeValue>),
    /// The program called `exit` with this status.
    ///
    /// C99: §7.20.4.3, pp. 315-316; PDF pp. 327-328.
    Exited(i32),
    /// The program called `abort`.
    ///
    /// C99: §7.20.4.1, p. 315; PDF p. 327.
    Aborted,
}

/// The bounds a run may not exceed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Limits {
    /// The most instructions a run may execute.
    pub(crate) steps:           u64,
    /// The most frames the call stack may hold.
    pub(crate) stack_depth:     usize,
    /// The largest object, in bytes; `malloc` of more returns null.
    pub(crate) max_object_size: u64,
}

impl Default for Limits {
    /// A hundred million steps, a million frames and 256 MiB objects.
    fn default() -> Self {
        Self {
            steps:           100_000_000,
            stack_depth:     1_000_000,
            max_object_size: 1 << 28,
        }
    }
}
