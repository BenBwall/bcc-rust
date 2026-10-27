//! One stream as glibc's libio keeps it. Its buffer has the size glibc
//! picks (the file's `st_blksize` when that is below `BUFSIZ`, which makes
//! 4096 for most files and 1024 for a Linux terminal). Reading refills it by
//! one `read` when the program has taken what it holds; a request at least
//! as large as the buffer reads straight into the program's array instead.
//! `ungetc` steps back over the byte just read when it is that byte, and
//! otherwise pushes into a separate pushback area that a seek discards. The
//! cached file offset decides how a seek refills the buffer. Writing fills
//! the buffer, which glibc writes out when it is full, at `fflush`, before a
//! seek or a read, at `fclose`, at the program's normal end, and after each
//! newline on a terminal.
//!
//! Each function follows its libio counterpart in glibc 2.43, named in its
//! documentation (`libio/fileops.c`, `libio/genops.c`, `libio/ioseekoff.c`
//! and `libio/ioftell.c`). glibc 2.39 agrees except where [`Stream::sync`]
//! and [`Stream::seek`] say otherwise. Captured standard output and error are
//! not streams of this kind: the host's console orders them.

use std::{
    fs::File,
    io::{
        self,
        IsTerminal as _,
        Read as _,
        Seek as _,
        SeekFrom,
        Write as _,
    },
};

use super::{
    super::errno::{
        EBADF,
        EINVAL,
        from_io,
    },
    Console,
};
use crate::{
    backend::interpreter::value::Pointer,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// glibc's `BUFSIZ`, its buffer size when the file's `st_blksize` is not
/// smaller.
const BUFSIZ: usize = 8192;

/// The buffer size of a stream modelled as a terminal: the `st_blksize` of
/// a Linux pseudo-terminal.
const TERMINAL_BUFFER: usize = 1024;

/// How many buffers' worth one direct read takes at most. glibc reads the
/// whole request at once; reading it in pieces makes no difference, since
/// glibc keeps reading directly while the rest is at least a buffer.
const DIRECT_READ_BUFFERS: usize = 16;

/// A stream's destination or source and its state.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent indicators and modes, as glibc keeps them in a stream's flag bits."
)]
pub(super) struct Stream<'a> {
    pub(super) target:    Target,
    /// The end-of-file and error indicators.
    pub(super) eof:       bool,
    pub(super) error:     bool,
    /// The access its mode gives (glibc's `_IO_NO_READS` and
    /// `_IO_NO_WRITES`), and whether it appends.
    pub(super) readable:  bool,
    pub(super) writable:  bool,
    pub(super) appending: bool,
    /// Whether to treat the stream as a terminal whatever it reads
    /// (`--interpret-terminal-stdin`).
    pub(super) terminal:  bool,
    /// The memory object of a stream `fopen` made, freed by `fclose`.
    pub(super) object:    Option<Pointer>,
    /// The buffer's size once it is allocated (`_IO_buf_end -
    /// _IO_buf_base`), else 0, and whether glibc line-buffers the stream,
    /// as it does a terminal.
    size:                 usize,
    line_buffered:        bool,
    /// The buffer up to `_IO_read_end`, with `_IO_read_base` and
    /// `_IO_read_ptr` as indices. While the pushback area holds bytes, the
    /// read position stays where `ungetc` left the buffer, and the base
    /// moves up to it, as glibc's `_IO_default_pbackfail` sets it.
    bytes:                ArenaVec<'a, u8>,
    base:                 usize,
    position:             usize,
    /// The pushback area's unread bytes, the next one last, and whether
    /// glibc reads from the area (`_IO_IN_BACKUP`), which it still does
    /// after the last byte until a read moves back to the buffer.
    backup:               ArenaVec<'a, u8>,
    in_backup:            bool,
    /// The file offset at the buffer's end, when glibc knows it (`_offset`;
    /// `None` is `_IO_pos_BAD`). Only a seek or a write sets it; a read keeps
    /// it up to date, and a read that finds the end forgets it.
    offset:               Option<i64>,
    /// Whether a byte operation has oriented the stream (`_IO_fwide`), as a
    /// write, `fgets`, `fscanf` or a refill does; until then a seek leaves
    /// the pushback area alone.
    pub(super) oriented:  bool,
    /// glibc's put area while the stream writes (`_IO_CURRENTLY_PUTTING`):
    /// where in the buffer the bytes it holds start, and those bytes, which
    /// reach the file only when glibc writes them out.
    putting:              bool,
    write_base:           usize,
    pending:              ArenaVec<'a, u8>,
    /// When the stream was opened, which orders `fflush(NULL)` and the
    /// flush at exit: glibc's `_IO_list_all` puts each stream it opens
    /// first.
    pub(super) opened:    u64,
    /// The `errno` the last failing operation set, until the host passes it
    /// to the program: `EBADF` for a direction the mode does not allow, and
    /// the system's code when a read, write or seek fails.
    pub(super) errno:     Option<i32>,
}

