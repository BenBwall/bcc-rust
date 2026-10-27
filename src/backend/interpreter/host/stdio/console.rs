//! The captured standard output and error, and when glibc writes out what
//! their streams hold, which decides how the command line interleaves
//! them.
//!
//! Each [`Channel`] keeps glibc's buffer state for one of the two streams
//! without holding the bytes: the host's output buffer holds `stdout`'s and
//! [`Console::errors`] `stderr`'s, and the channel only records how much of
//! them has been written out. A fully buffered stream writes its buffer out
//! when a byte finds it full and when a block does not fit
//! (`_IO_new_file_overflow` and `_IO_new_file_xsputn`, which then write
//! whole buffers' worth straight through); a line-buffered one also after
//! each newline; an unbuffered one at once. glibc allocates the buffer at
//! the first write, the descriptor's `st_blksize` when that is below
//! `BUFSIZ`, which the command line passes in ([`Channel::block`]); a
//! terminal is line-buffered. `stdout` starts fully buffered (line-buffered
//! on a terminal) and `stderr` unbuffered, and `setvbuf` changes either.
//!
//! [`Console::marks`] records, for each write-out of `stderr`, how much of
//! `stdout` had been written out before it, so the command line can replay
//! both in the order a pipe would have received them.
//!
//! C99: §7.19.3 paragraph 3, p. 264; PDF p. 276 (buffering);
//! §7.19.5.6 paragraphs 2-3, p. 274; PDF p. 286 (`setvbuf`).

use super::{
    BUFSIZ,
    Written,
};

/// The size of the staging area glibc's `printf` writes to a stream through
/// when the stream's put area has no room (`PRINTF_BUFFER_SIZE_TO_FILE_STAGE`).
pub(super) const PRINTF_STAGE: usize = 128;
use crate::util::bump::ArenaVec;

/// What the program wrote to the standard streams, in the order the
/// command line shows it.
pub(in crate::backend::interpreter::host) struct Console<'a> {
    /// Everything written to `stderr`.
    pub(in crate::backend::interpreter::host) errors: ArenaVec<'a, u8>,
    /// For each write-out of `stderr`: how much of the host's output buffer
    /// had been written out before it, and the end of the bytes it wrote in
    /// `errors`.
    pub(in crate::backend::interpreter::host) marks:  ArenaVec<'a, (usize, usize)>,
    /// `stdout`'s buffer, over the host's output buffer.
    pub(in crate::backend::interpreter::host) output: Channel,
    /// `stderr`'s buffer, over `errors`.
    pub(in crate::backend::interpreter::host) error:  Channel,
}

impl Console<'_> {
    /// How much of `output` has reached the terminal or pipe by now.
    pub(in crate::backend::interpreter::host) fn released(&self, output: &[u8]) -> usize {
        self.output.flushed.min(output.len())
    }

    /// Accounts for the bytes of `errors` after `before`, which the program
    /// has just written to `stderr` as `kind` writes, recording a mark when
    /// they, or what the buffer held, are written out.
    pub(super) fn put_errors(&mut self, before: usize, kind: Written, output: &[u8]) {
        let written = self.error.flushed;
        self.error.put(&self.errors, before, kind);
        if self.error.flushed > written {
            self.marks
                .push((self.released(output), self.error.flushed));
        }
    }

    /// Writes out what `stderr`'s buffer holds, as `fflush` does.
    pub(super) fn flush_errors(&mut self, output: &[u8]) {
        if self.error.flushed < self.errors.len() {
            self.error.flushed = self.errors.len();
            self.marks.push((self.released(output), self.error.flushed));
        }
    }
}

/// One standard stream's buffer: its mode, size, and how much of what it
/// was given it has written out.
#[derive(Clone, Copy, Debug)]
pub(in crate::backend::interpreter::host) struct Channel {
    /// How many of the stream's bytes have been written out.
    pub(in crate::backend::interpreter::host) flushed: usize,
    pub(in crate::backend::interpreter::host) mode:    Mode,
    /// The buffer's size once allocated, else 0; 1 for a stream that was
    /// unbuffered then (glibc's `_shortbuf`).
    size:                                              usize,
    /// The size glibc allocates: the descriptor's `st_blksize` when below
    /// `BUFSIZ`.
    pub(in crate::backend::interpreter::host) block:   usize,
    /// Whether the stream is writing (`_IO_CURRENTLY_PUTTING`), so a block
    /// first fills the buffer's free space.
    putting:                                           bool,
}

/// A stream's buffering mode (`_IOFBF`, `_IOLBF` or `_IONBF`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::backend::interpreter::host) enum Mode {
    Full,
    Line,
    Unbuffered,
}

impl Channel {
    /// A stream that has not allocated its buffer yet.
    pub(in crate::backend::interpreter::host) const fn new(mode: Mode) -> Self {
        Self {
            flushed: 0,
            mode,
            size: 0,
            block: BUFSIZ,
            putting: false,
        }
    }

