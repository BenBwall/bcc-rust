//! DWARF 5 debug information in the native listing, for `-g` on ELF.
//!
//! The emitter calls a [`DebugWriter`] as it writes each function. Before an
//! instruction whose IR instruction has a new source position it writes a
//! `.loc` (`lines.rs`), which the assembler, or the direct encoder's line
//! table (`object/line_table.rs`), turns into a row of `.debug_line`; each
//! file gets a `.file` the first time a row or an entry names it. Around
//! the code it records labels: where each function begins and ends, and
//! where the scope of the code changes, which is what bounds a lexical
//! block. Once every function is written, [`DebugWriter::finish`] writes
//! the debugging information entries (`description.rs`) and their
//! abbreviations (`entries.rs`) as data directives in `.debug_info` and
//! `.debug_abbrev`, with the range lists of `.debug_rnglists`.
//!
//! For `int add(int a, int b) { return a + b; }` at `-O0` the listing reads
//! `.Ldebug_begin0:`, `.loc 0 1 0` before the prologue, `.loc 0 1 30
//! prologue_end` before the load of `a`, `.Ldebug_end0:` after the code,
//! and a `DW_TAG_subprogram` whose `DW_AT_frame_base` is the canonical frame
//! address, with a `DW_TAG_formal_parameter` for `a` at `DW_OP_fbreg -20`,
//! its stack slot's offset from that address.
//!
//! Read [`DebugWriter`], then `lines.rs`, `description.rs` and `entries.rs`.
//!
//! Rows follow GCC's and Clang's conventions: the prologue is at the line
//! where the body opens, `prologue_end` marks the first instruction of the
//! first statement, a row is a statement (`is_stmt`) when its line differs
//! from the row before, and code at the top of a block with no position of
//! its own is line 0 rather than the previous block's line, as LLVM writes
//! it. The canonical frame address is what the `.cfi_*` directives already
//! define, so `DW_OP_call_frame_cfa` stays right however `%rsp` moves.
//!
//! The back end is not a C translation phase. The line table follows the
//! logical positions of C99 §6.10.4, pp. 154-155; PDF pp. 166-167, and a
//! block's variables its scope, C99 §6.2.1 paragraph 4, pp. 29-30; PDF pp.
//! 41-42.

// Rows and files
mod lines;

// Entries
mod description;
mod entries;

use std::fmt::{
    self,
    Write,
};

use self::lines::Lines;
use super::{
    frame::Frame,
    mir::InstSource,
};
use crate::{
    ir::{
        DebugScope,
        DebugVariable,
        EntryConvention,
        FuncId,
        Module,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The debug information of one listing, as the emitter writes it.
pub(crate) struct DebugWriter<'d, 'm, 'ir> {
    module:    &'m Module<'ir>,
    arena:     &'d Bump,
    lines:     Lines<'d>,
    /// Each described function, in the order the listing holds them.
    functions: ArenaVec<'d, Described<'d>>,
    /// The next label that bounds a run of one scope's code.
    runs:      u32,
}

/// A function the listing describes: one with a subprogram.
struct Described<'d> {
    function:    FuncId,
    /// Its labels' number: `.Ldebug_begin<number>` and
    /// `.Ldebug_end<number>`.
    number:      u32,
    /// Whether its code is in `.text`, so one range of it can cover the
    /// compilation unit.
    in_text:     bool,
    /// Whether the canonical frame address is defined throughout its code;
    /// an interrupt handler has no caller frame and a naked function no
    /// frame of its own, so neither places a variable.
    frame_base:  bool,
    /// Where each placed variable is, from the canonical frame address.
    variables:   ArenaVec<'d, (DebugVariable, i64)>,
    /// The runs of code in one scope, each up to the next one's start or
    /// the function's end.
    runs:        ArenaVec<'d, (DebugScope, Mark)>,
    /// The scope of the code being written.
    open_scope:  Option<DebugScope>,
}

/// A label the writer puts in a function's code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mark {
    Begin(u32),
    Run(u32),
    End(u32),
}

impl fmt::Display for Mark {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            | Self::Begin(number) => write!(f, ".Ldebug_begin{number}"),
            | Self::Run(number) => write!(f, ".Ldebug_run{number}"),
            | Self::End(number) => write!(f, ".Ldebug_end{number}"),
        }
    }
}

