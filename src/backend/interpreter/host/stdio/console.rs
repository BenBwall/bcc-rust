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
//! Once a wide function orients the stream, the channel counts wide
//! characters instead of bytes, as glibc's wide put area holds them
//! (`_IO_wfile_overflow` and `_IO_wfile_xsputn`): as many as the byte buffer
//! holds bytes, or a quarter as many in a buffer `setvbuf` supplied
//! (`_IO_wfile_doallocate`); writing it out converts and writes all it holds.
//!
//! [`Console::marks`] records, for each write-out of `stderr`, how much of
//! `stdout` had been written out before it, so the command line can replay
//! both in the order a pipe would have received them.
//!
//! C99: §7.19.3 paragraph 3, p. 266; PDF p. 278 (buffering);
//! §7.19.5.6 paragraphs 2-3, p. 274; PDF p. 286 (`setvbuf`).

use super::{
    BUFSIZ,
    Written,
};
use crate::util::bump::ArenaVec;

/// The size of the staging area glibc's `printf` writes to a stream through
/// when the stream's put area has no room (`PRINTF_BUFFER_SIZE_TO_FILE_STAGE`),
/// in bytes, or in wide characters for `wprintf`.
pub(super) const PRINTF_STAGE: usize = 128;

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
    /// A `printf` that `failed` keeps only what glibc keeps.
    pub(super) fn put_errors(&mut self, before: usize, kind: Written, failed: bool, output: &[u8]) {
        let written = self.error.flushed;
        if failed {
            let kept = self.error.put_failed_formatted(&self.errors, before);
            self.errors.truncate(kept);
        } else {
            self.error.put(&self.errors, before, kind);
        }
        if self.error.flushed > written {
            self.marks.push((self.released(output), self.error.flushed));
        }
    }

    /// Writes out what `stderr`'s buffer holds, as `fflush` does.
    pub(super) fn flush_errors(&mut self, output: &[u8]) {
        if self.error.flushed < self.errors.len() {
            self.error.flush(self.errors.len());
            self.marks.push((self.released(output), self.error.flushed));
        }
    }
}

/// One stream's buffer: its mode, size, and how much of what it was given it
/// has written out, in bytes or, for a wide-oriented stream, in wide
/// characters.
#[derive(Clone, Copy, Debug)]
pub(in crate::backend::interpreter::host) struct Channel {
    /// How many of the stream's bytes have been written out.
    pub(in crate::backend::interpreter::host) flushed: usize,
    pub(in crate::backend::interpreter::host) mode: Mode,
    /// The byte buffer's size once allocated, else 0; 1 for a stream that
    /// was unbuffered then (glibc's `_shortbuf`).
    size: usize,
    /// The size glibc allocates: the descriptor's `st_blksize` when below
    /// `BUFSIZ`.
    pub(in crate::backend::interpreter::host) block: usize,
    /// Whether the stream is writing (`_IO_CURRENTLY_PUTTING`), so a block
    /// first fills the buffer's free space.
    putting: bool,
    /// How many units (bytes, or wide characters) the buffer holds.
    held: usize,
    /// Whether `setvbuf` supplied the buffer (`_IO_USER_BUF`).
    user: bool,
    /// The wide put area once a wide function oriented the stream.
    wide: Option<WideArea>,
}

