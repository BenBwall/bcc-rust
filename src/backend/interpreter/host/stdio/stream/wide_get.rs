//! A wide stream's get areas as glibc 2.43 keeps them (`struct
//! _IO_wide_data`, `libio/wfileops.c` and `libio/wgenops.c`): the main wide
//! buffer, which `_IO_wfile_underflow` fills by converting the byte buffer,
//! and the backup buffer `ungetwc` falls back to, with glibc's read and
//! save pointers as indices. Each function follows its glibc counterpart
//! step by step, since `ftell`, `fflush`, `fseek` and `fclose` compute from
//! those pointers, including where the pointers of one area meet the byte
//! buffer of another. glibc 2.39 computes the same, except at `fclose`
//! (`Stream::close_out`).
//!
//! Reading order: [`WideGet`], then `read_wide`, `push_back_wide`,
//! `tell_wide`, `sync_wide` and `seek_wide`.

use std::io::SeekFrom;

use super::{
    EBADF,
    EILSEQ,
    EINVAL,
    ESPIPE,
    Release,
    Stream,
    Whence,
    from_io,
    read_from,
    to_offset,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// glibc's wide get areas.
pub(in crate::backend::interpreter::host) struct WideGet<'a> {
    /// The main wide buffer (`_IO_buf_base` up), sized once allocated, and
    /// its size in characters (`_IO_wfile_doallocate`), 0 until then.
    main:      ArenaVec<'a, u32>,
    capacity:  usize,
    /// The backup buffer while one is allocated (`_IO_save_base` is not
    /// null).
    backup:    ArenaVec<'a, u32>,
    /// `_IO_read_base`, `_IO_read_ptr` and `_IO_read_end`: in the backup
    /// buffer while `in_backup`, else in the main one.
    read:      Area,
    /// `_IO_save_base` and `_IO_save_end`, `None` without a backup buffer:
    /// in the main buffer while `in_backup`, else the backup buffer's
    /// bounds.
    save:      Option<(usize, usize)>,
    in_backup: bool,
}

/// A get area's base, read position and end.
#[derive(Clone, Copy, Default, Debug)]
struct Area {
    base: usize,
    ptr:  usize,
    end:  usize,
}

/// The size of the backup buffer glibc first allocates
/// (`_IO_wdefault_pbackfail`).
const BACKUP_SIZE: usize = 128;

/// glibc's `MB_LEN_MAX`, the bytes `_IO_wfile_underflow` keeps aside for a
/// character a buffer is too small to hold.
const HELD_BYTES: usize = 16;

/// Why a conversion into the wide buffer stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Conversion {
    /// It used the bytes it was given or filled the buffer.
    Done,
    /// The bytes end inside a character.
    Partial,
    /// The bytes are not a character in the stream's conversion.
    Invalid,
}

impl<'a> WideGet<'a> {
    pub(super) fn new_in(arena: &'a Bump) -> Self {
        Self {
            main:      ArenaVec::new_in(arena),
            capacity:  0,
            backup:    ArenaVec::new_in(arena),
            read:      Area::default(),
            save:      None,
            in_backup: false,
        }
    }
}