    /// Accounts for `bytes[before..]`, just written as `kind` writes,
    /// moving [`Channel::flushed`] past what glibc writes out.
    pub(in crate::backend::interpreter::host) fn put(
        &mut self,
        bytes: &[u8],
        before: usize,
        kind: Written,
    ) {
        if bytes.len() <= before {
            return;
        }
        // glibc allocates the buffer at the first write: one byte for an
        // unbuffered stream (`_shortbuf`).
        if self.size == 0 {
            self.size = if self.mode == Mode::Unbuffered {
                1
            } else {
                self.block
            };
        }
        let end = bytes.len();
        match kind {
            | Written::Byte => self.put_bytes(bytes, before, end),
            | Written::Block => self.put_block(bytes, before, end),
            // `puts` writes the string as a block and then its newline.
            | Written::Line => {
                self.put_block(bytes, before, end - 1);
                self.put_bytes(bytes, end - 1, end);
            },
            | Written::Formatted => self.put_formatted(bytes, before, end),
        }
    }

    /// Byte by byte through `__overflow`, as `putc` reaches it once the put
    /// area is used up (`_IO_new_file_overflow`): a full buffer is written
    /// out before the next byte goes in, a line-buffered one after each
    /// newline, and an unbuffered one after each byte.
    fn put_bytes(&mut self, bytes: &[u8], start: usize, end: usize) {
        for at in start..end {
            if at - self.flushed >= self.size {
                self.flushed = at;
            }
            if self.mode == Mode::Unbuffered || (self.mode == Mode::Line && bytes[at] == b'\n') {
                self.flushed = at + 1;
            }
        }
        self.putting = true;
    }

    /// The free space of the put area (`_IO_write_end - _IO_write_ptr`) when
    /// the stream holds `at - flushed` bytes: only a fully buffered stream
    /// that is writing has any, since glibc keeps a line-buffered or
    /// unbuffered stream's put area empty so that each byte reaches
    /// `__overflow`.
    fn room(&self, at: usize) -> usize {
        if self.putting && self.mode == Mode::Full {
            self.size.saturating_sub(at - self.flushed)
        } else {
            0
        }
    }

    /// A block (`_IO_new_file_xsputn`): what fits fills the put area, or on
    /// a line-buffered stream that is writing, a block that fits up to its
    /// last newline; the rest, or a newline, writes the buffer out, whole
    /// buffers' worth go straight through, and the remainder goes in byte
    /// by byte (`_IO_default_xsputn`).
    fn put_block(&mut self, bytes: &[u8], start: usize, end: usize) {
        let length = end - start;
        if length == 0 {
            return;
        }
        let mut count = self.room(start);
        let mut must_flush = false;
        if self.mode == Mode::Line && self.putting {
            count = self.size.saturating_sub(start - self.flushed);
            if count >= length
                && let Some(newline) = bytes[start..end].iter().rposition(|&byte| byte == b'\n')
            {
                count = newline + 1;
                must_flush = true;
            }
        }
        let taken = count.min(length);
        let rest = length - taken;
        if rest > 0 || must_flush {
            let direct = rest
                - if self.size >= 128 {
                    rest % self.size
                } else {
                    0
                };
            self.flushed = start + taken + direct;
            self.put_bytes(bytes, start + taken + direct, end);
        }
    }

    /// What the `printf` family formats (`__printf_buffer_to_file`): into
    /// the put area directly while it has room, a byte through `__overflow`
    /// when it fills, and otherwise through a 128-byte staging area that
    /// each fill or the end of the call writes as a block.
    fn put_formatted(&mut self, bytes: &[u8], start: usize, end: usize) {
        let mut at = start;
        while at < end {
            let room = self.room(at);
            if room > 0 {
                at += (end - at).min(room);
                if at < end {
                    self.put_bytes(bytes, at, at + 1);
                    at += 1;
                }
            } else {
                let chunk = (end - at).min(PRINTF_STAGE);
                self.put_block(bytes, at, at + chunk);
                at += chunk;
            }
        }
    }

    /// Changes the mode as `setvbuf` does. A buffer given (`size` is
    /// `Some`) or unbuffered mode first writes out what the stream holds
    /// (`_IO_SETBUF`), and its size becomes the buffer's: zero is
    /// unbuffered. Fully buffered mode without a buffer allocates the
    /// buffer now, if it had none, so that it is not a terminal's line
    /// buffer later.
    pub(super) fn set_buffering(&mut self, mode: Mode, size: Option<usize>, length: usize) {
        self.mode = mode;
        let size = if mode == Mode::Unbuffered {
            Some(0)
        } else {
            size
        };
        match size {
            | Some(size) => {
                self.flushed = length;
                self.putting = false;
                if size == 0 {
                    self.mode = Mode::Unbuffered;
                    self.size = 1;
                } else {
                    self.size = size;
                }
            },
            | None if self.size == 0 && mode == Mode::Full => self.size = self.block,
            | None => {},
        }
    }
}