/// Where a stream's bytes go or come from.
pub(super) enum Target {
    Closed,
    /// The host's captured standard output.
    Output,
    /// The host's captured standard error.
    Errors,
    /// The process's standard input, until its first use opens it as a
    /// file (on Unix, a duplicate of descriptor 0).
    Input,
    File(File),
}

/// Writes out a line-buffered `stdout`, as glibc's underflow of a
/// line-buffered input stream does before it reads: the captured output if
/// it is a terminal's, or the stream `freopen` put on a terminal.
pub(super) struct Release<'c, 'a, 's> {
    pub(super) console: &'c mut Console<'a>,
    /// How many bytes the program has written to the output buffer.
    pub(super) written: usize,
    /// The `stdout` stream, unless it is the stream being read.
    pub(super) stdout:  Option<&'c mut Stream<'s>>,
}

impl Release<'_, '_, '_> {
    /// `_IO_OVERFLOW (stdout, EOF)` when `stdout` is open, may write, and
    /// is line-buffered.
    fn release(&mut self) {
        let Some(stdout) = self.stdout.as_deref_mut() else {
            return;
        };
        match stdout.target {
            | Target::Output if self.console.line_buffered => self.console.flushed = self.written,
            | Target::File(_) if stdout.line_buffered && stdout.writable => {
                let _ = stdout.write_out();
            },
            | _ => {},
        }
    }
}

