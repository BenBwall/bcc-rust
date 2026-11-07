//! Writes a module's debug information ([`crate::ir::DebugInfo`]) as LLVM
//! metadata: a `DICompileUnit` with its producer and language, a `DIFile` per
//! source file, a `DISubprogram` or `DILexicalBlock` per scope, the types, the
//! variables, and a `DILocation` per interned location, which
//! [`Emitter`](super::Emitter) sets as the builder's location around each
//! instruction. A variable with a place gets a `#dbg_declare` record, which a
//! debugger reads and LLVM's own promotion follows at `-O1` and above.
//!
//! Debug records only describe code, and LLVM keeps code generation
//! independent of them, so `-g` does not change the machine code. An MSVC
//! target gets `CodeView` unless `-gdwarf-N` asks for DWARF, as with Clang;
//! every other target gets DWARF 5 by default, as with GCC and Clang.
//!
//! An instruction with no location keeps none. LLVM requires a location only
//! on a call that its inliner could expand, between two functions that both
//! have subprograms, so a call without one gets line 0 of its function, the
//! way LLVM marks compiler-generated code.

use llvm_sys::prelude::LLVMMetadataRef;

use super::api::{
    Block,
    CompileUnit,
    DebugBuilder,
    FunctionDescription,
    MemberDescription,
    Meta,
    Session,
    Val,
    VariableDescription,
};
use crate::{
    ir::{
        DebugEncoding,
        DebugFile,
        DebugInfo,
        DebugLoc,
        DebugQualifier,
        DebugRecord,
        DebugScope,
        DebugType,
        DebugTypeData,
        DebugVariable,
        DeclaredVariable,
        Entity,
        FuncId,
        GlobalVariable,
        Linkage,
        Module,
        Scope,
        SourceLocation,
    },
    target::Target,
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

const TAG_STRUCTURE: u32 = 0x13;
const TAG_UNION: u32 = 0x17;
const TAG_CONST: u32 = 0x26;
const TAG_VOLATILE: u32 = 0x35;
const TAG_RESTRICT: u32 = 0x37;
const TAG_ATOMIC: u32 = 0x47;

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
    /// Each variable's node, by variable index.
    variables: ArenaVec<'a, Meta<'id>>,
    /// Each described type's node, by type index.
    types:     ArenaVec<'a, Option<Meta<'id>>>,
    /// The type of what has no description: not `void`, which a variable
    /// cannot have.
    unknown:   Meta<'id>,
    files:     ArenaVec<'a, Meta<'id>>,
}

impl<'a, 'id, 'm, 'ir> DebugEmitter<'a, 'id, 'm, 'ir> {
    /// The emitter of `module`'s debug information, if it has any: the
    /// compile unit, files, types, scopes and variables exist once this
    /// returns.
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
        let unknown = builder.unspecified_type("unknown");
        let types = build_types(module, builder, &files, unknown, arena);
        let mut scopes: ArenaVec<'a, Meta<'id>> = ArenaVec::new_in(arena);
        for (_, scope) in debug.scopes() {
            scopes.push(match *scope {
                | Scope::Subprogram(subprogram) => {
                    let function = module.function(subprogram.function);
                    let file = files[subprogram.file.index()];
                    let ty = subprogram
                        .ty
                        .and_then(|ty| types[ty.index()])
                        .unwrap_or_else(|| builder.unknown_subroutine_type(file));
                    builder.function(FunctionDescription {
                        scope: file,
                        name: subprogram.name,
                        symbol: function.name,
                        file,
                        line: subprogram.line,
                        scope_line: subprogram.scope_line,
                        ty,
                        local: function.linkage == Linkage::Internal,
                        optimized: unit.optimized,
                    })
                },
                | Scope::Block(block) => builder.lexical_block(
                    scopes[block.parent.index()],
                    files[block.file.index()],
                    block.line,
                    block.column,
                ),
            });
        }
        let mut variables = ArenaVec::new_in(arena);
        for (_, variable) in debug.variables() {
            variables.push(
                builder.local_variable(VariableDescription {
                    name:     variable.name,
                    scope:    scopes[variable.scope.index()],
                    file:     files[variable.file.index()],
                    line:     variable.line,
                    ty:       variable
                        .ty
                        .and_then(|ty| types[ty.index()])
                        .unwrap_or(unknown),
                    argument: variable.argument,
                }),
            );
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
            variables,
            types,
            unknown,
            files,
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

    /// Records in `block`, as it is built, that the variable `declared`
    /// names lives at `storage`.
    pub(super) fn declare(&self, declared: DeclaredVariable, storage: Val<'id>, block: Block<'id>) {
        self.builder.declare(
            storage,
            self.variable(declared.variable),
            self.locations[declared.location.index()],
            block,
        );
    }

    fn variable(&self, variable: DebugVariable) -> Meta<'id> {
        self.variables[variable.index()]
    }

