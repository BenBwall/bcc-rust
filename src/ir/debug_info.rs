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
//! With `-g` or `-g2` the tables also describe the program's data: the types
//! ([`DebugTypeData`], reduced from semantic analysis's so that the IR does not
//! depend on it), the local variables, which a body places in a stack slot or
//! at an address ([`DeclaredVariable`]), and the globals. A scope is then a
//! function or a block inside one, so that a shadowing name resolves as in C.
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
    DebugType,
    DebugVariable,
    FuncId,
    GlobalId,
    StackSlot,
    Table,
    Value,
};
use crate::util::bump::{
    ArenaMap,
    ArenaVec,
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
    scopes:       Table<'ir, DebugScope, Scope<'ir>>,
    /// The scope of each function that has one.
    subprograms:  ArenaMap<'ir, FuncId, DebugScope>,
    locations:    Table<'ir, DebugLoc, SourceLocation>,
    location_ids: ArenaMap<'ir, SourceLocation, DebugLoc>,
    types:        Table<'ir, DebugType, DebugTypeData<'ir>>,
    variables:    Table<'ir, DebugVariable, Variable<'ir>>,
    globals:      ArenaVec<'ir, GlobalVariable<'ir>>,
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
    /// which is DWARF 5, or `CodeView` for an MSVC target.
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

/// A scope: a function, or a block inside one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Scope<'ir> {
    Subprogram(Subprogram<'ir>),
    Block(LexicalBlock),
}

impl Scope<'_> {
    /// The function the scope is in.
    pub(crate) const fn function(&self) -> FuncId {
        match self {
            | Self::Subprogram(subprogram) => subprogram.function,
            | Self::Block(block) => block.function,
        }
    }

    /// The file a location in the scope belongs to unless it says otherwise.
    pub(crate) const fn file(&self) -> DebugFile {
        match self {
            | Self::Subprogram(subprogram) => subprogram.file,
            | Self::Block(block) => block.file,
        }
    }
}

/// A block scope, where a name declared in it hides an outer one.
/// C99: §6.2.1 paragraph 4, pp. 29-30; PDF pp. 41-42.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LexicalBlock {
    pub(crate) parent:   DebugScope,
    pub(crate) function: FuncId,
    pub(crate) file:     DebugFile,
    pub(crate) line:     u32,
    pub(crate) column:   u32,
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
    /// Its function type, absent at `-g1`.
    pub(crate) ty:         Option<DebugType>,
}

/// How a base type's bits are read, as `DW_AT_encoding` says.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DebugEncoding {
    Boolean,
    Float,
    Complex,
    Signed,
    SignedChar,
    Unsigned,
    UnsignedChar,
}

impl DebugEncoding {
    /// The keyword of the IR's text form.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            | Self::Boolean => "boolean",
            | Self::Float => "float",
            | Self::Complex => "complex",
            | Self::Signed => "signed",
            | Self::SignedChar => "signed_char",
            | Self::Unsigned => "unsigned",
            | Self::UnsignedChar => "unsigned_char",
        }
    }

    /// The encoding `keyword` names in the IR's text form.
    pub(crate) fn from_keyword(keyword: &str) -> Option<Self> {
        [
            Self::Boolean,
            Self::Float,
            Self::Complex,
            Self::Signed,
            Self::SignedChar,
            Self::Unsigned,
            Self::UnsignedChar,
        ]
        .into_iter()
        .find(|encoding| encoding.keyword() == keyword)
    }
}

/// A qualifier on a type. C99: §6.7.3, pp. 108-109; PDF pp. 120-121.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DebugQualifier {
    Const,
    Volatile,
    Restrict,
    Atomic,
}

impl DebugQualifier {
    /// The keyword of the IR's text form.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            | Self::Const => "const",
            | Self::Volatile => "volatile",
            | Self::Restrict => "restrict",
            | Self::Atomic => "atomic",
        }
    }

    /// The qualifier `keyword` names in the IR's text form.
    pub(crate) fn from_keyword(keyword: &str) -> Option<Self> {
        [Self::Const, Self::Volatile, Self::Restrict, Self::Atomic]
            .into_iter()
            .find(|qualifier| qualifier.keyword() == keyword)
    }
}

/// A member of a described structure or union. An anonymous member has no
/// name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DebugMember<'ir> {
    pub(crate) name:        Option<&'ir str>,
    pub(crate) ty:          Option<DebugType>,
    pub(crate) line:        u32,
    /// From the start of the record, in bits.
    pub(crate) offset_bits: u64,
    /// The width of a bit-field.
    pub(crate) bit_field:   Option<u32>,
}

/// An enumeration constant. C99: §6.7.2.2, pp. 104-105; PDF pp. 116-117.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DebugEnumerator<'ir> {
    pub(crate) name:  &'ir str,
    pub(crate) value: i64,
}

/// A described structure or union.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DebugRecord<'ir> {
    pub(crate) union:      bool,
    pub(crate) name:       Option<&'ir str>,
    pub(crate) file:       DebugFile,
    pub(crate) line:       u32,
    pub(crate) size_bits:  u64,
    pub(crate) align_bits: u32,
    /// A type left incomplete, which only points or is pointed at.
    pub(crate) complete:   bool,
    pub(crate) members:    &'ir [DebugMember<'ir>],
}