impl Stream<'_> {
    /// The next byte, or `None` at end of file or on an error, which set
    /// the indicator (`__uflow` and `_IO_default_uflow`).
    pub(super) fn read(&mut self, stdout: &mut Release<'_, '_, '_>) -> Option<u8> {
        if !self.end_write() {
            return None;
        }
        if self.in_backup {
            if let Some(byte) = self.backup.pop() {
                return Some(byte);
            }
            // `__uflow` moves back to the buffer, orienting the stream.
            self.in_backup = false;
            self.oriented = true;
        }
        let byte = match self.bytes.get(self.position) {
            | Some(&byte) => byte,
            | None => {
                self.oriented = true;
                self.underflow(stdout)?
            },
        };
        self.position += 1;
        Some(byte)
    }

    /// Refills the empty buffer with one `read` and returns its first byte
    /// (`_IO_new_file_underflow`). A set end-of-file indicator fails without
    /// reading, as C99 requires; refilling a line-buffered stream first
    /// releases a line-buffered `stdout` (only `stdout`: glibc no longer
    /// flushes every line-buffered stream).
    /// C99: §7.19.7.1 paragraph 3, p. 296; PDF p. 308.
    fn underflow(&mut self, stdout: &mut Release<'_, '_, '_>) -> Option<u8> {
        if self.eof {
            return None;
        }
        if !self.readable {
            self.error = true;
            self.errno = Some(EBADF);
            return None;
        }
        if let Some(&byte) = self.bytes.get(self.position) {
            return Some(byte);
        }
        self.allocate();
        if self.line_buffered {
            stdout.release();
        }
        (self.base, self.position) = (0, 0);
        self.bytes.clear();
        self.bytes.resize(self.size, 0);
        let count = read_from(&mut self.target, &mut self.bytes);
        self.bytes.truncate(*count.as_ref().unwrap_or(&0));
        match count {
            | Ok(0) | Err(_) => {
                match &count {
                    | Ok(_) => self.eof = true,
                    | Err(error) => {
                        self.error = true;
                        self.errno = Some(from_io(error));
                    },
                }
                self.offset = None;
                None
            },
            | Ok(count) => {
                self.offset = self.offset.map(|offset| offset + to_offset(count));
                Some(self.bytes[0])
            },
        }
    }

    /// Reads up to `wanted` bytes for `fread`, passing them to `store`
    /// piece by piece; how many it read (`_IO_file_xsgetn`). Once the buffer
    /// is empty, a rest smaller than the buffer refills it, while a larger
    /// rest is read straight into the array: whole buffers' worth, without
    /// the end-of-file check or the release of `stdout` that a refill does.
    pub(super) fn read_block<E>(
        &mut self,
        wanted: usize,
        stdout: &mut Release<'_, '_, '_>,
        mut store: impl FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<usize, E> {
        if !self.end_write() {
            return Ok(0);
        }
        self.allocate();
        let mut want = wanted;
        while want > 0 {
            if self.in_backup {
                let take = want.min(self.backup.len());
                for _ in 0..take {
                    let byte = self.backup.pop().expect("counted");
                    store(&[byte])?;
                }
                want -= take;
                // Only a request the area cannot satisfy moves back to the
                // buffer.
                self.in_backup = want == 0;
                continue;
            }
            let have = self.bytes.len() - self.position;
            let take = want.min(have);
            store(&self.bytes[self.position..self.position + take])?;
            self.position += take;
            want -= take;
            if want == 0 {
                break;
            }
            if want < self.size {
                self.oriented = true;
                if self.underflow(stdout).is_none() {
                    break;
                }
                continue;
            }
            let mut count = want;
            if self.size >= 128 {
                count -= want % self.size;
            }
            (self.base, self.position) = (0, 0);
            self.bytes.clear();
            self.bytes
                .resize(count.min(self.size * DIRECT_READ_BUFFERS), 0);
            let read = read_from(&mut self.target, &mut self.bytes);
            let read = match read {
                | Ok(0) => {
                    self.eof = true;
                    0
                },
                | Err(error) => {
                    self.error = true;
                    self.errno = Some(from_io(&error));
                    0
                },
                | Ok(read) => read,
            };
            let stored = store(&self.bytes[..read]);
            self.bytes.clear();
            stored?;
            if read == 0 {
                break;
            }
            want -= read;
            self.offset = self.offset.map(|offset| offset + to_offset(read));
        }
        Ok(wanted - want)
    }

    /// Pushes `byte` back (`_IO_sputbackc` and `_IO_default_pbackfail`):
    /// by stepping back over it when it is the byte just read from the
    /// buffer, else into the pushback area.
    /// C99: §7.19.7.11 paragraphs 2-5, pp. 300-301; PDF pp. 312-313.
    pub(super) fn push_back(&mut self, byte: u8) {
        if !self.in_backup && self.position > self.base && self.bytes[self.position - 1] == byte {
            self.position -= 1;
        } else {
            if !self.in_backup {
                self.base = self.position;
                self.in_backup = true;
            }
            self.backup.push(byte);
        }
        self.eof = false;
    }

    /// Moves the file position to `offset` from `whence`, saying whether
    /// it could (`fseek`: `_IO_seekoff_unlocked` and
    /// `_IO_new_file_seekoff`). The pushback area goes first, even when the
    /// seek then fails; a seek that fails keeps the buffer, so a byte pushed
    /// back by stepping over it survives. Within the buffer's bytes a seek
    /// only moves the read position; otherwise it seeks to the block
    /// boundary below and reads up to the position, so that the buffer holds
    /// the bytes before it. A stream whose offset glibc does not know seeks
    /// `SEEK_CUR` relative to the descriptor and empties the buffer.
    /// C99: §7.19.9.2 paragraphs 2-5, p. 303; PDF p. 315.
    pub(super) fn seek(&mut self, offset: i64, whence: Whence) -> bool {
        let mut offset = offset;
        // `_IO_seekoff_unlocked` gives up the pushback area of a stream a byte
        // operation has oriented, counting its unread bytes for `SEEK_CUR`.
        // Before glibc 2.41 (commit 3e1d8d1d1dca) it skipped a stream whose
        // buffer was never allocated, whose `_IO_save_base` `ungetc` leaves
        // null, so the seek below dropped the area uncounted.
        if self.oriented && self.in_backup {
            if whence == Whence::Current {
                offset = offset.wrapping_sub(to_offset(self.backup.len()));
            }
            self.drop_backup();
        }
        // POSIX: after an `fflush` the descriptor's offset must be exact, so
        // nothing read ahead and nothing waiting to be written.
        let must_be_exact =
            !self.in_backup && self.base == self.bytes.len() && self.pending.is_empty();
        if !self.end_write() {
            return false;
        }
        if self.size == 0 {
            self.drop_backup();
        }
        self.allocate();
        match whence {
            | Whence::Current => {
                // The unread bytes of the area being read: an unoriented
                // stream still reading its pushback area counts only those.
                let ahead = if self.in_backup {
                    self.backup.len()
                } else {
                    self.bytes.len() - self.position
                };
                offset = offset.wrapping_sub(to_offset(ahead));
                let Some(known) = self.offset else {
                    return self.seek_descriptor(SeekFrom::Current(offset));
                };
                offset = offset.wrapping_add(known);
                if offset < 0 {
                    self.errno = Some(EINVAL);
                    return false;
                }
            },
            | Whence::Start => {},
            | Whence::End => match self.regular_file_size() {
                | Some(size) => offset = offset.wrapping_add(size),
                | None => return self.seek_descriptor(SeekFrom::End(offset)),
            },
        }
        self.drop_backup();
        if let Some(known) = self.offset {
            let start = known - to_offset(self.bytes.len());
            if (start..known).contains(&offset) {
                self.base = 0;
                self.position = to_count(offset - start);
                self.eof = false;
                return true;
            }
        }
        let Ok(start) = u64::try_from(offset) else {
            self.errno = Some(EINVAL);
            return false;
        };
        if !self.readable {
            return self.seek_descriptor(SeekFrom::Start(start));
        }
        let size = to_offset(self.size);
        let mut block = offset & !(size - 1);
        let mut delta = offset - block;
        if delta > size {
            (block, delta) = (offset, 0);
        }
        let Ok(at) = self.descriptor_seek(SeekFrom::Start(block.cast_unsigned())) else {
            return false;
        };
        self.bytes.clear();
        let mut count = 0;
        if delta > 0 {
            self.bytes.resize(
                if must_be_exact {
                    to_count(delta)
                } else {
                    self.size
                },
                0,
            );
            let read = read_from(&mut self.target, &mut self.bytes);
            count = read.as_ref().map_or(-1, |&read| to_offset(read));
            if count < delta {
                // The file ends before the position: glibc seeks on to it.
                (self.base, self.position) = (0, 0);
                self.bytes.clear();
                let rest = if count < 0 { delta } else { delta - count };
                return self.seek_descriptor(SeekFrom::Current(rest));
            }
            self.bytes.truncate(to_count(count));
        }
        self.base = 0;
        self.position = to_count(delta);
        self.offset = i64::try_from(at).ok().map(|at| at + count);
        self.eof = false;
        true
    }

    /// Seeks the descriptor and empties the buffer, glibc's fallback when it
    /// cannot seek within its buffer or to a block boundary.
    fn seek_descriptor(&mut self, position: SeekFrom) -> bool {
        let Ok(at) = self.descriptor_seek(position) else {
            return false;
        };
        self.eof = false;
        self.offset = i64::try_from(at).ok();
        self.drop_backup();
        (self.base, self.position) = (0, 0);
        self.bytes.clear();
        true
    }

    /// The file position, or `None` where there is none (`_IO_ftell` and
    /// `do_ftell`): the descriptor's offset less what was read ahead and
    /// pushed back. A stream without a buffer yet does not count its
    /// pushback area.
    /// C99: §7.19.9.4 paragraph 2, p. 304; PDF p. 316.
    pub(super) fn tell(&mut self) -> Option<i64> {
        if !self.pending.is_empty() {
            // The bytes glibc holds count after the descriptor's offset; an
            // appending stream's go at the file's end.
            let at = if self.appending {
                let at = self.descriptor_seek(SeekFrom::End(0)).ok()?;
                self.offset = i64::try_from(at).ok();
                at
            } else {
                self.descriptor_seek(SeekFrom::Current(0)).ok()?
            };
            return i64::try_from(at)
                .ok()
                .map(|at| at + to_offset(self.pending.len()));
        }
        let at = i64::try_from(self.descriptor_seek(SeekFrom::Current(0)).ok()?).ok()?;
        if self.size == 0 {
            return Some(at);
        }
        // `do_ftell` subtracts the unread bytes of the area being read: the
        // pushback area while it holds any, else the buffer.
        let (ahead, behind) = if self.in_backup {
            (self.backup.len(), self.bytes.len() - self.base)
        } else {
            (self.bytes.len() - self.position, 0)
        };
        // A character device that reports offset 0 after a read has no
        // position.
        let position = at - to_offset(ahead);
        if (at == 0 && ahead > 0) || position < 0 {
            self.errno = Some(EINVAL);
            return None;
        }
        // `_IO_ftell` then subtracts the buffer's unread bytes behind the
        // pushback area, without checking the sign again.
        Some(position - to_offset(behind))
    }

    /// Writes out what glibc holds and gives back what the buffer read
    /// ahead, seeking the descriptor back to the stream's position, and
    /// forgets the offset (`fflush`: `_IO_new_file_sync`); whether it worked.
    /// glibc 2.43 drops the pushback area too and seeks back over its bytes;
    /// 2.39 seeks back over the pushback area alone and keeps the buffer.
    pub(super) fn sync(&mut self) -> bool {
        if !self.write_out() {
            return false;
        }
        let ahead = if self.in_backup {
            self.backup.len() + self.bytes.len() - self.base
        } else {
            self.bytes.len() - self.position
        };
        self.drop_backup();
        if ahead > 0 {
            match self.descriptor_seek(SeekFrom::Current(-to_offset(ahead))) {
                | Ok(_) => self.bytes.truncate(self.position),
                | Err(error) if error.kind() != io::ErrorKind::NotSeekable => return false,
                | Err(_) => {},
            }
        }
        self.offset = None;
        true
    }

    /// Whether the area being read has unread bytes, which glibc 2.43's
    /// `fflush(NULL)` gives back.
    pub(super) fn reads_ahead(&self) -> bool {
        if self.in_backup {
            !self.backup.is_empty()
        } else {
            self.position < self.bytes.len()
        }
    }

    /// Writes out the bytes glibc holds (`_IO_do_write`), whether or not
    /// they all reach the file: the buffer is empty afterwards either way,
    /// and a failed write sets the error indicator. Whether it worked.
    pub(super) fn write_out(&mut self) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        let (written, complete) = write_file(&mut self.target, &self.pending, &mut self.errno);
        self.pending.clear();
        self.write_base = 0;
        self.wrote(written, complete)
    }

    /// Accounts for a write straight to the file (`new_do_write` and
    /// `_IO_new_file_write`) that took `written` bytes and may not have
    /// been `complete`: an appending stream no longer knows its offset,
    /// another moves it past what reached the file, and a short write sets
    /// the error indicator. Whether it was complete.
    fn wrote(&mut self, written: usize, complete: bool) -> bool {
        self.offset = if self.appending {
            None
        } else {
            self.offset.map(|offset| offset + to_offset(written))
        };
        self.error |= !complete;
        complete
    }

    /// Writes one byte as `fputc` does (`_IO_new_file_overflow`), saying
    /// whether it could: into the buffer, writing it out first when it is
    /// full, and after a newline on a line-buffered stream.
    pub(super) fn put_char(&mut self, byte: u8) -> bool {
        self.oriented = true;
        if !self.start_write() {
            return false;
        }
        if self.room() == 0 && !self.write_out() {
            return false;
        }
        self.pending.push(byte);
        !(self.line_buffered && byte == b'\n') || self.write_out()
    }

    /// Writes what the `printf` family formatted, as glibc's
    /// `__printf_buffer_to_file` does: byte by byte into the buffer, like
    /// `fputc`. A stream that may not write fails even with nothing to
    /// write. Whether all of it was written.
    pub(super) fn put_formatted(&mut self, bytes: &[u8]) -> bool {
        self.oriented = true;
        if !self.writable {
            self.error = true;
            self.errno = Some(EBADF);
            return false;
        }
        bytes.iter().all(|&byte| self.put_char(byte))
    }

    /// Writes `bytes` as `fputs` and `fwrite` do (`_IO_new_file_xsputn`),
    /// returning how many it took: what fits fills the buffer if the stream
    /// was already writing; otherwise the buffer is written out, whole
    /// buffers' worth go to the file directly, and the rest fills the
    /// buffer. Nothing to write is no write at all.
    pub(super) fn put_block(&mut self, bytes: &[u8]) -> usize {
        self.oriented = true;
        if bytes.is_empty() {
            return 0;
        }
        let writing = self.putting;
        if !self.start_write() {
            return 0;
        }
        if self.line_buffered {
            return bytes
                .iter()
                .take_while(|&&byte| self.put_char(byte))
                .count();
        }
        let mut taken = 0;
        if writing {
            taken = bytes.len().min(self.room());
            self.pending.extend_from_slice(&bytes[..taken]);
        }
        if taken < bytes.len() {
            if !self.write_out() {
                return taken;
            }
            let rest = bytes.len() - taken;
            let blocks = if self.size >= 128 {
                rest - rest % self.size
            } else {
                rest
            };
            if blocks > 0 {
                let (written, complete) = write_file(
                    &mut self.target,
                    &bytes[taken..taken + blocks],
                    &mut self.errno,
                );
                self.write_base = 0;
                taken += written;
                if !self.wrote(written, complete) {
                    return taken;
                }
            }
            while taken < bytes.len() && self.put_char(bytes[taken]) {
                taken += 1;
            }
        }
        taken
    }

    /// How many more bytes the buffer can hold before glibc writes it out.
    fn room(&self) -> usize {
        self.size - self.write_base - self.pending.len()
    }

    /// Readies the stream to write, saying whether it may. Writing starts
    /// where reading stopped, so the descriptor seeks back over what was
    /// read ahead (`_IO_new_file_overflow` and `new_do_write`). C99
    /// §7.19.5.3p6 asks for a seek between reading and writing.
    fn start_write(&mut self) -> bool {
        if !self.writable {
            self.error = true;
            self.errno = Some(EBADF);
            return false;
        }
        if self.putting {
            return true;
        }
        self.allocate();
        // Pushed-back bytes are given up, and the write position moves back
        // over as many bytes of the buffer; reading that used the whole
        // buffer leaves it to start again.
        let (mut start, mut end) = if self.in_backup {
            (
                self.base - self.backup.len().min(self.base),
                self.bytes.len(),
            )
        } else {
            (self.position, self.bytes.len())
        };
        if start == self.size {
            (start, end) = (0, 0);
        }
        if end > start {
            self.offset = self
                .descriptor_seek(SeekFrom::Current(-to_offset(end - start)))
                .ok()
                .and_then(|at| i64::try_from(at).ok());
        }
        self.drop_backup();
        (self.base, self.position) = (0, 0);
        self.bytes.clear();
        (self.putting, self.write_base) = (true, start);
        self.pending.clear();
        true
    }

    /// Leaves writing, as glibc does before it reads or seeks
    /// (`_IO_switch_to_get_mode`): what it holds is written out first, and
    /// a write that fails keeps the stream writing and fails the read or
    /// seek.
    fn end_write(&mut self) -> bool {
        if self.putting {
            if !self.write_out() {
                return false;
            }
            (self.putting, self.write_base) = (false, 0);
        }
        true
    }

    /// Gives up the pushback area, back to reading the buffer
    /// (`_IO_free_backup_area`).
    fn drop_backup(&mut self) {
        self.backup.clear();
        self.in_backup = false;
    }

    /// Allocates the buffer on first use (`_IO_doallocbuf` and
    /// `_IO_file_doallocate`): `st_blksize` when it is below `BUFSIZ`, line
    /// buffering for a terminal.
    fn allocate(&mut self) {
        if self.size != 0 {
            return;
        }
        self.open_input();
        (self.size, self.line_buffered) = if self.terminal {
            (TERMINAL_BUFFER, true)
        } else {
            buffering(&self.target)
        };
    }

    /// Opens the process's standard input as a file on Unix, so that it
    /// seeks and reports its type as glibc's `stdin` does.
    fn open_input(&mut self) {
        #[cfg(unix)]
        if matches!(self.target, Target::Input) {
            use std::os::fd::AsFd as _;
            if let Ok(descriptor) = io::stdin().as_fd().try_clone_to_owned() {
                self.target = Target::File(File::from(descriptor));
            }
        }
    }

    /// Seeks the stream's descriptor, keeping the `errno` of a failure; one
    /// that cannot seek fails as a pipe does.
    fn descriptor_seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.open_input();
        let result = match &mut self.target {
            | Target::File(file) => file.seek(position),
            | _ => Err(io::ErrorKind::NotSeekable.into()),
        };
        if let Err(error) = &result {
            self.errno = Some(from_io(error));
        }
        result
    }

    /// The size of a regular file, which `SEEK_END` counts from; other
    /// files seek relative to the descriptor's end.
    fn regular_file_size(&self) -> Option<i64> {
        match &self.target {
            | Target::File(file) => file
                .metadata()
                .ok()
                .filter(std::fs::Metadata::is_file)
                .and_then(|metadata| i64::try_from(metadata.len()).ok()),
            | _ => None,
        }
    }

    /// Opens the stream afresh on `target` with the given access, keeping
    /// its memory object and its buffers' storage.
    pub(super) fn reopen(
        &mut self,
        target: Target,
        readable: bool,
        writable: bool,
        appending: bool,
    ) {
        self.target = target;
        (self.eof, self.error) = (false, false);
        (self.readable, self.writable, self.appending) = (readable, writable, appending);
        self.terminal = false;
        (self.size, self.line_buffered) = (0, false);
        self.bytes.clear();
        (self.base, self.position) = (0, 0);
        self.drop_backup();
        self.offset = None;
        self.oriented = false;
        (self.putting, self.write_base) = (false, 0);
        self.pending.clear();
    }
}