/// A wide-oriented stream's put area.
#[derive(Clone, Copy, Debug)]
struct WideArea {
    /// Whether the stream's conversion, fixed when it was oriented, is
    /// UTF-8, whose characters span several bytes.
    utf8: bool,
    /// Its size in wide characters once allocated, else 0.
    size: usize,
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
            held: 0,
            user: false,
            wide: None,
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
        self.allocate();
        let end = bytes.len();
        match kind {
            | Written::Byte => self.put_units(bytes, before, end),
            | Written::Block => self.put_block(bytes, before, end),
            // `puts` writes the string as a block and then its newline.
            | Written::Line => {
                self.put_block(bytes, before, end - 1);
                self.put_units(bytes, end - 1, end);
            },
            | Written::Formatted => _ = self.put_formatted(bytes, before, end, false),
        }
    }

    /// How much of `bytes[before..]`, which the `printf` family formatted
    /// before it met a character it could not convert, glibc keeps: what it
    /// wrote into the put area or staged through before, not what its
    /// staging area still held, which the failure discards. Accounts for
    /// what it keeps.
    pub(in crate::backend::interpreter::host) fn put_failed_formatted(
        &mut self,
        bytes: &[u8],
        before: usize,
    ) -> usize {
        if bytes.len() <= before {
            return bytes.len();
        }
        self.allocate();
        self.put_formatted(bytes, before, bytes.len(), true)
    }

    /// Writes out what the buffer holds, up to `length`, as `fflush` does.
    pub(in crate::backend::interpreter::host) fn flush(&mut self, length: usize) {
        self.flushed = length;
        self.held = 0;
    }

    /// The wide put area of a file stream with the byte buffer `size` (0 if
    /// not yet allocated), which `setvbuf` may have supplied, converting as
    /// UTF-8 or not.
    pub(super) fn for_wide_stream(mode: Mode, size: usize, user: bool, utf8: bool) -> Self {
        let mut channel = Self::new(mode);
        channel.size = size;
        channel.user = user;
        channel.set_wide(utf8);
        channel
    }

    /// Keeps `other`'s count of what was written out, which positions in a
    /// stream's log of converted bytes refer to.
    pub(super) fn keep_position(&mut self, other: &Self) {
        self.flushed = other.flushed;
    }

    /// Orients the stream for wide characters, converting as UTF-8 or not.
    pub(in crate::backend::interpreter::host) fn set_wide(&mut self, utf8: bool) {
        if self.wide.is_none() {
            self.wide = Some(WideArea { utf8, size: 0 });
            self.held = 0;
        }
    }

    /// Allocates the buffers at the first write, as glibc does: a byte for an
    /// unbuffered stream, else the block size; and for a wide stream as many
    /// wide characters as the byte buffer holds bytes, or a quarter as many
    /// in a buffer `setvbuf` supplied.
    fn allocate(&mut self) {
        if self.size == 0 {
            self.size = if self.mode == Mode::Unbuffered {
                1
            } else {
                self.block
            };
        }
        let (size, user, unbuffered) = (self.size, self.user, self.mode == Mode::Unbuffered);
        if let Some(wide) = &mut self.wide
            && wide.size == 0
        {
            wide.size = match () {
                | () if unbuffered => 1,
                | () if user => size.div_ceil(4),
                | () => size,
            };
        }
    }

    /// The buffer's size in the units it counts.
    fn capacity(&self) -> usize {
        self.wide.map_or(self.size, |wide| wide.size)
    }

    /// Whether units are UTF-8 characters rather than bytes.
    fn multibyte(&self) -> bool {
        matches!(self.wide, Some(WideArea { utf8: true, .. }))
    }

    /// Where the unit (a byte, or a wide character's bytes) starting at `at`
    /// ends, at most the end of `bytes`.
    fn unit_end(&self, bytes: &[u8], at: usize) -> usize {
        match bytes.get(at) {
            | Some(&first) if self.multibyte() =>
                (at + super::super::locale::sequence_length(true, first)).min(bytes.len()),
            | _ => (at + 1).min(bytes.len()),
        }
    }

    /// Where `count` units starting at `at` end, at most the end of `bytes`.
    fn advance(&self, bytes: &[u8], mut at: usize, count: usize) -> usize {
        if !self.multibyte() {
            return at.saturating_add(count).min(bytes.len());
        }
        for _ in 0..count {
            if at >= bytes.len() {
                break;
            }
            at = self.unit_end(bytes, at);
        }
        at
    }

    /// How many units `bytes[start..end]` holds.
    fn units(&self, bytes: &[u8], start: usize, end: usize) -> usize {
        if !self.multibyte() {
            return end.saturating_sub(start);
        }
        let mut count = 0;
        let mut at = start;
        while at < end {
            at = self.unit_end(bytes, at);
            count += 1;
        }
        count
    }

    /// Unit by unit through `__overflow`, as `putc` reaches it once the put
    /// area is used up (`_IO_new_file_overflow`, `_IO_wfile_overflow`): a
    /// full buffer is written out before the next unit goes in, a
    /// line-buffered one after each newline, and an unbuffered one after
    /// each unit.
    fn put_units(&mut self, bytes: &[u8], start: usize, end: usize) {
        let capacity = self.capacity();
        let mut at = start;
        while at < end {
            let next = self.unit_end(bytes, at);
            if self.held >= capacity {
                self.flush(at);
            }
            self.held += 1;
            if self.mode == Mode::Unbuffered || (self.mode == Mode::Line && bytes[at] == b'\n') {
                self.flush(next);
            }
            at = next;
        }
        self.putting = true;
    }

    /// The free space of the put area (`_IO_write_end - _IO_write_ptr`):
    /// only a fully buffered stream that is writing has any, since glibc
    /// keeps a line-buffered or unbuffered stream's put area empty so that
    /// each unit reaches `__overflow`.
    fn room(&self) -> usize {
        if self.putting && self.mode == Mode::Full {
            self.capacity().saturating_sub(self.held)
        } else {
            0
        }
    }

    /// A block (`_IO_new_file_xsputn`, or `_IO_wfile_xsputn` for a wide
    /// stream): what fits fills the put area, or on a line-buffered stream
    /// that is writing, a block that fits up to its last newline. A byte
    /// stream then writes the buffer out for the rest, or for that newline,
    /// sends whole buffers' worth straight through, and puts the remainder
    /// unit by unit; a wide stream puts the rest unit by unit and then
    /// writes the buffer out for the newline.
    fn put_block(&mut self, bytes: &[u8], start: usize, end: usize) {
        let length = self.units(bytes, start, end);
        if length == 0 {
            return;
        }
        let mut count = self.room();
        let mut must_flush = false;
        if self.mode == Mode::Line && self.putting {
            count = self.capacity().saturating_sub(self.held);
            if count >= length
                && let Some(newline) = bytes[start..end].iter().rposition(|&byte| byte == b'\n')
            {
                count = self.units(bytes, start, start + newline + 1);
                must_flush = true;
            }
        }
        let taken = count.min(length);
        let taken_end = self.advance(bytes, start, taken);
        self.held += taken;
        let rest = length - taken;
        if self.wide.is_some() {
            self.put_units(bytes, taken_end, end);
            if must_flush {
                self.flush(end);
            }
            return;
        }
        if rest > 0 || must_flush {
            let capacity = self.capacity();
            let direct = rest - if capacity >= 128 { rest % capacity } else { 0 };
            let direct_end = self.advance(bytes, taken_end, direct);
            self.flush(direct_end);
            self.putting = true;
            self.put_units(bytes, direct_end, end);
        }
    }

    /// What the `printf` family formats (`__printf_buffer_to_file` and its
    /// wide counterpart): into the put area directly while it has room, a
    /// unit through `__overflow` when it fills, and otherwise through a
    /// 128-unit staging area that each fill or the end of the call writes as
    /// a block. A call that `failed` discards what its staging area holds at
    /// the end. Where what it keeps ends.
    fn put_formatted(&mut self, bytes: &[u8], start: usize, end: usize, failed: bool) -> usize {
        let mut at = start;
        while at < end {
            let room = self.room();
            if room > 0 {
                let filled = self.advance(bytes, at, room).min(end);
                self.held += self.units(bytes, at, filled);
                at = filled;
                if at < end {
                    let next = self.unit_end(bytes, at);
                    self.put_units(bytes, at, next);
                    at = next;
                }
            } else {
                let chunk_end = self.advance(bytes, at, PRINTF_STAGE).min(end);
                if failed && chunk_end == end {
                    return at;
                }
                self.put_block(bytes, at, chunk_end);
                at = chunk_end;
            }
        }
        end
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
                self.flush(length);
                self.putting = false;
                if size == 0 {
                    self.mode = Mode::Unbuffered;
                    self.size = 1;
                } else {
                    self.size = size;
                    self.user = true;
                }
            },
            | None if self.size == 0 && mode == Mode::Full => self.size = self.block,
            | None => {},
        }
    }
}
