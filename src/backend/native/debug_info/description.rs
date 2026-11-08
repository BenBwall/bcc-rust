//! What the entries say: the compilation unit with its producer, language,
//! files and code (DWARF 5 §3.1.1), a type entry for each type the module
//! describes (§5), the globals (§4.1), and each described function as a
//! `DW_TAG_subprogram` (§3.3) with its parameters, its locals and the
//! lexical blocks (§3.5) that hold them. A block's code is the runs of code
//! in it or a block inside it; one range is `DW_AT_low_pc` and
//! `DW_AT_high_pc`, several are a list in `.debug_rnglists` (§2.17.3). A
//! block with no code of its own leaves its variables to the block around
//! it, as LLVM drops a scope without code.
//!
//! A variable in a stack slot is at `DW_OP_fbreg`, its offset from the
//! canonical frame address (§2.5.1.2); the native frame keeps a slot at one
//! place for the whole function. One kept elsewhere, in a value or a slot
//! aligned beyond what the frame aligns, has no location: a debugger says it
//! was optimized out, which is better than a wrong value.

use std::fmt::{
    self,
    Write,
};

use super::{
    DebugWriter,
    Described,
    Mark,
    entries::*,
};
use crate::{
    backend::native::{
        defines,
        emit::Name,
        frame::Frame,
        mir::FrameObject,
        object::sleb128,
    },
    ir::{
        Body,
        DebugEncoding,
        DebugLanguage,
        DebugQualifier,
        DebugScope,
        DebugType,
        DebugTypeData,
        DebugVariable,
        Entity,
        Linkage,
        PRODUCER,
        Scope,
        Symbol,
        VariablePlace,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

/// `DW_OP_call_frame_cfa`, `DW_OP_fbreg` and `DW_OP_addr` (§7.7.1, Table
/// 7.9).
const OP_CALL_FRAME_CFA: u8 = 0x9C;
const OP_FBREG: u8 = 0x91;
const OP_ADDR: u8 = 0x03;

/// The label of a described type's entry.
struct TypeLabel(usize);

impl fmt::Display for TypeLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".Ldebug_type{}", self.0)
    }
}

/// The entry of the type of what has no description, which is not `void`.
const UNKNOWN: &str = ".Ldebug_unknown";
/// The entry of the type that counts an array's elements, as Clang names it.
const INDEX: &str = ".Ldebug_index";