impl<'a> Stream<'a> {
    /// A closed stream whose buffers live in `arena`.
    pub(super) fn new_in(arena: &'a Bump) -> Self {
        Self {
            target:        Target::Closed,
            eof:           false,
            error:         false,
            readable:      false,
            writable:      false,
            appending:     false,
            terminal:      false,
            object:        None,
            size:          0,
            line_buffered: false,
            bytes:         ArenaVec::new_in(arena),
            base:          0,
            position:      0,
            backup:        ArenaVec::new_in(arena),
            in_backup:     false,
            offset:        None,
            oriented:      false,
            putting:       false,
            write_base:    0,
            pending:       ArenaVec::new_in(arena),
            opened:        0,
            errno:         None,
        }
    }
}

/// The buffer size and line buffering glibc gives a stream on `target`.
fn buffering(target: &Target) -> (usize, bool) {
    match target {
        | Target::File(file) => {
            let Ok(metadata) = file.metadata() else {
                return (BUFSIZ, false);
            };
            #[cfg(unix)]
            {
                use std::os::unix::fs::{
                    FileTypeExt as _,
                    MetadataExt as _,
                };
                let block = usize::try_from(metadata.blksize()).unwrap_or(BUFSIZ);
                let size = if block > 0 && block < BUFSIZ {
                    block
                } else {
                    BUFSIZ
                };
                (
                    size,
                    metadata.file_type().is_char_device() && file.is_terminal(),
                )
            }
            #[cfg(not(unix))]
            {
                _ = metadata;
                (BUFSIZ, file.is_terminal())
            }
        },
        | Target::Input => (BUFSIZ, io::stdin().is_terminal()),
        | Target::Output | Target::Errors | Target::Closed => (BUFSIZ, false),
    }
}

