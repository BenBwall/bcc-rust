//! Where a module's instructions came from in the C source, for the debug
//! information `-g` asks of a back end.
//!
//! A [`DebugInfo`] holds the compilation unit's facts, the module's source
//! files, the scopes (one subprogram per function lowered with `-g`) and the
//! interned [`SourceLocation`]s. A body maps an instruction to a [`DebugLoc`]
//! in a side table beside its records ([`Body::inst_location`]), which stays
//! empty without `-g`, so a compilation without it pays for neither the table
//! nor a larger instruction record. Nothing that decides what code to generate
//! reads a location, so `-g` does not change the machine code.
//!
//! A location is the logical position `#line` and `#include` give the token,
//! and the place of use for a macro expansion, as GCC's and Clang's tables
//! report it. A location inlined from another function keeps that function's
//! scope and names the call it was copied for ([`DebugInfo::inlined`]), which
//! is what lets `addr2line -i` and a debugger show the inlined frame. Each
//! location an `at` names has a smaller id than the location that names it.
//!
//! The textual form prints these tables and each instruction's location only
//! when asked ([`Module::display_located`]).
//!
//! C99: the logical positions come from §6.10.4, pp. 154-155; PDF pp. 166-167.
//!
//! [`Body::inst_location`]: super::Body::inst_location
//! [`Module::display_located`]: super::Module::display_located

use rustc_hash::FxBuildHasher;

use super::entities::{
    DebugFile,
    DebugLoc,
    DebugScope,
    FuncId,
    Table,
};
use crate::util::bump::{
    ArenaMap,
    Bump,
};

/// A module's debug information; see the module docs.
#[derive(Debug)]
pub(crate) struct DebugInfo<'ir> {
    /// The compilation unit, without which a back end emits no debug
    /// information.
    unit:         Option<DebugUnit>,
    /// Each file's name, as the preprocessor knows it.
    files:        Table<'ir, DebugFile, &'ir str>,
    file_ids:     ArenaMap<'ir, &'ir str, DebugFile>,
    scopes:       Table<'ir, DebugScope, Subprogram<'ir>>,
    /// The scope of each function that has one.
    subprograms:  ArenaMap<'ir, FuncId, DebugScope>,
    locations:    Table<'ir, DebugLoc, SourceLocation>,
    location_ids: ArenaMap<'ir, SourceLocation, DebugLoc>,
}

/// The facts of a compilation unit that a back end writes as debug
/// information.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DebugUnit {
    pub(crate) file:          DebugFile,
    pub(crate) language:      DebugLanguage,
    /// How much the unit describes.
    pub(crate) level:         DebugLevel,
    /// The DWARF version `-gdwarf-N` asked for; `None` leaves the default,
    /// which is DWARF 5, or CodeView for an MSVC target.
    pub(crate) dwarf_version: Option<u8>,
    /// An optimizer ran, so a debugger should expect moved and merged code.
    pub(crate) optimized:     bool,
}

/// How much debug information `-g` asks for, as the levels of GCC and Clang.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum DebugLevel {
    /// `-g1`: line tables and functions.
    Lines,
    /// `-g` or `-g2`.
    Full,
    /// `-g3`, which would add the macros; bcc writes what `-g2` does.
    Macros,
}

impl DebugLevel {
    /// The level's number in `-gN`.
    pub(crate) const fn number(self) -> u8 {
        match self {
            | Self::Lines => 1,
            | Self::Full => 2,
            | Self::Macros => 3,
        }
    }

    /// The level `-gN` names; 0 and anything above 3 have none.
    pub(crate) const fn from_number(number: u8) -> Option<Self> {
        match number {
            | 1 => Some(Self::Lines),
            | 2 => Some(Self::Full),
            | 3 => Some(Self::Macros),
            | _ => None,
        }
    }
}

/// The language a unit's `DW_AT_language` names: the codes GCC 14.2 and
/// Clang 23 write, which have no separate C17 or C23 value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DebugLanguage {
    C89,
    C99,
    C11,
}

impl DebugLanguage {
    /// The keyword of the IR's text form.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            | Self::C89 => "c89",
            | Self::C99 => "c99",
            | Self::C11 => "c11",
        }
    }

    /// The language `keyword` names in the IR's text form.
    pub(crate) fn from_keyword(keyword: &str) -> Option<Self> {
        match keyword {
            | "c89" => Some(Self::C89),
            | "c99" => Some(Self::C99),
            | "c11" => Some(Self::C11),
            | _ => None,
        }
    }
}

