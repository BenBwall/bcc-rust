//! Writes a module's debug information ([`crate::ir::DebugInfo`]) as LLVM
//! metadata: a `DICompileUnit` with its producer and language, a `DIFile` per
//! source file, a `DISubprogram` per scope, and a `DILocation` per interned
//! location, which [`Emitter`](super::Emitter) sets as the builder's location
//! around each instruction.
//!
//! Debug records only describe code, and LLVM keeps code generation
//! independent of them, so `-g` does not change the machine code. An MSVC
//! target gets CodeView unless `-gdwarf-N` asks for DWARF, as with Clang; every
//! other target gets DWARF 5 by default, as with GCC and Clang.
//!
//! An instruction with no location keeps none. LLVM requires a location only
//! on a call that its inliner could expand, between two functions that both
//! have subprograms, so a call without one gets line 0 of its function, the
//! way LLVM marks compiler-generated code.

use super::api::{
    CompileUnit,
    DebugBuilder,
    FunctionDescription,
    Meta,
    Session,
    Val,
};
use crate::{
    ir::{
        DebugFile,
        DebugInfo,
        DebugLoc,
        DebugScope,
        Entity,
        FuncId,
        Linkage,
        Module,
        SourceLocation,
    },
    target::Target,
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// The name `DW_AT_producer` gives: this compiler, whatever personality's
/// options it was given, and not one of the compilers it is compatible with.
const PRODUCER: &str = concat!("bcc-rust ", env!("CARGO_PKG_VERSION"));

/// The `DIBuilder` of `module`, if the module has debug information, after
/// recording the debug format in the LLVM module.
pub(super) fn builder<'a, 'id>(
    module: &Module<'_>,
    api: &'a Session<'id>,
    target: Target,
) -> Option<DebugBuilder<'a, 'id>> {
    let unit = module.debug_info().unit()?;
    Some(api.debug_builder(dwarf_version(target, unit.dwarf_version)))
}

/// The metadata of one module's debug information.
pub(super) struct DebugEmitter<'a, 'id, 'm, 'ir> {
    module:    &'m Module<'ir>,
    builder:   &'a DebugBuilder<'a, 'id>,
    scopes:    ArenaVec<'a, Meta<'id>>,
    /// Each location's node, by location index.
    locations: ArenaVec<'a, Meta<'id>>,
}

impl<'a, 'id, 'm, 'ir> DebugEmitter<'a, 'id, 'm, 'ir> {
    /// The emitter of `module`'s debug information, if it has any: the
    /// compile unit, files and subprograms exist once this returns.
    pub(super) fn new(
        module: &'m Module<'ir>,
        builder: &'a DebugBuilder<'a, 'id>,
        arena: &'a Bump,
    ) -> Option<Self> {
        let debug = module.debug_info();
        let unit = debug.unit()?;
        let directory = compilation_directory(arena);
        let mut files = ArenaVec::new_in(arena);
        for (_, name) in debug.files() {
            files.push(builder.file(name, directory));
        }
        let compile_unit_file = files[unit.file.index()];
        _ = builder.compile_unit(CompileUnit {
            file:      compile_unit_file,
            language:  unit.language,
            producer:  PRODUCER,
            optimized: unit.optimized,
            level:     unit.level,
        });
        let mut scopes = ArenaVec::new_in(arena);
        for (_, subprogram) in debug.scopes() {
            let function = module.function(subprogram.function);
            let file = files[subprogram.file.index()];
            scopes.push(builder.function(FunctionDescription {
                scope: file,
                name: subprogram.name,
                symbol: function.name,
                file,
                line: subprogram.line,
                scope_line: subprogram.scope_line,
                ty: builder.unknown_subroutine_type(file),
                local: function.linkage == Linkage::Internal,
                optimized: unit.optimized,
            }));
        }
        // A location names only locations made before it.
        let mut in_file = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
        let mut locations = ArenaVec::with_capacity_in(debug.location_count(), arena);
        for index in 0..debug.location_count() {
            let location = debug.location_data(DebugLoc::new(index))?;
            let scope = scope_in_file(builder, debug, &scopes, &files, &mut in_file, location);
            let inlined_at = location.inlined_at.map(|call| locations[call.index()]);
            locations.push(builder.api().debug_location(
                location.line,
                location.column,
                scope,
                inlined_at,
            ));
        }
        Some(Self {
            module,
            builder,
            scopes,
            locations,
        })
    }

    /// Attaches the subprogram of `id`, if it has one, to its LLVM function.
    pub(super) fn attach(&self, id: FuncId, function: Val<'id>) {
        if let Some(scope) = self.module.debug_info().subprogram_of(id) {
            self.builder
                .api()
                .set_subprogram(function, self.scopes[scope.index()]);
        }
    }

    /// Sets the builder's location for an instruction of `function` at
    /// `location`. A location that does not lead back to the function stays
    /// out rather than fail LLVM's verifier, and a call without one gets
    /// line 0 (see the module docs).
    pub(super) fn set_location(&self, function: FuncId, location: Option<DebugLoc>, call: bool) {
        let debug = self.module.debug_info();
        let api = self.builder.api();
        let node = location
            .filter(|&location| debug.root_function(location) == Some(function))
            .map(|location| self.locations[location.index()])
            .or_else(|| {
                let scope = debug.subprogram_of(function).filter(|_| call)?;
                Some(api.debug_location(0, 0, self.scopes[scope.index()], None))
            });
        api.set_debug_location(node);
    }

    /// Finishes the metadata.
    pub(super) fn finalize(&self) {
        self.builder.finalize();
    }
}

/// The scope of `location`, or a child of it whose code is attributed to the
/// location's file when the scope's own file is another: a `DILocation` has
/// no file of its own. Code of a `#line` or an `#include` inside a function
/// needs this.
fn scope_in_file<'id>(
    builder: &DebugBuilder<'_, 'id>,
    debug: &DebugInfo<'_>,
    scopes: &[Meta<'id>],
    files: &[Meta<'id>],
    in_file: &mut ArenaMap<'_, (DebugScope, DebugFile), Meta<'id>>,
    location: SourceLocation,
) -> Meta<'id> {
    let (scope, file) = (location.scope, location.file);
    let node = scopes[scope.index()];
    if debug.scope(scope).map(|subprogram| subprogram.file) == Some(file) {
        return node;
    }
    *in_file
        .entry((scope, file))
        .or_insert_with(|| builder.file_scope(node, files[file.index()]))
}

/// The DWARF version a module's objects carry, or `None` for CodeView: an
/// MSVC target's format unless `-gdwarf-N` asks for DWARF, and otherwise
/// DWARF 5, the default of GCC 14.2 and Clang 23.
const fn dwarf_version(target: Target, asked: Option<u8>) -> Option<u8> {
    match (asked, target) {
        | (Some(version), _) => Some(version),
        | (None, Target::WindowsMsvc) => None,
        | (None, _) => Some(5),
    }
}

/// The working directory, which `DW_AT_comp_dir` records and relative source
/// names are relative to; empty if it cannot be read.
#[expect(
    clippy::disallowed_methods,
    reason = "The compilation directory is a fact of the host that debug information records."
)]
fn compilation_directory(arena: &Bump) -> &str {
    std::env::current_dir().map_or("", |path| arena.alloc_str(&path.to_string_lossy()))
}