/// One `read` into `buffer` from `target`; a stream without a source fails
/// as for a bad descriptor.
fn read_from(target: &mut Target, buffer: &mut [u8]) -> io::Result<usize> {
    match target {
        | Target::File(file) => file.read(buffer),
        | Target::Input => io::stdin().lock().read(buffer),
        | Target::Output | Target::Errors | Target::Closed =>
            Err(io::Error::from(io::ErrorKind::PermissionDenied)),
    }
}

/// The origin of a seek: `SEEK_SET`, `SEEK_CUR` or `SEEK_END`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Whence {
    Start,
    Current,
    End,
}

/// Writes `bytes` to `target`'s file: how many reached it, and whether all
/// did, keeping the `errno` of a write that failed.
fn write_file(target: &mut Target, bytes: &[u8], errno: &mut Option<i32>) -> (usize, bool) {
    let Target::File(file) = target else {
        *errno = Some(EBADF);
        return (0, false);
    };
    let mut written = 0;
    while written < bytes.len() {
        match file.write(&bytes[written..]) {
            | Ok(0) => return (written, false),
            | Err(error) => {
                *errno = Some(from_io(&error));
                return (written, false);
            },
            | Ok(count) => written += count,
        }
    }
    (written, true)
}

/// A byte count as a file offset.
fn to_offset(count: usize) -> i64 {
    i64::try_from(count).expect("buffer sizes fit a file offset")
}

