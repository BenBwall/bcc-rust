//! The wide character stream functions: `fwide`, `fputwc`, `fputws`,
//! `fgetwc`, `fgetws`, `ungetwc`, the `wprintf` family with `swprintf`, and
//! the `wscanf` family.
//!
//! A stream has no orientation until a byte or wide character function
//! uses it (or `fwide` sets one), and keeps the first it gets; a byte
//! function on a wide-oriented stream, or a wide function on a
//! byte-oriented one, is undefined and traps. A wide stream converts as
//! the locale says (`locale.rs`) and then reads and writes bytes through
//! the stream's byte buffer, so it reaches the file, and the command line,
//! where a byte stream's bytes would; glibc's separate wide buffer, which
//! holds as many wide characters as the byte buffer holds bytes, releases
//! the same bytes at the same points for ASCII text. The `wprintf` family
//! formats with the narrow engine (`printf.rs`), and the `wscanf` family
//! reads wide characters through the narrow `scanf` engine
//! (`scanf::Input::Wide`), both over UTF-8.
//!
//! C99: §7.19.2 paragraphs 4-5, pp. 264-265; PDF pp. 276-277 (orientation);
//! §7.24.2, pp. 349-367; PDF pp. 361-379 (formatted wide I/O); §7.24.3, pp.
//! 367-372; PDF pp. 379-384 (wide character I/O).

use super::{
    super::{
        errno::{
            E2BIG,
            EILSEQ,
        },
        locale,
        scanf,
    },
    DefaultHost,
    FormatArguments,
    HostCall,
    HostReturn,
    Target,
    Written,
    integer,
    va_list_operand,
};
use crate::{
    backend::interpreter::{
        arithmetic::sext,
        memory::Memory,
        trap::{
            Fault,
            UbKind,
        },
        value::RuntimeValue,
    },
    ir::{
        Align,
        Type,
    },
};

/// The functions this module runs.
pub(in crate::backend::interpreter::host) const FUNCTIONS: &[&str] = &[
    "fwide",
    "fputwc",
    "putwc",
    "putwchar",
    "fputws",
    "fgetwc",
    "getwc",
    "getwchar",
    "fgetws",
    "ungetwc",
    "wprintf",
    "fwprintf",
    "vwprintf",
    "vfwprintf",
    "swprintf",
    "vswprintf",
    "wscanf",
    "fwscanf",
    "swscanf",
    "__isoc99_wscanf",
    "__isoc99_fwscanf",
    "__isoc99_swscanf",
    "__isoc23_wscanf",
    "__isoc23_fwscanf",
    "__isoc23_swscanf",
];

/// `WEOF`, as the 32-bit `wint_t` holds it.
const WEOF: u128 = 0xFFFF_FFFF;