/// Writes `writer`'s entries after the listing's code.
pub(super) fn write(writer: &mut DebugWriter<'_, '_, '_>, out: &mut dyn Write) -> fmt::Result {
    let module = writer.module;
    let debug = module.debug_info();
    let Some(unit) = debug.unit() else {
        return Ok(());
    };
    let arena = writer.arena;
    let scopes = debug.scopes().count();
    let (mut variables, mut blocks, mut statics) = (
        by_scope(scopes, arena),
        by_scope(scopes, arena),
        by_scope(scopes, arena),
    );
    for (id, variable) in debug.variables() {
        variables[variable.scope.index()].push(id);
    }
    for (id, scope) in debug.scopes() {
        if let Scope::Block(block) = scope {
            blocks[block.parent.index()].push(id);
        }
    }
    for (at, global) in debug.globals().iter().enumerate() {
        if let Some(scope) = global.scope {
            statics[scope.index()].push(at);
        }
    }
    let mut description = Description {
        writer,
        entries: Entries::new(arena),
        ranges: ArenaString::new_in(arena),
        lists: 0,
        unknown: false,
        index: false,
        variables,
        blocks,
        statics,
    };
    let mut entry = description.entries.entry(TAG_COMPILE_UNIT, true);
    entry.string(AT_PRODUCER, PRODUCER)?;
    entry.short(
        AT_LANGUAGE,
        match unit.language {
            | DebugLanguage::C89 => 0x01,
            | DebugLanguage::C99 => 0x0C,
            | DebugLanguage::C11 => 0x1D,
        },
    )?;
    entry.string(AT_NAME, description.writer.lines.root_name())?;
    entry.offset(AT_STMT_LIST, &".Ldebug_line")?;
    entry.string(AT_COMP_DIR, description.writer.lines.directory())?;
    let functions = &description.writer.functions;
    if let (Some(first), Some(last)) = (functions.first(), functions.last())
        && functions.iter().all(|function| function.in_text)
    {
        let (begin, end) = (Mark::Begin(first.number), Mark::End(last.number));
        entry.address(AT_LOW_PC, &begin)?;
        entry.length(AT_HIGH_PC, &end, &begin)?;
    } else if !functions.is_empty() {
        let mut runs = ArenaVec::new_in(description.writer.arena);
        runs.extend(
            functions
                .iter()
                .map(|function| (Mark::Begin(function.number), Mark::End(function.number))),
        );
        entry.address(AT_LOW_PC, &0)?;
        let list = description.list(&runs)?;
        entry.offset(AT_RANGES, &list)?;
    }
    description.entries.write(entry, None)?;
    for (id, data) in debug.types() {
        description.ty(id, *data)?;
    }
    for global in debug.globals() {
        let local = global.scope.is_some_and(|scope| {
            let function = debug.scope(scope).map(Scope::function);
            description
                .writer
                .functions
                .iter()
                .any(|described| Some(described.function) == function)
        });
        if !local {
            description.global(global)?;
        }
    }
    for index in 0..description.writer.functions.len() {
        description.subprogram(index)?;
    }
    if description.unknown {
        let mut entry = description.entries.entry(TAG_UNSPECIFIED_TYPE, false);
        entry.string(AT_NAME, "unknown")?;
        description.entries.write(entry, Some(&UNKNOWN))?;
    }
    if description.index {
        let mut entry = description.entries.entry(TAG_BASE_TYPE, false);
        entry.string(AT_NAME, "__ARRAY_SIZE_TYPE__")?;
        entry.byte(AT_BYTE_SIZE, 8)?;
        entry.byte(AT_ENCODING, 0x07)?;
        description.entries.write(entry, Some(&INDEX))?;
    }
    description.entries.end_children()?;
    let Description {
        entries, ranges, ..
    } = description;
    // DWARF 5 §7.5.1.1: the unit's length, version 5, `DW_UT_compile`, the
    // address size and the abbreviations' offset.
    writeln!(
        out,
        "\t.section\t.debug_abbrev,\"\",@progbits\n.Ldebug_abbrev:"
    )?;
    entries.abbreviations(out)?;
    writeln!(
        out,
        "\t.section\t.debug_info,\"\",@progbits\n{UNIT}:\n\t.long\t.Ldebug_info_end-.\
         Ldebug_info_start\n.Ldebug_info_start:\n\t.short\t5\n\t.byte\t1\n\t.byte\t8\n\t.long\t.\
         Ldebug_abbrev"
    )?;
    out.write_str(&entries.text)?;
    writeln!(out, ".Ldebug_info_end:")?;
    if !ranges.is_empty() {
        // §7.28: the length, version 5, the address and segment selector
        // sizes, and no offset table.
        writeln!(
            out,
            "\t.section\t.debug_rnglists,\"\",@progbits\n\t.long\t.Ldebug_rnglists_end-.\
             Ldebug_rnglists_start\n.Ldebug_rnglists_start:\n\t.short\t5\n\t.byte\t8\n\t.\
             byte\t0\n\t.long\t0"
        )?;
        out.write_str(&ranges)?;
        writeln!(out, ".Ldebug_rnglists_end:")?;
    }
    // The line table follows this label, which `DW_AT_stmt_list` names.
    writeln!(out, "\t.section\t.debug_line,\"\",@progbits\n.Ldebug_line:")
}