/// A non-negative offset within a buffer as a byte count.
fn to_count(offset: i64) -> usize {
    usize::try_from(offset).expect("an offset within a buffer")
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "The test names temporary files with std paths; the arena rule covers the \
              interpreter, not its tests."
)]
mod tests {
    use std::fs::File;

    use super::{
        Console,
        Release,
        Stream,
        Target,
    };
    use crate::util::bump::{
        ArenaVec,
        Bump,
    };

    /// Refilling a terminal's input writes out a line-buffered `stdout`
    /// even when `freopen` put it on a file (a terminal device), and
    /// refilling a regular file's input does not.
    #[cfg(not(miri))]
    #[test]
    fn terminal_refill_writes_out_a_reopened_line_buffered_stdout() {
        let directory = std::env::temp_dir();
        let input = directory.join(format!("bcc-stdio-refill-in-{}", std::process::id()));
        let output = directory.join(format!("bcc-stdio-refill-out-{}", std::process::id()));
        std::fs::write(&input, "x\n").unwrap();
        let arena = Bump::new();
        let mut console = Console {
            errors:        ArenaVec::new_in(&arena),
            marks:         ArenaVec::new_in(&arena),
            flushed:       0,
            line_buffered: false,
            scanned:       0,
            line_end:      0,
        };
        let mut stdout = Stream::new_in(&arena);
        stdout.reopen(
            Target::File(File::create(&output).unwrap()),
            false,
            true,
            false,
        );
        stdout.terminal = true;
        assert!(stdout.put_char(b'A'));
        let mut held = [0, 0];
        for (terminal, held) in [false, true].into_iter().zip(&mut held) {
            let mut stdin = Stream::new_in(&arena);
            stdin.reopen(
                Target::File(File::open(&input).unwrap()),
                true,
                false,
                false,
            );
            stdin.terminal = terminal;
            let mut release = Release {
                console: &mut console,
                written: 0,
                stdout:  Some(&mut stdout),
            };
            assert_eq!(stdin.read(&mut release), Some(b'x'));
            *held = arena.read_to_str_lossy(&output).unwrap().len();
        }
        std::fs::remove_file(&input).unwrap();
        std::fs::remove_file(&output).unwrap();
        assert_eq!(held, [0, 1]);
    }
}