    /// Describes the module's global objects, once `globals` have them.
    pub(super) fn describe_globals(&self, globals: &[Val<'id>]) {
        let debug = self.module.debug_info();
        for &GlobalVariable {
            global,
            name,
            scope,
            file,
            line,
            ty,
        } in debug.globals()
        {
            let file = self.files[file.index()];
            let linkage = self.module.global(global).linkage;
            self.builder.global_variable(
                globals[global.index()],
                VariableDescription {
                    name,
                    scope: scope.map_or(file, |scope| self.scopes[scope.index()]),
                    file,
                    line,
                    ty: ty
                        .and_then(|ty| self.types[ty.index()])
                        .unwrap_or(self.unknown),
                    argument: None,
                },
                linkage == Linkage::Internal,
            );
        }
    }

    /// Finishes the metadata.
    pub(super) fn finalize(&self) {
        self.builder.finalize();
    }
}

/// The nodes of the described types. A structure or union starts as a
/// temporary, so that what points back at it has something to name, and is
/// replaced once its members exist; everything else names types made before
/// it, by how lowering makes them.
fn build_types<'a, 'id>(
    module: &Module<'_>,
    builder: &DebugBuilder<'_, 'id>,
    files: &[Meta<'id>],
    unknown: Meta<'id>,
    arena: &'a Bump,
) -> ArenaVec<'a, Option<Meta<'id>>> {
    let debug = module.debug_info();
    let pointer_bits = module.pointer_bytes() * 8;
    let mut types: ArenaVec<'a, Option<Meta<'id>>> =
        ArenaVec::with_capacity_in(debug.type_count(), arena);
    types.resize(debug.type_count(), None);
    for (id, data) in debug.types() {
        if let DebugTypeData::Record(record) = data {
            let tag = if record.union {
                TAG_UNION
            } else {
                TAG_STRUCTURE
            };
            let name = record.name.unwrap_or("");
            let file = files[record.file.index()];
            types[id.index()] = Some(if record.complete {
                builder.replaceable_record(tag, name, file, record.line)
            } else {
                builder.incomplete_record(tag, name, file, record.line)
            });
        }
    }
    let lookup = |types: &ArenaVec<'a, Option<Meta<'id>>>, ty: Option<DebugType>| {
        ty.and_then(|ty| types[ty.index()])
    };
    for (id, data) in debug.types() {
        let node = match *data {
            | DebugTypeData::Record(_) => continue,
            | DebugTypeData::Base {
                name,
                size_bits,
                encoding,
            } => builder.base_type(name, size_bits, encoding_code(encoding)),
            | DebugTypeData::Pointer { pointee } =>
                builder.pointer_type(lookup(&types, pointee), pointer_bits),
            | DebugTypeData::Qualified { qualifier, ty } => builder.qualified_type(
                match qualifier {
                    | DebugQualifier::Const => TAG_CONST,
                    | DebugQualifier::Volatile => TAG_VOLATILE,
                    | DebugQualifier::Restrict => TAG_RESTRICT,
                    | DebugQualifier::Atomic => TAG_ATOMIC,
                },
                lookup(&types, ty),
            ),
            | DebugTypeData::Array {
                element,
                count,
                size_bits,
                align_bits,
            } => builder.array_type(
                lookup(&types, element).unwrap_or(unknown),
                count
                    .and_then(|count| i64::try_from(count).ok())
                    .unwrap_or(-1),
                (size_bits, align_bits),
                false,
            ),
            | DebugTypeData::Vector {
                element,
                count,
                size_bits,
                align_bits,
            } => builder.array_type(
                lookup(&types, element).unwrap_or(unknown),
                i64::try_from(count).unwrap_or(-1),
                (size_bits, align_bits),
                true,
            ),
            | DebugTypeData::Enumeration {
                name,
                file,
                line,
                size_bits,
                align_bits,
                base,
                enumerators,
            } => {
                let mut constants = ArenaVec::with_capacity_in(enumerators.len(), arena);
                for enumerator in enumerators {
                    constants.push(builder.enumerator(enumerator.name, enumerator.value));
                }
                builder.enumeration(
                    name.unwrap_or(""),
                    (files[file.index()], line),
                    (size_bits, align_bits),
                    &mut constants,
                    lookup(&types, base),
                )
            },
            | DebugTypeData::Typedef {
                name,
                ty,
                file,
                line,
            } => builder.typedef(name, lookup(&types, ty), files[file.index()], line),
            | DebugTypeData::Function {
                result,
                parameters,
                variadic,
                prototyped,
            } => {
                let raw = |node: Option<Meta<'id>>| node.map_or(std::ptr::null_mut(), Meta::raw);
                let mut elements: ArenaVec<'_, LLVMMetadataRef> =
                    ArenaVec::with_capacity_in(parameters.len() + 2, arena);
                elements.push(raw(lookup(&types, result)));
                for &parameter in parameters {
                    elements.push(raw(Some(lookup(&types, parameter).unwrap_or(unknown))));
                }
                if variadic {
                    elements.push(std::ptr::null_mut());
                }
                builder.subroutine_type(unit_file(module, files), &mut elements, prototyped)
            },
        };
        types[id.index()] = Some(node);
    }
    for (id, data) in debug.types() {
        if let DebugTypeData::Record(record) = data
            && record.complete
        {
            let finished = record_node(module, builder, files, unknown, &types, id, record, arena);
            builder.replace_record(types[id.index()].expect("made above"), finished);
            types[id.index()] = Some(finished);
        }
    }
    types
}

/// The finished node of `record`, whose members name the temporary.
#[expect(
    clippy::too_many_arguments,
    reason = "The tables of the type pass; none of them is its own concern."
)]
fn record_node<'id>(
    module: &Module<'_>,
    builder: &DebugBuilder<'_, 'id>,
    files: &[Meta<'id>],
    unknown: Meta<'id>,
    types: &[Option<Meta<'id>>],
    id: DebugType,
    record: &DebugRecord<'_>,
    arena: &Bump,
) -> Meta<'id> {
    let temporary = types[id.index()].expect("made above");
    let file = files[record.file.index()];
    let mut members = ArenaVec::with_capacity_in(record.members.len(), arena);
    for member in record.members {
        let size_bits = member
            .ty
            .map_or(0, |ty| type_bits(module.debug_info(), ty, module));
        members.push(
            builder.member(
                temporary,
                MemberDescription {
                    name: member.name.unwrap_or(""),
                    file,
                    line: member.line,
                    size_bits,
                    offset_bits: member.offset_bits,
                    // As Clang's: where the storage unit of the declared type
                    // starts.
                    storage_offset_bits: match size_bits {
                        | 0 => member.offset_bits,
                        | size => member.offset_bits / size * size,
                    },
                    bit_field: member.bit_field,
                    ty: member
                        .ty
                        .and_then(|ty| types[ty.index()])
                        .unwrap_or(unknown),
                },
            ),
        );
    }
    builder.record(
        record.union,
        record.name.unwrap_or(""),
        (file, record.line),
        (record.size_bits, record.align_bits),
        &mut members,
    )
}