/// Finds where `frame` keeps the variables `body` places in stack slots.
pub(super) fn place_variables(
    module: &crate::ir::Module<'_>,
    body: &Body<'_>,
    frame: &Frame<'_>,
    variables: &mut ArenaVec<'_, (DebugVariable, i64)>,
) {
    let debug = module.debug_info();
    let cfa = i64::from(frame.cfa_offset());
    for declared in body.declared_variables() {
        let VariablePlace::Slot(slot) = declared.place else {
            continue;
        };
        // An inlined copy's variable belongs to the inlined frame.
        let inlined = debug
            .location_data(declared.location)
            .is_some_and(|location| location.inlined_at.is_some());
        // A slot aligned beyond 16 bytes has its address rounded up at run
        // time, so it has no offset of its own from the frame.
        if inlined
            || body.stack_slot(slot).align.bytes() > 16
            || variables.iter().any(|&(variable, _)| variable == declared.variable)
        {
            continue;
        }
        let Ok(index) = u32::try_from(slot.index()) else {
            continue;
        };
        let (_, offset) = frame.address(FrameObject::Slot(index));
        variables.push((declared.variable, offset - cfa));
    }
}

/// The entries as they are written.
struct Description<'w, 'd, 'm, 'ir> {
    writer:    &'w mut DebugWriter<'d, 'm, 'ir>,
    entries:   Entries<'d>,
    /// The range lists of `.debug_rnglists`, after its header.
    ranges:    ArenaString<'d>,
    lists:     u32,
    /// Whether an entry named the unknown type or the index type.
    unknown:   bool,
    index:     bool,
    /// By scope: the variables declared in it, the blocks directly inside
    /// it, and the static locals (as indices of the module's globals).
    variables: ArenaVec<'d, ArenaVec<'d, DebugVariable>>,
    blocks:    ArenaVec<'d, ArenaVec<'d, DebugScope>>,
    statics:   ArenaVec<'d, ArenaVec<'d, usize>>,
}