impl WideGet<'_> {
    /// Empties both areas and frees the buffers, as reopening a stream or
    /// `setvbuf` does.
    pub(super) fn reset(&mut self) {
        self.main.clear();
        self.capacity = 0;
        self.backup.clear();
        self.read = Area::default();
        self.save = None;
        self.in_backup = false;
    }

    /// Whether the area being read has unread characters (`_IO_read_ptr <
    /// _IO_read_end`), which `fflush(NULL)` and `fclose` give back.
    pub(super) fn reads_ahead(&self) -> bool {
        self.read.ptr < self.read.end
    }

    /// Whether the backup area is being read (`_IO_in_backup`).
    pub(super) const fn in_backup(&self) -> bool {
        self.in_backup
    }

    /// The buffer the read pointers are in.
    fn current(&self) -> &[u32] {
        if self.in_backup {
            &self.backup
        } else {
            &self.main
        }
    }

    /// `_IO_switch_to_wbackup_area`.
    fn switch_to_backup(&mut self) {
        let save = self.save.expect("a backup buffer");
        self.in_backup = true;
        self.save = Some((self.read.base, self.read.end));
        self.read = Area {
            base: save.0,
            ptr:  save.1,
            end:  save.1,
        };
    }

    /// `_IO_switch_to_main_wget_area`.
    fn switch_to_main(&mut self) {
        let save = self.save.expect("a backup buffer");
        self.in_backup = false;
        self.save = Some((self.read.base, self.read.end));
        self.read = Area {
            base: save.0,
            ptr:  save.0,
            end:  save.1,
        };
    }

    /// `_IO_free_wbackup_area`.
    fn free_backup(&mut self) {
        if self.in_backup {
            self.switch_to_main();
        }
        self.save = None;
        self.backup.clear();
    }

    /// Makes the main area empty at the buffer's start (`_IO_wsetg` with
    /// `_IO_buf_base` thrice).
    fn empty_main(&mut self) {
        self.read = Area::default();
    }
}

