//! The program's file descriptor table. Each open descriptor refers to the
//! captured console (standard input, output or error) or to a host file,
//! which the host's own descriptor holds: a duplicate is a host duplicate,
//! so the two share the file offset and the status flags, as POSIX open
//! file descriptions do. The table hands out the lowest free number, as
//! Linux does, up to glibc's default `RLIMIT_NOFILE` of 1024.

use std::{
    fs::File,
    io,
};

/// How many descriptors a process may have open: Linux's default soft
/// `RLIMIT_NOFILE`.
pub(in crate::backend::interpreter::host) const MAX_DESCRIPTORS: usize = 1024;

/// The open descriptors, by number.
pub(in crate::backend::interpreter::host) struct Descriptors {
    entries: [Option<Descriptor>; MAX_DESCRIPTORS],
    /// A duplicate of the process's standard input, once a descriptor reads
    /// it, so that reads bypass Rust's buffered `Stdin`.
    input:   Option<File>,
}

/// One open descriptor.
pub(in crate::backend::interpreter::host) struct Descriptor {
    pub(in crate::backend::interpreter::host) open:     Open,
    /// The access its open file description gives (`O_ACCMODE`).
    pub(in crate::backend::interpreter::host) readable: bool,
    pub(in crate::backend::interpreter::host) writable: bool,
    /// Whether it is an end of a pipe, which the host keeps non-blocking
    /// so that a read or write that would wait forever is caught rather
    /// than hanging.
    pub(in crate::backend::interpreter::host) pipe:     bool,
}

/// What a descriptor refers to.
#[expect(
    variant_size_differences,
    reason = "A host file is one descriptor; the table holds few entries."
)]
pub(in crate::backend::interpreter::host) enum Open {
    Console(Sink),
    File(File),
}

/// A standard stream of the console.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::backend::interpreter::host) enum Sink {
    Input,
    Output,
    Errors,
}

impl Descriptors {
    /// Descriptors 0, 1 and 2 open on the console, the rest closed.
    pub(in crate::backend::interpreter::host) fn new() -> Self {
        let mut entries = std::array::from_fn(|_| None);
        for (number, sink) in [Sink::Input, Sink::Output, Sink::Errors]
            .into_iter()
            .enumerate()
        {
            entries[number] = Some(Descriptor {
                open:     Open::Console(sink),
                readable: sink == Sink::Input,
                writable: sink != Sink::Input,
                pipe:     false,
            });
        }
        Self {
            entries,
            input: None,
        }
    }

    /// The open descriptor `number`, if it is one.
    pub(in crate::backend::interpreter::host) fn get(&self, number: i128) -> Option<&Descriptor> {
        let index = usize::try_from(number).ok()?;
        self.entries.get(index)?.as_ref()
    }

    /// The lowest closed descriptor number from `from` up, if any.
    pub(in crate::backend::interpreter::host) fn lowest_free(&self, from: usize) -> Option<usize> {
        (from..MAX_DESCRIPTORS).find(|&index| self.entries[index].is_none())
    }

    /// Opens `descriptor` at the lowest free number, which it returns, or
    /// gives it back when every number is taken (`EMFILE`).
    pub(in crate::backend::interpreter::host) fn insert(
        &mut self,
        descriptor: Descriptor,
    ) -> Result<i32, Descriptor> {
        match self.lowest_free(0) {
            | Some(index) => {
                self.entries[index] = Some(descriptor);
                Ok(i32::try_from(index).expect("a small table"))
            },
            | None => Err(descriptor),
        }
    }

    /// Opens `descriptor` at `number`, closing what was open there.
    pub(in crate::backend::interpreter::host) fn place(
        &mut self,
        number: usize,
        descriptor: Descriptor,
    ) {
        self.entries[number] = Some(descriptor);
    }

    /// Closes descriptor `number`; whether it was open.
    pub(in crate::backend::interpreter::host) fn close(&mut self, number: i128) -> bool {
        usize::try_from(number)
            .ok()
            .and_then(|index| self.entries.get_mut(index))
            .and_then(Option::take)
            .is_some()
    }

    /// A new descriptor for the open file description of `number`, as
    /// `dup` makes one.
    pub(in crate::backend::interpreter::host) fn duplicate(
        &self,
        number: i128,
    ) -> Option<io::Result<Descriptor>> {
        let descriptor = self.get(number)?;
        let open = match &descriptor.open {
            | Open::Console(sink) => Ok(Open::Console(*sink)),
            | Open::File(file) => file.try_clone().map(Open::File),
        };
        Some(open.map(|open| Descriptor {
            open,
            readable: descriptor.readable,
            writable: descriptor.writable,
            pipe: descriptor.pipe,
        }))
    }

    /// The host file that reading the console's input, or the file
    /// `number` refers to, goes through.
    pub(in crate::backend::interpreter::host) fn file(
        &mut self,
        number: i128,
    ) -> Option<&mut File> {
        let index = usize::try_from(number).ok()?;
        match self.entries.get_mut(index)?.as_mut()? {
            | Descriptor {
                open: Open::File(file),
                ..
            } => Some(file),
            | Descriptor {
                open: Open::Console(Sink::Input),
                ..
            } => {
                if self.input.is_none() {
                    self.input = standard_input();
                }
                self.input.as_mut()
            },
            | _ => None,
        }
    }
}

/// A duplicate of the process's standard input descriptor.
fn standard_input() -> Option<File> {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd as _;
        io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .ok()
            .map(File::from)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle as _;
        io::stdin()
            .as_handle()
            .try_clone_to_owned()
            .ok()
            .map(File::from)
    }
}