impl DefaultHost<'_> {
    /// Runs a wide character stream function.
    pub(super) fn run_wide(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<HostReturn, Fault> {
        let args = call.args;
        let arg = |index: usize| args.get(index).copied().unwrap_or(RuntimeValue::Poison);
        let character =
            |value: RuntimeValue| -> Result<u32, Fault> { Ok(truncate_u32(integer(value)?)) };
        let value = match call.name {
            | "fwide" => {
                // glibc's `fwide`: zero asks, and an orientation once set
                // stays.
                let stream = self.stream(arg(0), memory)?;
                let mode = sext(Type::I32, integer(arg(1))?);
                if mode != 0 && !self.streams.entries[stream].oriented {
                    if mode > 0 {
                        self.orient_wide(stream)?;
                    } else {
                        self.streams.entries[stream].oriented = true;
                    }
                }
                let entry = &self.streams.entries[stream];
                let orientation: i32 = match (entry.oriented, entry.wide) {
                    | (false, _) => 0,
                    | (true, true) => 1,
                    | (true, false) => -1,
                };
                RuntimeValue::int(Type::I32, u128::from(orientation.cast_unsigned()))
            },
            | "fputwc" | "putwc" | "putwchar" => {
                let stream = if call.name == "putwchar" {
                    self.standard(1)?
                } else {
                    self.stream(arg(1), memory)?
                };
                let wide = character(arg(0))?;
                if self.put_wide(stream, &[wide], Written::Byte)? {
                    RuntimeValue::Int(u128::from(wide))
                } else {
                    RuntimeValue::Int(WEOF)
                }
            },
            | "fputws" => {
                let stream = self.stream(arg(1), memory)?;
                let mut characters = super::super::ArenaVec::new_in(self.allocator);
                self.wide_characters(arg(0), &mut characters, memory)?;
                // glibc returns 1 when it wrote them all.
                RuntimeValue::Int(if self.put_wide(stream, &characters, Written::Block)? {
                    1
                } else {
                    WEOF
                })
            },
            | "fgetwc" | "getwc" | "getwchar" => {
                let stream = if call.name == "getwchar" {
                    self.standard(0)?
                } else {
                    self.stream(arg(0), memory)?
                };
                self.orient_wide(stream)?;
                RuntimeValue::Int(self.read_wide(stream).map_or(WEOF, u128::from))
            },
            | "fgetws" => {
                let stream = self.stream(arg(2), memory)?;
                let limit = sext(Type::I32, integer(arg(1))?);
                if limit <= 0 {
                    return Ok(HostReturn::Value(Some(RuntimeValue::NULL)));
                }
                self.orient_wide(stream)?;
                let wide = self.wide_type();
                let earlier_error = std::mem::take(&mut self.streams.entries[stream].error);
                let mut count = 0_i128;
                while count + 1 < limit {
                    let Some(next) = self.read_wide(stream) else {
                        break;
                    };
                    store_wide(memory, arg(0), count, wide, next)?;
                    count += 1;
                    if next == u32::from(b'\n') {
                        break;
                    }
                }
                let entry = &mut self.streams.entries[stream];
                let failed = entry.error;
                entry.error |= earlier_error;
                if (count == 0 && limit > 1) || failed {
                    RuntimeValue::NULL
                } else {
                    store_wide(memory, arg(0), count, wide, 0)?;
                    arg(0)
                }
            },
            | "ungetwc" => {
                let stream = self.stream(arg(1), memory)?;
                self.orient_wide(stream)?;
                // glibc keeps the wide character itself in the wide pushback
                // area, unconverted.
                let wide = integer(arg(0))? & 0xFFFF_FFFF;
                if wide == WEOF {
                    RuntimeValue::Int(WEOF)
                } else {
                    let entry = &mut self.streams.entries[stream];
                    entry.push_back_wide(truncate_u32(wide));
                    entry.eof = false;
                    RuntimeValue::Int(wide)
                }
            },
            | "wprintf" | "fwprintf" | "vwprintf" | "vfwprintf" => {
                let (stream, format) = match call.name {
                    | "wprintf" | "vwprintf" => (self.standard(1)?, 0),
                    | _ => (self.stream(arg(0), memory)?, 1),
                };
                self.orient_wide(stream)?;
                let mut values = match va_list_operand(call.name) {
                    | Some(list) =>
                        FormatArguments::from_va_list(args[list], call.va_list, memory)?,
                    | None =>
                        FormatArguments::Values(args.get(format + 1..).unwrap_or_default().iter()),
                };
                let before = self.output.len();
                if let Err(fault) = self.wide_printf(arg(format), &mut values, memory) {
                    self.output.truncate(before);
                    return Err(fault);
                }
                let count = characters_in(&self.output[before..]);
                self.convert_for_stream(stream, before);
                if self.move_output(stream, before, Written::Formatted) {
                    RuntimeValue::Int(count as u128)
                } else {
                    RuntimeValue::Int(u128::from(u32::MAX))
                }
            },
            | "swprintf" | "vswprintf" => {
                let limit = integer(arg(1))?;
                // glibc fails a call with no room before it formats
                // anything, leaving `errno` alone.
                if limit == 0 {
                    return Ok(HostReturn::Value(Some(RuntimeValue::int(
                        Type::I32,
                        u128::from(u32::MAX),
                    ))));
                }
                let mut values = match va_list_operand(call.name) {
                    | Some(list) =>
                        FormatArguments::from_va_list(args[list], call.va_list, memory)?,
                    | None => FormatArguments::Values(args.get(3..).unwrap_or_default().iter()),
                };
                let before = self.output.len();
                let formatted = self.wide_printf(arg(2), &mut values, memory);
                let result =
                    formatted.and_then(|()| self.store_wide_output(before, arg(0), limit, memory));
                self.output.truncate(before);
                match result? {
                    | Stored::All(count) => RuntimeValue::Int(count as u128),
                    // glibc fails when the output and its null do not fit,
                    // after storing what fits; with no room at all it fails
                    // at once, leaving `errno` alone.
                    | Stored::Truncated => {
                        self.set_errno(memory, E2BIG)?;
                        RuntimeValue::Int(u128::from(u32::MAX))
                    },
                    | Stored::NoRoom => RuntimeValue::Int(u128::from(u32::MAX)),
                }
            },
            | name => return self.wide_scanf(call, name, memory),
        };
        Ok(HostReturn::Value(call.result.map(|ty| match value {
            | RuntimeValue::Int(bits) if ty.is_int() => RuntimeValue::int(ty, bits),
            | value => value,
        })))
    }

    /// The `wscanf` family: reads wide characters from a wide string or a
    /// wide stream through the `scanf` engine.
    fn wide_scanf(
        &mut self,
        call: HostCall<'_>,
        name: &str,
        memory: &mut Memory<'_>,
    ) -> Result<HostReturn, Fault> {
        let arg = |index: usize| {
            call.args
                .get(index)
                .copied()
                .unwrap_or(RuntimeValue::Poison)
        };
        let base = name
            .trim_start_matches("__isoc99_")
            .trim_start_matches("__isoc23_");
        let (input, fixed) = match base {
            | "wscanf" => (scanf::Input::wide_stream(self.standard(0)?), 1),
            | "fwscanf" => {
                let stream = self.stream(arg(0), memory)?;
                (scanf::Input::wide_stream(stream), 2)
            },
            | _ => (scanf::Input::wide_string(arg(0)), 2),
        };
        if let scanf::Input::Wide {
            source: scanf::WideSource::Stream(stream),
            ..
        } = input
        {
            self.orient_wide(stream)?;
        }
        let dialect = if name.starts_with("__isoc23_") {
            scanf::Dialect::C23
        } else {
            scanf::Dialect::C99
        };
        let mut values = FormatArguments::Values(call.args.get(fixed..).unwrap_or_default().iter());
        let count = self.scanf(input, arg(fixed - 1), &mut values, memory, dialect)?;
        Ok(HostReturn::Value(Some(RuntimeValue::int(
            Type::I32,
            u128::from(count.cast_unsigned()),
        ))))
    }

    /// Orients `stream` for wide characters, as glibc's `_IO_fwide (fp, 1)`
    /// does before a wide function reads or writes; one already
    /// byte-oriented is undefined.
    /// C99: §7.19.2 paragraph 5, p. 265; PDF p. 277.
    pub(in crate::backend::interpreter::host) fn orient_wide(
        &mut self,
        stream: usize,
    ) -> Result<(), Fault> {
        let utf8 = self.locale.utf8();
        let entry = &mut self.streams.entries[stream];
        if entry.oriented && !entry.wide {
            return Err(UbKind::StreamOrientation.into());
        }
        if !entry.wide {
            // The conversion is the locale's now, and stays the stream's.
            (entry.oriented, entry.wide, entry.utf8) = (true, true, utf8);
            match entry.target {
                | Target::Output => self.console.output.set_wide(utf8),
                | Target::Errors => self.console.error.set_wide(utf8),
                | _ => {},
            }
        }
        Ok(())
    }

    /// Whether `stream` is wide-oriented, where a byte function is
    /// undefined.
    pub(in crate::backend::interpreter::host) fn is_wide(&self, stream: usize) -> bool {
        self.streams.entries[stream].wide
    }

    /// Writes `characters` to `stream` as the locale converts them for a
    /// stream, in the way `kind` writes; whether it worked.
    fn put_wide(
        &mut self,
        stream: usize,
        characters: &[u32],
        kind: Written,
    ) -> Result<bool, Fault> {
        self.orient_wide(stream)?;
        let before = self.output.len();
        let utf8 = self.streams.entries[stream].utf8;
        for &character in characters {
            locale::encode_for_stream(utf8, character, &mut self.output);
        }
        Ok(self.move_output(stream, before, kind))
    }

    /// Replaces the UTF-8 the wide `printf` engine appended after `before`
    /// with what `stream` writes for it with the conversion its orientation
    /// fixed: the same for "C.UTF-8", `?` for each character beyond ASCII
    /// for "C".
    fn convert_for_stream(&mut self, stream: usize, before: usize) {
        if self.streams.entries[stream].utf8 {
            return;
        }
        self.text.clear();
        self.text.extend_from_slice(&self.output[before..]);
        self.output.truncate(before);
        for &byte in &self.text {
            match byte {
                | 0..0x80 => self.output.push(byte),
                | 0xC0.. => self.output.push(b'?'),
                | _ => {},
            }
        }
    }

    /// Reads one wide character from `stream`, converting its multibyte
    /// form: `None` at the end of the file, on an error, or, setting the
    /// error indicator and `EILSEQ`, for bytes the locale cannot convert.
    pub(in crate::backend::interpreter::host) fn read_wide(
        &mut self,
        stream: usize,
    ) -> Option<u32> {
        let entry = &mut self.streams.entries[stream];
        if let Some(character) = entry.pop_wide_backup() {
            return Some(character);
        }
        let utf8 = entry.utf8;
        let first = self.read(stream)?;
        let length = locale::sequence_length(utf8, first);
        let mut sequence = [first, 0, 0, 0];
        for slot in sequence.iter_mut().take(length).skip(1) {
            match self.read(stream) {
                | Some(byte) => *slot = byte,
                | None => break,
            }
        }
        match locale::decode(utf8, &sequence[..length]) {
            | Some((character, _)) => Some(character),
            | None => {
                let entry = &mut self.streams.entries[stream];
                entry.error = true;
                entry.errno = Some(EILSEQ);
                None
            },
        }
    }

    /// The wide characters of the null-terminated wide string at `string`.
    fn wide_characters(
        &self,
        string: RuntimeValue,
        out: &mut super::super::ArenaVec<'_, u32>,
        memory: &Memory<'_>,
    ) -> Result<(), Fault> {
        let wide = self.wide_type();
        let size = u64::from(wide.bytes());
        let align = Align::from_bytes(size).expect("a power of two");
        for index in 0.. {
            let at = memory.ptr_add(string, RuntimeValue::Int(u128::from(index * size)), true);
            let character = truncate_u32(integer(memory.load(wide, at, align)?)?);
            if character == 0 {
                return Ok(());
            }
            out.push(character);
        }
        Ok(())
    }

    /// Stores the UTF-8 formatted after `before` as wide characters at
    /// `destination`, which holds `limit` of them: all of them and a null,
    /// or when they do not fit as many as do and a null, or nothing in no
    /// room.
    /// C99: §7.24.2.3 paragraph 3, p. 362; PDF p. 374.
    fn store_wide_output(
        &self,
        before: usize,
        destination: RuntimeValue,
        limit: u128,
        memory: &mut Memory<'_>,
    ) -> Result<Stored, Fault> {
        let wide = self.wide_type();
        let text = &self.output[before..];
        let count = characters_in(text);
        let fits = (count as u128) < limit;
        let stored = if fits {
            count
        } else {
            usize::try_from(limit.saturating_sub(1)).unwrap_or(usize::MAX)
        };
        if limit == 0 {
            return Ok(Stored::NoRoom);
        }
        let mut index = 0;
        let mut position = 0;
        while index < stored {
            let (character, length) =
                locale::decode(true, &text[position..]).unwrap_or((u32::from(b'?'), 1));
            store_wide(memory, destination, index as i128, wide, character)?;
            position += length;
            index += 1;
        }
        store_wide(memory, destination, stored as i128, wide, 0)?;
        Ok(if fits {
            Stored::All(count)
        } else {
            Stored::Truncated
        })
    }

    /// Pushes the UTF-8 characters of `bytes`, in reading order, back onto
    /// the wide stream `stream`, as the `wscanf` family's `ungetwc` of what
    /// it read ahead does.
    pub(in crate::backend::interpreter::host) fn push_back_wide(
        &mut self,
        stream: usize,
        bytes: &[u8],
    ) {
        let entry = &mut self.streams.entries[stream];
        // The last character read goes back first.
        let mut end = bytes.len();
        while end > 0 {
            let mut start = end - 1;
            while start > 0 && end - start < 4 && bytes[start] & 0xC0 == 0x80 {
                start -= 1;
            }
            match locale::decode(true, &bytes[start..end]) {
                | Some((character, length)) if start + length == end =>
                    entry.push_back_wide(character),
                | _ => break,
            }
            end = start;
        }
        entry.eof = false;
    }
}

/// What `swprintf` stored.
enum Stored {
    /// Everything and a null, this many characters.
    All(usize),
    /// What fits and a null.
    Truncated,
    /// Nothing: the destination holds no character.
    NoRoom,
}

/// How many characters the UTF-8 `text` holds.
fn characters_in(text: &[u8]) -> usize {
    text.iter().filter(|&&byte| byte & 0xC0 != 0x80).count()
}

/// Stores the wide character `character` as element `index` of the
/// `wchar_t` array at `array`.
fn store_wide(
    memory: &mut Memory<'_>,
    array: RuntimeValue,
    index: i128,
    wide: Type,
    character: u32,
) -> Result<(), Fault> {
    let size = u128::from(wide.bytes());
    let at = memory.ptr_add(array, RuntimeValue::Int(index.unsigned_abs() * size), true);
    memory.store(
        wide,
        RuntimeValue::int(wide, u128::from(character)),
        at,
        Align::from_bytes(u64::from(wide.bytes())).expect("a power of two"),
    )
}

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
const fn truncate_u32(bits: u128) -> u32 {
    bits as u32
}