impl<'d, 'm, 'ir> DebugWriter<'d, 'm, 'ir> {
    /// The writer of `module`'s debug information, or `None` if the module
    /// has no compilation unit to describe. `directory` is where relative
    /// file names are, which `DW_AT_comp_dir` records.
    pub(crate) fn new(module: &'m Module<'ir>, directory: &'d str, arena: &'d Bump) -> Option<Self> {
        let unit = module.debug_info().unit()?;
        Some(Self {
            module,
            arena,
            lines: Lines::new(module.debug_info(), unit.file, directory, arena),
            functions: ArenaVec::new_in(arena),
            runs: 0,
        })
    }

    /// The root file's `.file 0`, before the listing's first `.loc`.
    pub(crate) fn header(&mut self, out: &mut dyn Write) -> fmt::Result {
        self.lines.root(out)
    }

    /// Starts `function`, right after its label: its begin label, and line 0
    /// if it has no subprogram. `in_text` says whether its code is in
    /// `.text`.
    pub(crate) fn begin_function(
        &mut self,
        out: &mut dyn Write,
        function: FuncId,
        in_text: bool,
    ) -> fmt::Result {
        let debug = self.module.debug_info();
        let Some(scope) = debug.subprogram_of(function) else {
            return self.lines.unknown(out);
        };
        let Some(crate::ir::Scope::Subprogram(subprogram)) = debug.scope(scope).copied() else {
            return self.lines.unknown(out);
        };
        let number = u32::try_from(self.functions.len()).map_err(|_| fmt::Error)?;
        let entry = self.module.function(function).entry;
        self.functions.push(Described {
            function,
            number,
            in_text,
            frame_base: !matches!(entry, EntryConvention::Interrupt | EntryConvention::Naked),
            variables: ArenaVec::new_in(self.arena),
            runs: ArenaVec::new_in(self.arena),
            open_scope: Some(scope),
        });
        writeln!(out, "{}:", Mark::Begin(number))?;
        self.functions
            .last_mut()
            .expect("pushed above")
            .runs
            .push((scope, Mark::Begin(number)));
        self.lines.begin(subprogram.file, subprogram.scope_line);
        Ok(())
    }

    /// Before the function's prologue: its row, at the line its body opens
    /// on, unless the prologue is `empty`. A function without one starts at
    /// its first statement if its first instruction has a position, as with
    /// LLVM.
    pub(crate) fn prologue(&mut self, out: &mut dyn Write, empty: bool) -> fmt::Result {
        if empty {
            return Ok(());
        }
        self.lines.prologue(out, self.module.debug_info())
    }

    /// Before the code of `function` that `source` asked for: its row, if
    /// its position is new, and a label where its scope changes.
    /// `block_start` says whether it opens a block other than the entry.
    pub(crate) fn instruction(
        &mut self,
        out: &mut dyn Write,
        function: FuncId,
        source: Option<InstSource>,
        block_start: bool,
    ) -> fmt::Result {
        let module = self.module;
        let debug = module.debug_info();
        let location = match source {
            | Some(InstSource::Ir(inst)) => module
                .function(function)
                .body
                .as_ref()
                .and_then(|body| body.inst_location(inst))
                .filter(|&location| debug.root_function(location) == Some(function))
                .and_then(|location| debug.location_data(location)),
            | _ => None,
        };
        let Some(described) = self
            .functions
            .last_mut()
            .filter(|described| described.function == function)
        else {
            return Ok(());
        };
        if let Some(location) = location {
            let scope = lines::outer_scope(debug, location);
            if described.open_scope != Some(scope) {
                let mark = Mark::Run(self.runs);
                self.runs += 1;
                writeln!(out, "{mark}:")?;
                described.runs.push((scope, mark));
                described.open_scope = Some(scope);
            }
        }
        self.lines.locate(out, debug, location, block_start)
    }

    /// Ends `function`, after its code: its end label, and where `frame`
    /// keeps the variables the body places in stack slots.
    pub(crate) fn end_function(
        &mut self,
        out: &mut dyn Write,
        function: FuncId,
        frame: &Frame<'_>,
    ) -> fmt::Result {
        let Some(described) = self
            .functions
            .last_mut()
            .filter(|described| described.function == function)
        else {
            return Ok(());
        };
        writeln!(out, "{}:", Mark::End(described.number))?;
        if described.frame_base
            && let Some(body) = &self.module.function(function).body
        {
            description::place_variables(self.module, body, frame, &mut described.variables);
        }
        Ok(())
    }

    /// Writes the entries, their abbreviations and range lists, and the
    /// start of `.debug_line`, after every function and object.
    pub(crate) fn finish(&mut self, out: &mut dyn Write) -> fmt::Result {
        description::write(self, out)
    }
}