impl<'d> Description<'_, 'd, '_, '_> {
    /// `DW_AT_type` naming `ty`, or nothing for `void`.
    fn type_of(entry: &mut Entry<'_>, ty: Option<DebugType>) -> fmt::Result {
        match ty {
            | Some(ty) => entry.reference(AT_TYPE, &TypeLabel(ty.index())),
            | None => Ok(()),
        }
    }

    /// `DW_AT_type` naming `ty`, or the unknown type for a variable or
    /// member that has no description.
    fn object_type(&mut self, entry: &mut Entry<'_>, ty: Option<DebugType>) -> fmt::Result {
        if ty.is_none() {
            self.unknown = true;
            return entry.reference(AT_TYPE, &UNKNOWN);
        }
        Self::type_of(entry, ty)
    }

    /// `DW_AT_decl_file` and `DW_AT_decl_line`.
    fn declared(
        &mut self,
        entry: &mut Entry<'_>,
        file: crate::ir::DebugFile,
        line: u32,
    ) -> fmt::Result {
        let debug = self.writer.module.debug_info();
        let number = self.writer.lines.number(&mut self.entries.text, debug, file)?;
        entry.unsigned(AT_DECL_FILE, u64::from(number))?;
        entry.unsigned(AT_DECL_LINE, u64::from(line))
    }

    /// A range list of `runs`, merged where one ends at the next one's
    /// start, as `DW_RLE_start_end` entries; its label is returned.
    fn list(&mut self, runs: &[(Mark, Mark)]) -> Result<ListLabel, fmt::Error> {
        let label = ListLabel(self.lists);
        self.lists += 1;
        writeln!(self.ranges, "{label}:")?;
        for &(begin, end) in runs {
            // DW_RLE_start_end, §7.25 Table 7.30.
            writeln!(self.ranges, "\t.byte\t6\n\t.quad\t{begin}\n\t.quad\t{end}")?;
        }
        writeln!(self.ranges, "\t.byte\t0")?;
        Ok(label)
    }

    /// The entry of a described type, and its children.
    fn ty(&mut self, id: DebugType, data: DebugTypeData<'_>) -> fmt::Result {
        let label = TypeLabel(id.index());
        match data {
            | DebugTypeData::Base {
                name,
                size_bits,
                encoding,
            } => {
                let mut entry = self.entries.entry(TAG_BASE_TYPE, false);
                entry.string(AT_NAME, name)?;
                entry.unsigned(AT_BYTE_SIZE, size_bits / 8)?;
                entry.byte(AT_ENCODING, encoding_code(encoding))?;
                self.entries.write(entry, Some(&label))
            },
            | DebugTypeData::Pointer { pointee } => {
                let mut entry = self.entries.entry(TAG_POINTER_TYPE, false);
                Self::type_of(&mut entry, pointee)?;
                self.entries.write(entry, Some(&label))
            },
            | DebugTypeData::Qualified { qualifier, ty } => {
                let mut entry = self.entries.entry(
                    match qualifier {
                        | DebugQualifier::Const => TAG_CONST_TYPE,
                        | DebugQualifier::Volatile => TAG_VOLATILE_TYPE,
                        | DebugQualifier::Restrict => TAG_RESTRICT_TYPE,
                        | DebugQualifier::Atomic => TAG_ATOMIC_TYPE,
                    },
                    false,
                );
                Self::type_of(&mut entry, ty)?;
                self.entries.write(entry, Some(&label))
            },
            | DebugTypeData::Array { element, count, .. } =>
                self.array(&label, element, count, false),
            | DebugTypeData::Vector { element, count, .. } =>
                self.array(&label, element, Some(count), true),
            | DebugTypeData::Record(record) => {
                let tag = if record.union {
                    TAG_UNION_TYPE
                } else {
                    TAG_STRUCTURE_TYPE
                };
                let children = record.complete && !record.members.is_empty();
                let mut entry = self.entries.entry(tag, children);
                if let Some(name) = record.name {
                    entry.string(AT_NAME, name)?;
                }
                if record.complete {
                    entry.unsigned(AT_BYTE_SIZE, record.size_bits / 8)?;
                } else {
                    entry.flag(AT_DECLARATION);
                }
                self.declared(&mut entry, record.file, record.line)?;
                self.entries.write(entry, Some(&label))?;
                if !children {
                    return Ok(());
                }
                for member in record.members {
                    let mut entry = self.entries.entry(TAG_MEMBER, false);
                    if let Some(name) = member.name {
                        entry.string(AT_NAME, name)?;
                    }
                    self.object_type(&mut entry, member.ty)?;
                    self.declared(&mut entry, record.file, member.line)?;
                    // A bit-field's place is in bits from the record's start
                    // (§5.7.6).
                    if let Some(width) = member.bit_field {
                        entry.unsigned(AT_BIT_SIZE, u64::from(width))?;
                        entry.unsigned(AT_DATA_BIT_OFFSET, member.offset_bits)?;
                    } else {
                        entry.unsigned(AT_DATA_MEMBER_LOCATION, member.offset_bits / 8)?;
                    }
                    self.entries.write(entry, None)?;
                }
                self.entries.end_children()
            },
            | DebugTypeData::Enumeration {
                name,
                file,
                line,
                size_bits,
                base,
                enumerators,
                ..
            } => {
                let children = !enumerators.is_empty();
                let mut entry = self.entries.entry(TAG_ENUMERATION_TYPE, children);
                Self::type_of(&mut entry, base)?;
                if let Some(name) = name {
                    entry.string(AT_NAME, name)?;
                }
                entry.unsigned(AT_BYTE_SIZE, size_bits / 8)?;
                self.declared(&mut entry, file, line)?;
                self.entries.write(entry, Some(&label))?;
                if !children {
                    return Ok(());
                }
                for enumerator in enumerators {
                    let mut entry = self.entries.entry(TAG_ENUMERATOR, false);
                    entry.string(AT_NAME, enumerator.name)?;
                    entry.signed(AT_CONST_VALUE, enumerator.value)?;
                    self.entries.write(entry, None)?;
                }
                self.entries.end_children()
            },
            | DebugTypeData::Typedef {
                name,
                ty,
                file,
                line,
            } => {
                let mut entry = self.entries.entry(TAG_TYPEDEF, false);
                Self::type_of(&mut entry, ty)?;
                entry.string(AT_NAME, name)?;
                self.declared(&mut entry, file, line)?;
                self.entries.write(entry, Some(&label))
            },
            | DebugTypeData::Function {
                result,
                parameters,
                variadic,
                prototyped,
            } => {
                let children = !parameters.is_empty() || variadic;
                let mut entry = self.entries.entry(TAG_SUBROUTINE_TYPE, children);
                if prototyped {
                    entry.flag(AT_PROTOTYPED);
                }
                Self::type_of(&mut entry, result)?;
                self.entries.write(entry, Some(&label))?;
                if !children {
                    return Ok(());
                }
                for &parameter in parameters {
                    let mut entry = self.entries.entry(TAG_FORMAL_PARAMETER, false);
                    self.object_type(&mut entry, parameter)?;
                    self.entries.write(entry, None)?;
                }
                if variadic {
                    let entry = self.entries.entry(TAG_UNSPECIFIED_PARAMETERS, false);
                    self.entries.write(entry, None)?;
                }
                self.entries.end_children()
            },
        }
    }

    /// An array or GNU vector type with one subrange: `count` elements, or
    /// an unknown number (§5.5, §5.13).
    fn array(
        &mut self,
        label: &TypeLabel,
        element: Option<DebugType>,
        count: Option<u64>,
        vector: bool,
    ) -> fmt::Result {
        let mut entry = self.entries.entry(TAG_ARRAY_TYPE, true);
        if vector {
            entry.flag(AT_GNU_VECTOR);
        }
        self.object_type(&mut entry, element)?;
        self.entries.write(entry, Some(label))?;
        let mut entry = self.entries.entry(TAG_SUBRANGE_TYPE, false);
        self.index = true;
        entry.reference(AT_TYPE, &INDEX)?;
        if let Some(count) = count {
            entry.unsigned(AT_COUNT, count)?;
        }
        self.entries.write(entry, None)?;
        self.entries.end_children()
    }

    /// A global object or static local: at its address if the listing
    /// defines it, and a declaration if the module does not. A
    /// thread-local one is described without a location.
    fn global(&mut self, global: &crate::ir::GlobalVariable<'_>) -> fmt::Result {
        let module = self.writer.module;
        let object = module.global(global.global);
        let mut entry = self.entries.entry(TAG_VARIABLE, false);
        entry.string(AT_NAME, global.name)?;
        self.object_type(&mut entry, global.ty)?;
        if object.linkage != Linkage::Internal {
            entry.flag(AT_EXTERNAL);
        }
        self.declared(&mut entry, global.file, global.line)?;
        if !defines(module, Symbol::Global(global.global)) {
            entry.flag(AT_DECLARATION);
        } else if !object.thread_local {
            entry.expression(AT_LOCATION, &[OP_ADDR], Some(&Name(object.name)))?;
        }
        self.entries.write(entry, None)
    }

    /// The entry of the `index`th described function, with its parameters,
    /// locals and blocks.
    fn subprogram(&mut self, index: usize) -> fmt::Result {
        let module = self.writer.module;
        let debug = module.debug_info();
        let described = &self.writer.functions[index];
        let (number, function_id, frame_base) =
            (described.number, described.function, described.frame_base);
        let Some(scope) = debug.subprogram_of(function_id) else {
            return Ok(());
        };
        let Some(&Scope::Subprogram(subprogram)) = debug.scope(scope) else {
            return Ok(());
        };
        let function = module.function(function_id);
        let mut entry = self.entries.entry(TAG_SUBPROGRAM, true);
        let begin = Mark::Begin(number);
        entry.address(AT_LOW_PC, &begin)?;
        entry.length(AT_HIGH_PC, &Mark::End(number), &begin)?;
        if frame_base {
            entry.expression(AT_FRAME_BASE, &[OP_CALL_FRAME_CFA], None)?;
        }
        if subprogram.name != function.name {
            entry.string(AT_LINKAGE_NAME, function.name)?;
        }
        entry.string(AT_NAME, subprogram.name)?;
        self.declared(&mut entry, subprogram.file, subprogram.line)?;
        if let Some(&DebugTypeData::Function {
            result, prototyped, ..
        }) = subprogram.ty.and_then(|ty| debug.type_data(ty))
        {
            if prototyped {
                entry.flag(AT_PROTOTYPED);
            }
            Self::type_of(&mut entry, result)?;
        }
        if function.linkage != Linkage::Internal {
            entry.flag(AT_EXTERNAL);
        }
        self.entries.write(entry, None)?;
        self.scope_contents(index, scope)?;
        self.entries.end_children()
    }

    /// The variables, static locals and blocks of `scope` in the `index`th
    /// described function: the parameters first, by position.
    fn scope_contents(&mut self, index: usize, scope: DebugScope) -> fmt::Result {
        let module = self.writer.module;
        let debug = module.debug_info();
        let mut variables = self.variables[scope.index()].clone();
        variables.sort_by_key(|&id| {
            debug
                .variable(id)
                .and_then(|variable| variable.argument)
                .unwrap_or(u32::MAX)
        });
        for id in variables {
            let Some(&variable) = debug.variable(id) else {
                continue;
            };
            let tag = if variable.argument.is_some() {
                TAG_FORMAL_PARAMETER
            } else {
                TAG_VARIABLE
            };
            let place = self.writer.functions[index]
                .variables
                .iter()
                .find(|&&(placed, _)| placed == id)
                .map(|&(_, offset)| offset);
            let mut entry = self.entries.entry(tag, false);
            if let Some(offset) = place {
                let mut bytes = ArenaVec::new_in(self.writer.arena);
                bytes.push(OP_FBREG);
                sleb128(&mut bytes, offset);
                entry.expression(AT_LOCATION, &bytes, None)?;
            }
            entry.string(AT_NAME, variable.name)?;
            self.declared(&mut entry, variable.file, variable.line)?;
            self.object_type(&mut entry, variable.ty)?;
            self.entries.write(entry, None)?;
        }
        for at in self.statics[scope.index()].clone() {
            self.global(&debug.globals()[at])?;
        }
        for block in self.blocks[scope.index()].clone() {
            let runs = self.runs_within(index, block);
            if runs.is_empty() {
                self.scope_contents(index, block)?;
                continue;
            }
            let mut entry = self.entries.entry(TAG_LEXICAL_BLOCK, true);
            if let [(begin, end)] = runs[..] {
                entry.address(AT_LOW_PC, &begin)?;
                entry.length(AT_HIGH_PC, &end, &begin)?;
            } else {
                let list = self.list(&runs)?;
                entry.offset(AT_RANGES, &list)?;
            }
            self.entries.write(entry, None)?;
            self.scope_contents(index, block)?;
            self.entries.end_children()?;
        }
        Ok(())
    }

    /// The code of the `index`th described function in `block` or a block
    /// inside it, as ranges from one label to another.
    fn runs_within(&self, index: usize, block: DebugScope) -> ArenaVec<'d, (Mark, Mark)> {
        let debug = self.writer.module.debug_info();
        let described: &Described<'_> = &self.writer.functions[index];
        let mut ranges: ArenaVec<'d, (Mark, Mark)> = ArenaVec::new_in(self.writer.arena);
        for (at, &(scope, begin)) in described.runs.iter().enumerate() {
            let end = described
                .runs
                .get(at + 1)
                .map_or(Mark::End(described.number), |&(_, next)| next);
            let mut inner = Some(scope);
            let within = loop {
                match inner {
                    | Some(current) if current == block => break true,
                    | Some(current) => {
                        inner = match debug.scope(current) {
                            | Some(Scope::Block(lexical)) => Some(lexical.parent),
                            | _ => None,
                        };
                    },
                    | None => break false,
                }
            };
            if !within {
                continue;
            }
            match ranges.last_mut() {
                | Some(last) if last.1 == begin => last.1 = end,
                | _ => ranges.push((begin, end)),
            }
        }
        ranges
    }
}

/// An empty list for each of `count` scopes.
fn by_scope<T>(count: usize, arena: &Bump) -> ArenaVec<'_, ArenaVec<'_, T>> {
    let mut table = ArenaVec::with_capacity_in(count, arena);
    table.resize_with(count, || ArenaVec::new_in(arena));
    table
}

/// The label of a range list.
struct ListLabel(u32);

impl fmt::Display for ListLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".Ldebug_ranges{}", self.0)
    }
}

/// `DW_AT_encoding`'s code for `encoding` (§7.8, Table 7.11).
const fn encoding_code(encoding: DebugEncoding) -> u8 {
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