/// A function as debug information describes it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Subprogram<'ir> {
    pub(crate) function:   FuncId,
    /// The name the source gives it, which `asm` labels and internal copies
    /// may make differ from the symbol.
    pub(crate) name:       &'ir str,
    pub(crate) file:       DebugFile,
    /// The line of its name.
    pub(crate) line:       u32,
    /// The line its body opens on, where its prologue is reported.
    pub(crate) scope_line: u32,
}

/// One instruction's source position; see the module docs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SourceLocation {
    pub(crate) file:       DebugFile,
    pub(crate) line:       u32,
    /// The 1-based column, 0 for none (`-gno-column-info`).
    pub(crate) column:     u32,
    pub(crate) scope:      DebugScope,
    /// The call this location's instruction was inlined for.
    pub(crate) inlined_at: Option<DebugLoc>,
}

impl<'ir> DebugInfo<'ir> {
    /// Tables that describe nothing, so back ends emit no debug information.
    pub(crate) fn new_in(arena: &'ir Bump) -> Self {
        Self {
            unit:         None,
            files:        Table::new_in(arena),
            file_ids:     ArenaMap::with_hasher_in(FxBuildHasher, arena),
            scopes:       Table::new_in(arena),
            subprograms:  ArenaMap::with_hasher_in(FxBuildHasher, arena),
            locations:    Table::new_in(arena),
            location_ids: ArenaMap::with_hasher_in(FxBuildHasher, arena),
        }
    }

    pub(crate) const fn unit(&self) -> Option<DebugUnit> {
        self.unit
    }

    pub(crate) const fn set_unit(&mut self, unit: DebugUnit) {
        self.unit = Some(unit);
    }

    /// The file named `name`, added the first time it is seen. `arena` is
    /// the module's.
    pub(crate) fn file(&mut self, arena: &'ir Bump, name: &str) -> DebugFile {
        if let Some(&file) = self.file_ids.get(name) {
            return file;
        }
        let name = &*arena.alloc_str(name);
        let file = self.files.push(name);
        _ = self.file_ids.insert(name, file);
        file
    }

    /// Every file with its name, in the order they were added.
    pub(crate) fn files(&self) -> impl Iterator<Item = (DebugFile, &'ir str)> + '_ {
        self.files.iter().map(|(file, &name)| (file, name))
    }

    /// Adds the scope of a function. `arena` is the module's.
    pub(crate) fn add_subprogram(
        &mut self,
        arena: &'ir Bump,
        subprogram: Subprogram<'_>,
    ) -> DebugScope {
        let scope = self.scopes.push(Subprogram {
            name: arena.alloc_str(subprogram.name),
            ..subprogram
        });
        _ = self.subprograms.insert(subprogram.function, scope);
        scope
    }

    /// Every scope, in the order they were added.
    pub(crate) fn scopes(&self) -> impl Iterator<Item = (DebugScope, &Subprogram<'ir>)> {
        self.scopes.iter()
    }

    /// The data of `scope`, if the module has it.
    pub(crate) fn scope(&self, scope: DebugScope) -> Option<&Subprogram<'ir>> {
        self.scopes.get(scope)
    }

    /// The scope of `function`, if it was lowered with debug information.
    pub(crate) fn subprogram_of(&self, function: FuncId) -> Option<DebugScope> {
        self.subprograms.get(&function).copied()
    }

    /// The id of `location`, interning it the first time it is seen.
    pub(crate) fn location(&mut self, location: SourceLocation) -> DebugLoc {
        if let Some(&id) = self.location_ids.get(&location) {
            return id;
        }
        let id = self.locations.push(location);
        _ = self.location_ids.insert(location, id);
        id
    }

    /// The data of `id`, if the module has it.
    pub(crate) fn location_data(&self, id: DebugLoc) -> Option<SourceLocation> {
        self.locations.get(id).copied()
    }

    /// How many locations the module has.
    pub(crate) fn location_count(&self) -> usize {
        self.locations.len()
    }

    /// The location of an instruction whose callee location is `callee`,
    /// copied for a call at `call`: the same position, inlined at the end
    /// of `callee`'s own chain of calls.
    pub(crate) fn inlined(&mut self, callee: DebugLoc, call: DebugLoc) -> Option<DebugLoc> {
        let location = self.location_data(callee)?;
        let inlined_at = match location.inlined_at {
            | Some(outer) => self.inlined(outer, call)?,
            | None => call,
        };
        Some(self.location(SourceLocation {
            inlined_at: Some(inlined_at),
            ..location
        }))
    }

    /// The function a location's instruction belongs in: the scope at the
    /// outer end of its chain of inlined calls.
    pub(crate) fn root_function(&self, id: DebugLoc) -> Option<FuncId> {
        let mut location = self.location_data(id)?;
        while let Some(outer) = location.inlined_at {
            location = self.location_data(outer)?;
        }
        Some(self.scope(location.scope)?.function)
    }
}
