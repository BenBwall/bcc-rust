//! Where streams meet the POSIX descriptors (`host/posix.rs`): `fileno`,
//! `fdopen`, the descriptor each stream reads and writes through, and the
//! console's order when descriptors and streams both write to it.
//!
//! A stream remembers its descriptor's number, as glibc's `_fileno` does,
//! and holds a host duplicate of what that number referred to, so the two
//! share the file offset. When the number changes meaning (`close`,
//! `dup2`, or an `open` that reuses it), [`DefaultHost::retarget_streams`]
//! points every stream on that number at its new file, or detaches it, so
//! the stream writes where glibc's would.
//!
//! The captured standard output and error stay buffered in the host's
//! console model; [`DefaultHost::route_console`] passes what their streams
//! have written out since the last call to whatever descriptors 1 and 2
//! now refer to, after every host call. A `write` to the console inserts
//! its bytes after what the console has already received, so they appear
//! before what `stdout` still holds, as they do with glibc.
//!
//! POSIX.1-2008: `fileno`, `fdopen` (XSH, "fdopen", "fileno").

use std::fs::File;

use super::{
    super::{
        errno,
        posix::{
            Descriptor,
            Open,
            Sink,
        },
    },
    DefaultHost,
    EOF,
    MAX_STREAMS,
    Target,
};
use crate::{
    backend::interpreter::{
        memory::{
            Memory,
            ObjectKind,
        },
        trap::Fault,
        value::RuntimeValue,
    },
    ir::Align,
};