/// How many bits a value of the described type `ty` takes.
fn type_bits(debug: &DebugInfo<'_>, mut ty: DebugType, module: &Module<'_>) -> u64 {
    loop {
        let Some((_, data)) = debug.types().find(|&(id, _)| id == ty) else {
            return 0;
        };
        match *data {
            | DebugTypeData::Base { size_bits, .. }
            | DebugTypeData::Array { size_bits, .. }
            | DebugTypeData::Vector { size_bits, .. }
            | DebugTypeData::Enumeration { size_bits, .. } => return size_bits,
            | DebugTypeData::Record(record) => return record.size_bits,
            | DebugTypeData::Pointer { .. } => return module.pointer_bytes() * 8,
            | DebugTypeData::Function { .. } => return 0,
            | DebugTypeData::Qualified { ty: inner, .. }
            | DebugTypeData::Typedef { ty: inner, .. } => match inner {
                | Some(inner) => ty = inner,
                | None => return 0,
            },
        }
    }
}

const fn encoding_code(encoding: DebugEncoding) -> u32 {
    match encoding {
        | DebugEncoding::Boolean => 0x02,
        | DebugEncoding::Complex => 0x03,
        | DebugEncoding::Float => 0x04,
        | DebugEncoding::Signed => 0x05,
        | DebugEncoding::SignedChar => 0x06,
        | DebugEncoding::Unsigned => 0x07,
        | DebugEncoding::UnsignedChar => 0x08,
    }
}

fn unit_file<'id>(module: &Module<'_>, files: &[Meta<'id>]) -> Meta<'id> {
    let unit = module
        .debug_info()
        .unit()
        .expect("only a module with a unit has an emitter");
    files[unit.file.index()]
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
    if debug.scope(scope).map(Scope::file) == Some(file) {
        return node;
    }
    *in_file
        .entry((scope, file))
        .or_insert_with(|| builder.file_scope(node, files[file.index()]))
}

/// The DWARF version a module's objects carry, or `None` for `CodeView`: an
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
fn compilation_directory(arena: &Bump) -> &str {
    std::env::current_dir().map_or("", |path| arena.alloc_str(&path.to_string_lossy()))
}