/// A described type. `None` where a type is expected means `void`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DebugTypeData<'ir> {
    Base {
        name:      &'ir str,
        size_bits: u64,
        encoding:  DebugEncoding,
    },
    Pointer {
        pointee: Option<DebugType>,
    },
    Qualified {
        qualifier: DebugQualifier,
        ty:        Option<DebugType>,
    },
    Array {
        element:    Option<DebugType>,
        /// `None` for an incomplete or variable-length array.
        count:      Option<u64>,
        size_bits:  u64,
        align_bits: u32,
    },
    Vector {
        element:    Option<DebugType>,
        count:      u64,
        size_bits:  u64,
        align_bits: u32,
    },
    Record(DebugRecord<'ir>),
    Enumeration {
        name:        Option<&'ir str>,
        file:        DebugFile,
        line:        u32,
        size_bits:   u64,
        align_bits:  u32,
        /// The compatible integer type. C99: §6.7.2.2 paragraph 4, p. 105;
        /// PDF p. 117.
        base:        Option<DebugType>,
        enumerators: &'ir [DebugEnumerator<'ir>],
    },
    Typedef {
        name: &'ir str,
        ty:   Option<DebugType>,
        file: DebugFile,
        line: u32,
    },
    Function {
        result:     Option<DebugType>,
        parameters: &'ir [Option<DebugType>],
        variadic:   bool,
        prototyped: bool,
    },
}

/// A local variable or parameter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Variable<'ir> {
    pub(crate) name:     &'ir str,
    pub(crate) scope:    DebugScope,
    pub(crate) file:     DebugFile,
    pub(crate) line:     u32,
    pub(crate) ty:       Option<DebugType>,
    /// A parameter's 1-based position.
    pub(crate) argument: Option<u32>,
}

/// Where a body keeps a variable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum VariablePlace {
    Slot(StackSlot),
    /// A value holding its address, for an object of variable size or one a
    /// parameter's caller made.
    Address(Value),
}

/// A variable's storage in one function body, in a side table beside the
/// instructions: an instruction would count toward the inliner's budget and
/// so change the code. C99: §6.2.4, p. 32; PDF p. 44.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DeclaredVariable {
    pub(crate) variable: DebugVariable,
    pub(crate) place:    VariablePlace,
    pub(crate) location: DebugLoc,
}

/// A global object, or a static local when it has a scope.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct GlobalVariable<'ir> {
    pub(crate) global: GlobalId,
    pub(crate) name:   &'ir str,
    pub(crate) scope:  Option<DebugScope>,
    pub(crate) file:   DebugFile,
    pub(crate) line:   u32,
    pub(crate) ty:     Option<DebugType>,
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
            types:        Table::new_in(arena),
            variables:    Table::new_in(arena),
            globals:      ArenaVec::new_in(arena),
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
        let scope = self.scopes.push(Scope::Subprogram(Subprogram {
            name: arena.alloc_str(subprogram.name),
            ..subprogram
        }));
        _ = self.subprograms.insert(subprogram.function, scope);
        scope
    }

    /// Adds the scope of a block.
    pub(crate) fn add_block(&mut self, block: LexicalBlock) -> DebugScope {
        self.scopes.push(Scope::Block(block))
    }

    /// Every scope, in the order they were added. A block's parent comes
    /// before it.
    pub(crate) fn scopes(&self) -> impl Iterator<Item = (DebugScope, &Scope<'ir>)> {
        self.scopes.iter()
    }

    /// The data of `scope`, if the module has it.
    pub(crate) fn scope(&self, scope: DebugScope) -> Option<&Scope<'ir>> {
        self.scopes.get(scope)
    }

    /// Adds a described type.
    pub(crate) fn add_type(&mut self, data: DebugTypeData<'ir>) -> DebugType {
        self.types.push(data)
    }

    /// Fills in a structure or union added before its members, which may
    /// point back at it.
    pub(crate) fn set_type(&mut self, ty: DebugType, data: DebugTypeData<'ir>) {
        self.types[ty] = data;
    }

    /// Every described type, in the order they were added.
    pub(crate) fn types(&self) -> impl Iterator<Item = (DebugType, &DebugTypeData<'ir>)> {
        self.types.iter()
    }

    /// How many types the module describes.
    pub(crate) fn type_count(&self) -> usize {
        self.types.len()
    }

    /// Adds a variable.
    pub(crate) fn add_variable(&mut self, variable: Variable<'ir>) -> DebugVariable {
        self.variables.push(variable)
    }

    /// Every variable, in the order they were added.
    pub(crate) fn variables(&self) -> impl Iterator<Item = (DebugVariable, &Variable<'ir>)> {
        self.variables.iter()
    }

    /// The data of `variable`, if the module has it.
    pub(crate) fn variable(&self, variable: DebugVariable) -> Option<&Variable<'ir>> {
        self.variables.get(variable)
    }

    /// Adds a global object.
    pub(crate) fn add_global(&mut self, global: GlobalVariable<'ir>) {
        self.globals.push(global);
    }

    /// Every described global object.
    pub(crate) fn globals(&self) -> &[GlobalVariable<'ir>] {
        &self.globals
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
        Some(self.scope(location.scope)?.function())
    }
}