impl DefaultHost<'_> {
    /// `fileno`: the descriptor of the stream at `pointer`, or -1 with
    /// `EBADF` for a stream without one.
    pub(super) fn fileno(
        &mut self,
        pointer: RuntimeValue,
        memory: &mut Memory<'_>,
    ) -> Result<RuntimeValue, Fault> {
        let stream = self.stream(pointer, memory)?;
        Ok(match self.streams.entries[stream].descriptor {
            | Some(number) => RuntimeValue::Int(u128::from(number.cast_unsigned())),
            | None => {
                self.set_errno(memory, errno::EBADF)?;
                RuntimeValue::Int(EOF)
            },
        })
    }

    /// `fdopen`: a stream on descriptor `number` with `mode`'s access, or
    /// null: `EINVAL` for an unknown mode or one the descriptor's access
    /// does not allow, `EBADF` for a closed descriptor. An appending mode
    /// sets `O_APPEND` and, without `+`, starts at the end, as glibc does.
    pub(super) fn fdopen(
        &mut self,
        number: i128,
        mode: RuntimeValue,
        memory: &mut Memory<'_>,
    ) -> Result<RuntimeValue, Fault> {
        self.text.clear();
        memory.read_c_string(mode, &mut self.text)?;
        let modifiers = self.text.get(1..self.text.len().min(7)).unwrap_or_default();
        let plus = modifiers.contains(&b'+');
        let access = self.text.first().copied();
        if !matches!(access, Some(b'r' | b'w' | b'a')) {
            self.set_errno(memory, errno::EINVAL)?;
            return Ok(RuntimeValue::NULL);
        }
        let readable = access == Some(b'r') || plus;
        let writable = access != Some(b'r') || plus;
        let appending = access == Some(b'a');
        let Some(descriptor) = self.descriptors.get(number) else {
            self.set_errno(memory, errno::EBADF)?;
            return Ok(RuntimeValue::NULL);
        };
        // glibc refuses a direction the descriptor's access mode lacks.
        let any_access = !descriptor.readable && !descriptor.writable;
        if !any_access && ((writable && !descriptor.writable) || (readable && !descriptor.readable))
        {
            self.set_errno(memory, errno::EINVAL)?;
            return Ok(RuntimeValue::NULL);
        }
        let target = match self.target_for(number) {
            | Ok(target) => target,
            | Err(code) => {
                self.set_errno(memory, code)?;
                return Ok(RuntimeValue::NULL);
            },
        };
        let mut target = target;
        if appending && let Some(file) = self.descriptors.file(number) {
            super::super::posix::set_append(file);
        }
        // Without `+`, an appending stream starts at the end.
        if appending
            && !readable
            && let Target::File(file) = &mut target
        {
            _ = std::io::Seek::seek(file, std::io::SeekFrom::End(0)).ok();
        }
        let Some(index) = (3..MAX_STREAMS)
            .find(|&index| matches!(self.streams.entries[index].target, Target::Closed))
        else {
            self.set_errno(memory, errno::EMFILE)?;
            return Ok(RuntimeValue::NULL);
        };
        let stream = u32::try_from(index).expect("a small table");
        let object = memory.allocate(ObjectKind::Stream(stream), 0, Align::BYTE)?;
        let opened = self.streams.open_next();
        let entry = &mut self.streams.entries[index];
        entry.reopen(target, readable, writable, appending);
        entry.object = Some(object);
        entry.opened = opened;
        entry.descriptor = i32::try_from(number).ok();
        Ok(RuntimeValue::Ptr(object))
    }

    /// Gives a file `fopen` or `freopen` opened its descriptor: `number`,
    /// the one `freopen`'s stream had, or the lowest free one; the
    /// descriptor's number, or the `errno` of a failure (`EMFILE`).
    pub(super) fn attach_descriptor(
        &mut self,
        file: &File,
        readable: bool,
        writable: bool,
        number: Option<i32>,
    ) -> Result<i32, i32> {
        let duplicate = file.try_clone().map_err(|error| errno::from_io(&error))?;
        let descriptor = Descriptor {
            open: Open::File(duplicate),
            readable,
            writable,
            pipe: false,
        };
        let number = match number.and_then(|number| usize::try_from(number).ok()) {
            | Some(index) => {
                self.descriptors.place(index, descriptor);
                i32::try_from(index).expect("a small table")
            },
            | None => self
                .descriptors
                .insert(descriptor)
                .map_err(|_| errno::EMFILE)?,
        };
        self.retarget_streams(number.into());
        Ok(number)
    }

    /// A stream target on what descriptor `number` refers to: a duplicate
    /// of its host file, the console's input, or the console's output or
    /// error buffered as glibc buffers them; `Detached` once it is closed.
    fn target_for(&mut self, number: i128) -> Result<Target, i32> {
        let terminal = self.terminal_output;
        let (output_block, error_block) = (self.console.output.block, self.console.error.block);
        Ok(
            match self.descriptors.get(number).map(|entry| &entry.open) {
                | None => Target::Detached,
                | Some(Open::Console(Sink::Input)) => Target::Input,
                | Some(&Open::Console(sink)) => Target::Console {
                    sink,
                    block: if sink == Sink::Output {
                        output_block
                    } else {
                        error_block
                    },
                    terminal: terminal && sink == Sink::Output,
                },
                | Some(Open::File(file)) =>
                    Target::File(file.try_clone().map_err(|error| errno::from_io(&error))?),
            },
        )
    }

    /// Points every open stream whose descriptor is `number` at what the
    /// number now refers to, keeping its buffers, as glibc's streams follow
    /// their `_fileno`. The standard output and error streams on the
    /// console stay on it: [`DefaultHost::route_console`] follows their
    /// descriptors.
    pub(in crate::backend::interpreter::host) fn retarget_streams(&mut self, number: i128) {
        for index in 0..MAX_STREAMS {
            let entry = &self.streams.entries[index];
            if entry.descriptor.map(i128::from) != Some(number)
                || matches!(
                    entry.target,
                    Target::Closed | Target::Output | Target::Errors
                )
            {
                continue;
            }
            // A duplicate that the host cannot make leaves the stream
            // failing as for a closed descriptor.
            let target = self.target_for(number).unwrap_or(Target::Detached);
            self.streams.entries[index].target = target;
        }
    }

    /// Closes the descriptor of `stream`, which is being closed or
    /// reopened, as glibc's `_IO_file_close` does; whether there was one to
    /// close.
    pub(super) fn close_stream_descriptor(&mut self, stream: usize) -> bool {
        self.route_console();
        let Some(number) = self.streams.entries[stream].descriptor.take() else {
            return false;
        };
        let number = i128::from(number);
        let closed = self.descriptors.close(number);
        if closed {
            self.retarget_streams(number);
        }
        closed
    }

    /// Writes `bytes` to the console's output or error as a `write` to its
    /// descriptor does: after what has reached the console, before what
    /// the standard stream's buffer still holds.
    pub(in crate::backend::interpreter::host) fn write_console(
        &mut self,
        sink: Sink,
        bytes: &[u8],
    ) {
        self.route_console();
        self.insert_console(sink, bytes);
    }

    /// Adds `bytes` to what the console's `sink` has received.
    fn insert_console(&mut self, sink: Sink, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        match sink {
            | Sink::Output => {
                let at = self.console.output.flushed.min(self.output.len());
                self.output.extend_from_slice(bytes);
                self.output[at..].rotate_right(bytes.len());
                self.console.output.flushed = at + bytes.len();
                self.routed_output = self.console.output.flushed;
                self.inserted_output += bytes.len();
            },
            | Sink::Errors => {
                let errors = &mut self.console.errors;
                let at = self.console.error.flushed.min(errors.len());
                errors.extend_from_slice(bytes);
                errors[at..].rotate_right(bytes.len());
                self.console.error.flushed = at + bytes.len();
                self.routed_errors = self.console.error.flushed;
                let released = self.console.released(&self.output);
                self.console
                    .marks
                    .push((released, self.console.error.flushed));
            },
            | Sink::Input => {},
        }
    }

    /// Passes on what the console's standard streams wrote out since the
    /// last time to what descriptors 1 and 2 refer to now, when that is not
    /// their own console stream, and what streams on the console through
    /// other descriptors wrote out. A failed write sets the stream's error
    /// indicator and its `errno`, as glibc's write-out does.
    pub(in crate::backend::interpreter::host) fn route_console(&mut self) {
        // Take both standard streams' write-outs that leave the console
        // before passing either on, so that each lands after what its
        // destination already holds.
        let output = self.take_written_out(Sink::Output);
        let errors = self.take_written_out(Sink::Errors);
        if output {
            let bytes = std::mem::replace(
                &mut self.io_buffer,
                crate::util::bump::ArenaVec::new_in(self.allocator),
            );
            self.deliver(1, &bytes);
            self.io_buffer = bytes;
        }
        if errors {
            let bytes = std::mem::replace(
                &mut self.io_errors,
                crate::util::bump::ArenaVec::new_in(self.allocator),
            );
            self.deliver(2, &bytes);
            self.io_errors = bytes;
        }
        // Streams on the console through other descriptors.
        for index in 0..MAX_STREAMS {
            let entry = &mut self.streams.entries[index];
            let Target::Console { sink, .. } = entry.target else {
                continue;
            };
            if entry.console_writes.is_empty() {
                continue;
            }
            let mut bytes = std::mem::replace(
                &mut self.io_buffer,
                crate::util::bump::ArenaVec::new_in(self.allocator),
            );
            bytes.clear();
            let entry = &mut self.streams.entries[index];
            bytes.extend_from_slice(&entry.console_writes);
            entry.console_writes.clear();
            self.insert_console(sink, &bytes);
            self.io_buffer = bytes;
        }
    }

    /// Passes on what the console's standard `stream` has just written
    /// out, as its write-out does; whether that worked.
    pub(super) fn route_flushed(&mut self, stream: usize) -> bool {
        let failed_before = self.streams.entries[stream].errno.is_some();
        self.route_console();
        failed_before || self.streams.entries[stream].errno.is_none()
    }

    /// Takes what the console's standard output or error stream wrote out
    /// since the last routing out of the console, into `io_buffer` or
    /// `io_errors`, when its descriptor (1 or 2) no longer refers to it;
    /// whether there is something to pass on.
    fn take_written_out(&mut self, sink: Sink) -> bool {
        let mut removed = 0;
        let (number, channel, bytes, routed, buffer, removed) = match sink {
            | Sink::Errors => (
                2,
                &mut self.console.error,
                &mut self.console.errors,
                &mut self.routed_errors,
                &mut self.io_errors,
                &mut removed,
            ),
            | _ => (
                1,
                &mut self.console.output,
                &mut self.output,
                &mut self.routed_output,
                &mut self.io_buffer,
                &mut self.removed_output,
            ),
        };
        let flushed = channel.flushed.min(bytes.len());
        if flushed <= *routed {
            return false;
        }
        let start = *routed;
        *routed = flushed;
        if matches!(
            self.descriptors.get(number).map(|entry| &entry.open),
            Some(Open::Console(open)) if *open == sink
        ) {
            return false;
        }
        buffer.clear();
        buffer.extend_from_slice(&bytes[start..flushed]);
        drop(bytes.drain(start..flushed));
        *removed += flushed - start;
        channel.flushed = start;
        *routed = start;
        for mark in &mut self.console.marks {
            if sink == Sink::Errors {
                mark.1 = mark.1.min(start);
            } else {
                mark.0 = mark.0.min(start);
            }
        }
        true
    }

    /// Writes what standard stream `number` (1 or 2) wrote out to what its
    /// descriptor refers to now, which is not that stream's own console.
    fn deliver(&mut self, number: usize, bytes: &[u8]) {
        let key = i128::try_from(number).expect("a standard stream");
        let failure = match self.descriptors.get(key) {
            | None => Some(errno::EBADF),
            | Some(descriptor) if !descriptor.writable => Some(errno::EBADF),
            | Some(descriptor) => match descriptor.open {
                | Open::Console(sink) => {
                    self.insert_console(sink, bytes);
                    None
                },
                | Open::File(_) => {
                    let file = self.descriptors.file(key).expect("a file");
                    super::super::posix::write_all(file, bytes).err()
                },
            },
        };
        if let Some(code) = failure {
            let entry = &mut self.streams.entries[number];
            entry.error = true;
            entry.errno = Some(code);
        }
    }
}
