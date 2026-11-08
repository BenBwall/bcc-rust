//! The rows of the line table as `.loc` directives, and its files as
//! `.file` (DWARF 5 §6.2), numbered as Clang numbers them for DWARF 5: the
//! compilation unit's file is 0, and each other file takes the next number
//! the first time a row or an entry names it.

use std::fmt::{
    self,
    Write,
};

use crate::{
    ir::{
        DebugFile,
        DebugInfo,
        DebugScope,
        Entity,
        SourceLocation,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// What the listing's `.file` and `.loc` directives have said so far.
pub(super) struct Lines<'d> {
    /// Each file's number, by file index, once it has one.
    numbers:   ArenaVec<'d, Option<u32>>,
    next:      u32,
    root_name: &'d str,
    directory: &'d str,
    /// The last row's file, line and column.
    row:       Option<(u32, u32, u32)>,
    /// The last row's line other than 0.
    last_line: u32,
    /// The `is_stmt` the assembler keeps from one `.loc` to the next.
    is_stmt:   bool,
    /// Whether the next row is the first of the function's body.
    prologue:  bool,
}

impl<'d> Lines<'d> {
    pub(super) fn new(
        debug: &DebugInfo<'_>,
        root: DebugFile,
        directory: &'d str,
        arena: &'d Bump,
    ) -> Self {
        let mut numbers = ArenaVec::new_in(arena);
        numbers.resize(debug.files().count(), None);
        numbers[root.index()] = Some(0);
        let root_name = debug.file_name(root).unwrap_or("");
        Self {
            numbers,
            next: 1,
            root_name: arena.alloc_str(root_name),
            directory,
            row: None,
            last_line: 0,
            is_stmt: true,
            prologue: false,
        }
    }

    /// The compilation unit's file name, as the preprocessor knows it.
    pub(super) const fn root_name(&self) -> &'d str {
        self.root_name
    }

    /// The compilation directory.
    pub(super) const fn directory(&self) -> &'d str {
        self.directory
    }

    /// `.file 0`, the compilation unit's file in its directory.
    pub(super) fn root(&self, out: &mut dyn Write) -> fmt::Result {
        writeln!(
            out,
            "\t.file\t0 {} {}",
            Quoted(self.directory),
            Quoted(self.root_name)
        )
    }

    /// The number of `file`, writing its `.file` the first time. A name with
    /// a directory part is split there, as Clang splits it; any other is in
    /// the compilation directory. A file whose last component is the root
    /// file's name is the root file to the assembler, which tells files of
    /// one name apart only by their MD5 checksums, so it is written as the
    /// root file.
    pub(super) fn number(
        &mut self,
        out: &mut dyn Write,
        debug: &DebugInfo<'_>,
        file: DebugFile,
    ) -> Result<u32, fmt::Error> {
        if let Some(number) = self.numbers.get(file.index()).copied().flatten() {
            return Ok(number);
        }
        let name = debug.file_name(file).unwrap_or("");
        let (directory, base) = match name.rfind('/') {
            | Some(0) => ("/", &name[1..]),
            | Some(at) => (&name[..at], &name[at + 1..]),
            | None => (self.directory, name),
        };
        if base == self.root_name || base.is_empty() {
            self.numbers[file.index()] = Some(0);
            return Ok(0);
        }
        let number = self.next;
        self.next += 1;
        self.numbers[file.index()] = Some(number);
        writeln!(
            out,
            "\t.file\t{number} {} {}",
            Quoted(directory),
            Quoted(base)
        )?;
        Ok(number)
    }

    /// The row of a function's prologue: the line its body opens on, column
    /// 0, a statement, as GCC and Clang write it.
    pub(super) fn prologue(
        &mut self,
        out: &mut dyn Write,
        debug: &DebugInfo<'_>,
        file: DebugFile,
        line: u32,
    ) -> fmt::Result {
        let file = self.number(out, debug, file)?;
        self.prologue = true;
        self.write(out, (file, line, 0), true, false)
    }

    /// Line 0 for the code of a function without a subprogram, so that it is
    /// not taken for the previous function's last line.
    pub(super) fn unknown(&mut self, out: &mut dyn Write) -> fmt::Result {
        self.prologue = false;
        match self.row {
            | Some((file, line, _)) if line != 0 => self.write(out, (file, 0, 0), false, false),
            | _ => Ok(()),
        }
    }

    /// The row of code at `location`, if it has a new position. Code with no
    /// position keeps the row before it, except at the top of a block, where
    /// it gets line 0.
    pub(super) fn locate(
        &mut self,
        out: &mut dyn Write,
        debug: &DebugInfo<'_>,
        location: Option<SourceLocation>,
        block_start: bool,
    ) -> fmt::Result {
        let Some(location) = location else {
            return match self.row {
                | Some((file, line, _)) if block_start && line != 0 =>
                    self.write(out, (file, 0, 0), false, false),
                | _ => Ok(()),
            };
        };
        let file = self.number(out, debug, location.file)?;
        let row = (file, location.line, location.column);
        let prologue_end = self.prologue && location.line != 0;
        if self.row == Some(row) && !prologue_end {
            return Ok(());
        }
        // A new line starts a statement; so does the body.
        let statement = location.line != 0 && (location.line != self.last_line || prologue_end);
        self.prologue &= !prologue_end;
        self.write(out, row, statement, prologue_end)
    }

    fn write(
        &mut self,
        out: &mut dyn Write,
        (file, line, column): (u32, u32, u32),
        statement: bool,
        prologue_end: bool,
    ) -> fmt::Result {
        write!(out, "\t.loc\t{file} {line} {column}")?;
        if prologue_end {
            out.write_str(" prologue_end")?;
        }
        if statement != self.is_stmt {
            write!(out, " is_stmt {}", u8::from(statement))?;
            self.is_stmt = statement;
        }
        out.write_char('\n')?;
        self.row = Some((file, line, column));
        if line != 0 {
            self.last_line = line;
        }
        Ok(())
    }
}

/// The scope a location's code runs in within its function: its own, or for
/// an inlined copy, the scope of the outermost call it was copied for.
pub(super) fn outer_scope(debug: &DebugInfo<'_>, mut location: SourceLocation) -> DebugScope {
    while let Some(outer) = location.inlined_at.and_then(|at| debug.location_data(at)) {
        location = outer;
    }
    location.scope
}

/// A string as an assembler operand: in quotes, with each quote, backslash
/// and byte outside printable ASCII as an octal escape.
pub(super) struct Quoted<'a>(pub(super) &'a str);

impl fmt::Display for Quoted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('"')?;
        for byte in self.0.bytes() {
            if matches!(byte, b' '..=b'~') && !matches!(byte, b'"' | b'\\') {
                f.write_char(char::from(byte))?;
            } else {
                write!(f, "\\{byte:03o}")?;
            }
        }
        f.write_char('"')
    }
}