impl Stream<'_> {
    /// The next wide character (`_IO_getwc_unlocked` and `__wuflow`): from
    /// the area being read, else from the main area once the backup area
    /// is used up, else after refilling the main buffer, freeing the
    /// backup buffer first. `None` at the end of the file or on an error,
    /// which set the indicators and `errno`.
    pub(in crate::backend::interpreter::host) fn read_wide(
        &mut self,
        stdout: &mut Release<'_, '_, '_>,
    ) -> Option<u32> {
        if !self.end_write() {
            return None;
        }
        let get = &mut self.wide_get;
        if get.read.ptr < get.read.end {
            let character = get.current()[get.read.ptr];
            get.read.ptr += 1;
            return Some(character);
        }
        if get.in_backup {
            get.switch_to_main();
            if get.read.ptr < get.read.end {
                let character = get.main[get.read.ptr];
                get.read.ptr += 1;
                return Some(character);
            }
        }
        if get.save.is_some() {
            get.free_backup();
        }
        self.wide_underflow(stdout)?;
        let get = &mut self.wide_get;
        let character = get.main[get.read.ptr];
        get.read.ptr += 1;
        Some(character)
    }

    /// Pushes `character` back as `ungetwc` does (`_IO_sputbackwc`): the
    /// character before the read position only moves the position back.
    /// Otherwise `_IO_wdefault_pbackfail` moves the byte position back when
    /// the byte before it equals the character, as glibc compares them, or
    /// stores the character before the backup area's position, switching
    /// to that area (with a buffer of 128, doubled when it fills).
    pub(in crate::backend::interpreter::host) fn push_back_wide(&mut self, character: u32) {
        let get = &mut self.wide_get;
        if get.read.ptr > get.read.base && get.current()[get.read.ptr - 1] == character {
            get.read.ptr -= 1;
            return;
        }
        if get.read.ptr > get.read.base
            && !get.in_backup
            && self.position > 0
            && u32::from(self.bytes[self.position - 1]) == character
        {
            self.position -= 1;
            return;
        }
        if !get.in_backup {
            // `save_for_wbackup` copies nothing without stream markers.
            if get.save.is_none() {
                get.backup.clear();
                get.backup.resize(BACKUP_SIZE, 0);
                get.save = Some((0, BACKUP_SIZE));
            }
            get.read.base = get.read.ptr;
            get.switch_to_backup();
        } else if get.read.ptr <= get.read.base {
            let old = get.read.end - get.read.base;
            let size = 2 * old;
            get.backup.truncate(get.read.end);
            drop(get.backup.drain(..get.read.base));
            get.backup.resize(size, 0);
            get.backup.rotate_right(size - old);
            get.read = Area {
                base: 0,
                ptr:  size - old,
                end:  size,
            };
        }
        get.read.ptr -= 1;
        let at = get.read.ptr;
        if get.in_backup {
            get.backup[at] = character;
        } else {
            get.main[at] = character;
        }
    }

    /// Refills the empty main wide buffer (`_IO_wfile_underflow`): it
    /// converts what the byte buffer still holds; failing that, it moves an
    /// incomplete sequence to the byte buffer's start and reads after it,
    /// keeping too short a buffer's bytes aside until they complete a
    /// character. A set end-of-file indicator fails without reading.
    fn wide_underflow(&mut self, stdout: &mut Release<'_, '_, '_>) -> Option<()> {
        if self.eof {
            return None;
        }
        if !self.readable {
            self.error = true;
            self.errno = Some(EBADF);
            return None;
        }
        if self.position < self.bytes.len() {
            let start = self.position;
            self.wide_get.empty_main();
            let (used, conversion) = self.convert(start, None);
            (self.base, self.position) = (start, start + used);
            if self.wide_get.read.end > 0 {
                return Some(());
            }
            if conversion == Conversion::Invalid {
                return self.invalid_sequence();
            }
            drop(self.bytes.drain(..self.position));
        } else {
            self.bytes.clear();
        }
        (self.base, self.position) = (0, 0);
        self.allocate();
        self.allocate_wide();
        if self.line_buffered || self.unbuffered {
            stdout.release();
        }
        self.oriented = true;
        self.wide_get.empty_main();
        let mut held = [0_u8; HELD_BYTES];
        let mut holding = 0;
        loop {
            let end = self.bytes.len();
            self.bytes.resize(self.size.max(end), 0);
            let read = read_from(&mut self.target, &mut self.bytes[end..]);
            let count = *read.as_ref().unwrap_or(&0);
            self.bytes.truncate(end + count);
            if count == 0 {
                match read {
                    | Ok(_) if holding == 0 => {
                        self.eof = true;
                        self.offset = None;
                    },
                    | Ok(_) => {
                        self.error = true;
                        self.errno = Some(EILSEQ);
                    },
                    | Err(error) => {
                        self.error = true;
                        self.errno = Some(from_io(&error));
                    },
                }
                return None;
            }
            self.offset = self.offset.map(|offset| offset + to_offset(count));
            self.base = self.position;
            let (used, conversion) = if holding == 0 {
                let (used, conversion) = self.convert(self.position, None);
                self.position += used;
                (used, conversion)
            } else {
                let copied = (HELD_BYTES - holding).min(count);
                held[holding..holding + copied]
                    .copy_from_slice(&self.bytes[self.position..self.position + copied]);
                holding += copied;
                let (used, conversion) = self.convert(0, Some(&held[..holding]));
                self.position += used.saturating_sub(holding - copied);
                (used, conversion)
            };
            if self.wide_get.read.end > 0 {
                return Some(());
            }
            if conversion == Conversion::Invalid {
                return self.invalid_sequence();
            }
            // The bytes read make no complete character: read again.
            if holding == 0 {
                if self.base < self.position {
                    drop(self.bytes.drain(self.base..self.position));
                    self.position = self.base;
                    continue;
                }
                holding = self.bytes.len() - self.position;
                if holding >= HELD_BYTES {
                    return self.invalid_sequence();
                }
                held[..holding].copy_from_slice(&self.bytes[self.position..]);
            } else {
                if used > 0 {
                    held.copy_within(used..holding, 0);
                    holding -= used;
                }
                if holding == HELD_BYTES {
                    return self.invalid_sequence();
                }
            }
            self.bytes.truncate(self.base);
            self.position = self.base;
        }
    }

    /// Allocates the main wide buffer (`_IO_wfile_doallocate`): one
    /// character unbuffered, else as many as the byte buffer holds bytes,
    /// or a quarter as many in a buffer `setvbuf` supplied.
    fn allocate_wide(&mut self) {
        let get = &mut self.wide_get;
        if get.capacity > 0 {
            return;
        }
        get.capacity = if self.unbuffered {
            1
        } else if self.user_buffer {
            self.size.div_ceil(4)
        } else {
            self.size
        };
        get.main.clear();
        get.main.resize(get.capacity, 0);
    }

    /// Converts the bytes from `start` of the byte buffer, or `held`
    /// instead, into the main buffer from its read end while it has room
    /// (`__libio_codecvt_in`): how many bytes it used, and why it stopped.
    fn convert(&mut self, start: usize, held: Option<&[u8]>) -> (usize, Conversion) {
        self.convert_up_to(start, self.bytes.len(), held)
    }

    /// As [`Stream::convert`], with the byte buffer's bytes ending at `end`.
    fn convert_up_to(
        &mut self,
        start: usize,
        end: usize,
        held: Option<&[u8]>,
    ) -> (usize, Conversion) {
        let from = held.unwrap_or(&self.bytes[start..end]);
        let get = &mut self.wide_get;
        let mut at = 0;
        while get.read.end < get.capacity.max(1) {
            let Some(&first) = from.get(at) else {
                return (at, Conversion::Done);
            };
            let length = super::super::super::locale::sequence_length(self.utf8, first);
            if at + length > from.len() {
                // An incomplete sequence is partial while what there is of
                // it continues a valid start.
                let valid = length > 1 && from[at + 1..].iter().all(|&byte| byte & 0xC0 == 0x80);
                return (
                    at,
                    if valid {
                        Conversion::Partial
                    } else {
                        Conversion::Invalid
                    },
                );
            }
            match super::super::super::locale::decode(self.utf8, &from[at..at + length]) {
                | Some((character, used)) => {
                    let end = get.read.end;
                    if get.main.len() <= end {
                        get.main.resize(end + 1, 0);
                    }
                    get.main[end] = character;
                    get.read.end += 1;
                    at += used;
                },
                | None => return (at, Conversion::Invalid),
            }
        }
        (at, Conversion::Done)
    }

    /// Fails a read on bytes the stream's conversion rejects.
    fn invalid_sequence(&mut self) -> Option<()> {
        self.errno = Some(EILSEQ);
        self.error = true;
        None
    }

    /// How many bytes of the byte buffer, from its base, convert to
    /// `characters` characters, or to as many as they do
    /// (`__libio_codecvt_length`): glibc counts the characters of one area
    /// against the bytes from wherever the last conversion started.
    fn converted_length(&self, characters: usize) -> usize {
        let bytes = &self.bytes[self.base..];
        let mut at = 0;
        for _ in 0..characters {
            match super::super::super::locale::decode(self.utf8, &bytes[at..]) {
                | Some((_, length)) => at += length,
                | None => break,
            }
        }
        at
    }

    /// `ftell` of a wide stream (`do_ftell_wide`), once its wide buffer
    /// exists: the file offset less, for the "C" locale's fixed width, the
    /// unread characters and the bytes not yet converted, or for UTF-8 the
    /// bytes from the byte buffer's base less those that convert to the
    /// characters read. In the backup area unread characters fail with
    /// `EINVAL`; read ones count from where they were pushed back.
    pub(super) fn tell_wide(&mut self) -> Option<i64> {
        let mut offset = 0;
        if self.wide_get.capacity > 0 {
            let get = &self.wide_get;
            let area = if get.in_backup {
                if get.read.ptr < get.read.end {
                    self.errno = Some(EINVAL);
                    return None;
                }
                let (base, end) = get.save.expect("a backup buffer");
                Area {
                    base,
                    ptr: base,
                    end,
                }
            } else {
                get.read
            };
            offset = if self.utf8 {
                let read = self.converted_length(area.ptr - area.base);
                -to_offset(self.bytes.len() - self.base - read)
            } else {
                -to_offset(area.end - area.ptr + self.bytes.len() - self.position)
            };
        }
        let at = self.file_offset()? + offset;
        if at < 0 {
            self.errno = Some(EINVAL);
            return None;
        }
        Some(at)
    }

    /// `fflush` of a wide stream (`_IO_wfile_sync`): it seeks the descriptor
    /// back over what the area being read has not given out, backup area
    /// included (in UTF-8, the bytes past those that convert to the
    /// characters before its read position), and drops it; a descriptor
    /// that cannot seek keeps everything. Whether it worked.
    pub(super) fn sync_wide(&mut self) -> bool {
        if !self.write_out() {
            return false;
        }
        let read = self.wide_get.read;
        if read.ptr != read.end {
            let delta = if self.utf8 {
                let consumed = self.converted_length(read.ptr - read.base);
                self.position = self.base + consumed;
                -to_offset(self.bytes.len() - self.base - consumed)
            } else {
                -to_offset(read.end - read.ptr)
            };
            match self.descriptor_seek(SeekFrom::Current(delta)) {
                | Ok(_) => {
                    self.wide_get.read.end = read.ptr;
                    self.bytes.truncate(self.position);
                },
                | Err(_) if self.errno == Some(ESPIPE) => {},
                | Err(_) => return false,
            }
        }
        self.offset = None;
        true
    }

    /// `fseek` of a wide stream (`_IO_seekoff_unlocked` and
    /// `_IO_wfile_seekoff`), saying whether it moved. glibc frees the backup
    /// buffer first and aborts the program for `SEEK_CUR` while the backup
    /// area is being read. A seek within the byte buffer keeps it,
    /// counting the characters before the new position as read (converted
    /// again for UTF-8); any other seeks to the block boundary below and
    /// reads up to the position, or, when glibc does not know the offset,
    /// seeks the descriptor and empties both buffers.
    pub(super) fn seek_wide(&mut self, offset: i64, whence: Whence) -> bool {
        let mut offset = offset;
        let mut whence = whence;
        if self.wide_get.save.is_some() {
            if whence == Whence::Current && self.wide_get.in_backup {
                self.aborted = true;
                return false;
            }
            self.wide_get.free_backup();
        }
        let must_be_exact = self.wide_get.read.base == self.wide_get.read.end;
        let was_writing = self.putting || self.wide_written < self.wide_log.len();
        if was_writing && !(self.write_out() && self.end_write()) {
            return false;
        }
        // Without a wide buffer, `_IO_doallocbuf` allocates both buffers,
        // but only when the byte buffer is missing too; either way both get
        // areas start empty.
        if self.wide_get.capacity == 0 {
            if self.size == 0 {
                self.allocate();
                self.allocate_wide();
            }
            self.bytes.clear();
            (self.base, self.position) = (0, 0);
            (self.putting, self.write_base) = (false, 0);
            self.wide_get.empty_main();
        }
        match whence {
            | Whence::Current => {
                if !was_writing {
                    let read = self.wide_get.read;
                    if self.utf8 {
                        let consumed = self.converted_length(read.ptr - read.base);
                        self.position = self.base + consumed;
                        self.wide_get.read.end = read.ptr;
                        offset -= to_offset(self.bytes.len() - self.base - consumed);
                    } else {
                        offset -= to_offset(read.end - read.ptr + self.bytes.len() - self.position);
                    }
                }
                let Some(cached) = self.offset else {
                    return self.dumb_seek(offset, Whence::Current);
                };
                offset += cached;
                whence = Whence::Start;
            },
            | Whence::Start => {},
            | Whence::End => match self.regular_file_size() {
                | Some(size) => {
                    offset += size;
                    whence = Whence::Start;
                },
                | None => return self.dumb_seek(offset, whence),
            },
        }
        self.wide_get.free_backup();
        // Within the byte buffer.
        if let Some(cached) = self.offset
            && self.size > 0
            && !self.wide_get.in_backup
        {
            let start = cached - to_offset(self.bytes.len());
            if offset >= start && offset < cached {
                self.base = 0;
                self.position = usize::try_from(offset - start).expect("within the buffer");
                (self.putting, self.write_base) = (false, 0);
                self.wide_get.empty_main();
                if !self.adjust_wide_data(false) {
                    return self.dumb_seek(offset, whence);
                }
                self.eof = false;
                // glibc resynchronizes the descriptor with the cached offset.
                _ = self
                    .descriptor_seek(SeekFrom::Start(cached.cast_unsigned()))
                    .ok();
                return true;
            }
        }
        if !self.readable {
            return self.dumb_seek(offset, whence);
        }
        // To the block boundary below, reading up to the position.
        let size = to_offset(self.size);
        let mut boundary = offset & !(size - 1);
        let mut delta = offset - boundary;
        if delta > size {
            (boundary, delta) = (offset, 0);
        }
        let Ok(boundary) = u64::try_from(boundary) else {
            self.errno = Some(EINVAL);
            return false;
        };
        if self.descriptor_seek(SeekFrom::Start(boundary)).is_err() {
            return false;
        }
        let wanted = usize::try_from(delta).expect("within a buffer");
        let mut count = 0;
        if wanted > 0 {
            self.bytes.clear();
            self.bytes
                .resize(if must_be_exact { wanted } else { self.size }, 0);
            let read = read_from(&mut self.target, &mut self.bytes);
            count = *read.as_ref().unwrap_or(&0);
            self.bytes.truncate(count);
            if count < wanted {
                let rest = if read.is_err() {
                    delta
                } else {
                    delta - to_offset(count)
                };
                return self.dumb_seek(rest, Whence::Current);
            }
        } else {
            self.bytes.clear();
        }
        self.base = 0;
        self.position = wanted;
        (self.putting, self.write_base) = (false, 0);
        self.wide_get.empty_main();
        if !self.adjust_wide_data(true) {
            return self.dumb_seek(offset, whence);
        }
        self.offset = Some(boundary.cast_signed() + to_offset(count));
        self.eof = false;
        true
    }

    /// `adjust_wide_data`: counts the characters between the byte buffer's
    /// base and position as read, converting them when `convert` (always
    /// for UTF-8) and otherwise only advancing the end, as glibc does for
    /// a fixed width without looking at the buffer. Whether the bytes
    /// converted.
    fn adjust_wide_data(&mut self, convert: bool) -> bool {
        if !convert && !self.utf8 {
            let end = self.position - self.base;
            let get = &mut self.wide_get;
            if get.main.len() < end {
                get.main.resize(end, 0);
            }
            get.read.end = end;
        } else {
            let (base, position) = (self.base, self.position);
            let (used, conversion) = self.convert_up_to(base, position, None);
            if conversion == Conversion::Invalid && used < position - base {
                self.error = true;
                return false;
            }
        }
        self.wide_get.read.ptr = self.wide_get.read.end;
        true
    }

    /// The `dumb` path of `_IO_wfile_seekoff`: seeks the descriptor and,
    /// when that worked, empties both buffers; whether it did.
    fn dumb_seek(&mut self, offset: i64, whence: Whence) -> bool {
        let position = match whence {
            | Whence::Start => match u64::try_from(offset) {
                | Ok(offset) => SeekFrom::Start(offset),
                | Err(_) => {
                    self.errno = Some(EINVAL);
                    return false;
                },
            },
            | Whence::Current => SeekFrom::Current(offset),
            | Whence::End => SeekFrom::End(offset),
        };
        let Ok(at) = self.descriptor_seek(position) else {
            return false;
        };
        self.eof = false;
        self.offset = i64::try_from(at).ok();
        self.bytes.clear();
        (self.base, self.position) = (0, 0);
        (self.putting, self.write_base) = (false, 0);
        self.wide_get.empty_main();
        true
    }
}
