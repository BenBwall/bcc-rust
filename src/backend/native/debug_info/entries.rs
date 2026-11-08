//! Debugging information entries as assembler data directives (DWARF 5
//! §7.5): each entry's abbreviation code, then its attributes' values in
//! their forms. An abbreviation, the entry's tag, whether it has children
//! and its attributes with their forms, gets its code the first time an
//! entry has its shape, and `.debug_abbrev` lists them once every entry is
//! written (§7.5.3).

use std::fmt::{
    self,
    Write,
};

use crate::util::bump::{
    ArenaString,
    ArenaVec,
    Bump,
};

// Tags, DWARF 5 §7.5.3, Table 7.3.
pub(super) const TAG_ARRAY_TYPE: u16 = 0x01;
pub(super) const TAG_ENUMERATION_TYPE: u16 = 0x04;
pub(super) const TAG_FORMAL_PARAMETER: u16 = 0x05;
pub(super) const TAG_LEXICAL_BLOCK: u16 = 0x0B;
pub(super) const TAG_MEMBER: u16 = 0x0D;
pub(super) const TAG_POINTER_TYPE: u16 = 0x0F;
pub(super) const TAG_COMPILE_UNIT: u16 = 0x11;
pub(super) const TAG_STRUCTURE_TYPE: u16 = 0x13;
pub(super) const TAG_SUBROUTINE_TYPE: u16 = 0x15;
pub(super) const TAG_TYPEDEF: u16 = 0x16;
pub(super) const TAG_UNION_TYPE: u16 = 0x17;
pub(super) const TAG_UNSPECIFIED_PARAMETERS: u16 = 0x18;
pub(super) const TAG_SUBRANGE_TYPE: u16 = 0x21;
pub(super) const TAG_BASE_TYPE: u16 = 0x24;
pub(super) const TAG_CONST_TYPE: u16 = 0x26;
pub(super) const TAG_ENUMERATOR: u16 = 0x28;
pub(super) const TAG_SUBPROGRAM: u16 = 0x2E;
pub(super) const TAG_VARIABLE: u16 = 0x34;
pub(super) const TAG_VOLATILE_TYPE: u16 = 0x35;
pub(super) const TAG_RESTRICT_TYPE: u16 = 0x37;
pub(super) const TAG_UNSPECIFIED_TYPE: u16 = 0x3B;
pub(super) const TAG_ATOMIC_TYPE: u16 = 0x47;

// Attributes, DWARF 5 §7.5.4, Table 7.5, and GNU's vector flag.
pub(super) const AT_LOCATION: u16 = 0x02;
pub(super) const AT_NAME: u16 = 0x03;
pub(super) const AT_BYTE_SIZE: u16 = 0x0B;
pub(super) const AT_STMT_LIST: u16 = 0x10;
pub(super) const AT_LOW_PC: u16 = 0x11;
pub(super) const AT_HIGH_PC: u16 = 0x12;
pub(super) const AT_LANGUAGE: u16 = 0x13;
pub(super) const AT_COMP_DIR: u16 = 0x1B;
pub(super) const AT_CONST_VALUE: u16 = 0x1C;
pub(super) const AT_PRODUCER: u16 = 0x25;
pub(super) const AT_PROTOTYPED: u16 = 0x27;
pub(super) const AT_COUNT: u16 = 0x37;
pub(super) const AT_DATA_MEMBER_LOCATION: u16 = 0x38;
pub(super) const AT_DECL_FILE: u16 = 0x3A;
pub(super) const AT_DECL_LINE: u16 = 0x3B;
pub(super) const AT_DECLARATION: u16 = 0x3C;
pub(super) const AT_ENCODING: u16 = 0x3E;
pub(super) const AT_EXTERNAL: u16 = 0x3F;
pub(super) const AT_FRAME_BASE: u16 = 0x40;
pub(super) const AT_TYPE: u16 = 0x49;
pub(super) const AT_RANGES: u16 = 0x55;
pub(super) const AT_DATA_BIT_OFFSET: u16 = 0x6B;
pub(super) const AT_LINKAGE_NAME: u16 = 0x6E;
pub(super) const AT_BIT_SIZE: u16 = 0x0D;
pub(super) const AT_GNU_VECTOR: u16 = 0x2107;

// Forms, DWARF 5 §7.5.6, Table 7.6.
const FORM_ADDR: u8 = 0x01;
const FORM_DATA2: u8 = 0x05;
const FORM_DATA4: u8 = 0x06;
const FORM_STRING: u8 = 0x08;
const FORM_DATA1: u8 = 0x0B;
const FORM_SDATA: u8 = 0x0D;
const FORM_UDATA: u8 = 0x0F;
const FORM_REF4: u8 = 0x13;
const FORM_SEC_OFFSET: u8 = 0x17;
const FORM_EXPRLOC: u8 = 0x18;
const FORM_FLAG_PRESENT: u8 = 0x19;

/// The label of the compilation unit's header, which `DW_FORM_ref4`
/// offsets count from (§7.5.5).
pub(super) const UNIT: &str = ".Ldebug_info";

/// The entries of `.debug_info` as they are written, and their
/// abbreviations.
pub(super) struct Entries<'d> {
    arena:  &'d Bump,
    /// The shapes, whose codes count from 1.
    shapes: ArenaVec<'d, Shape<'d>>,
    pub(super) text: ArenaString<'d>,
}

/// An abbreviation (§7.5.3).
struct Shape<'d> {
    tag:        u16,
    children:   bool,
    attributes: ArenaVec<'d, (u16, u8)>,
}

/// One entry as it is built: its shape and its values' directives.
pub(super) struct Entry<'d> {
    shape:  Shape<'d>,
    values: ArenaString<'d>,
}

impl<'d> Entries<'d> {
    pub(super) fn new(arena: &'d Bump) -> Self {
        Self {
            arena,
            shapes: ArenaVec::new_in(arena),
            text: ArenaString::new_in(arena),
        }
    }

    /// A new entry with `tag`, which has children if `children`.
    pub(super) fn entry(&self, tag: u16, children: bool) -> Entry<'d> {
        Entry {
            shape:  Shape {
                tag,
                children,
                attributes: ArenaVec::new_in(self.arena),
            },
            values: ArenaString::new_in(self.arena),
        }
    }

    /// Writes `entry`, at `label` if it has one; its children follow until
    /// [`Self::end_children`].
    pub(super) fn write(&mut self, entry: Entry<'d>, label: Option<&dyn fmt::Display>) -> fmt::Result {
        let code = match self.shapes.iter().position(|shape| {
            shape.tag == entry.shape.tag
                && shape.children == entry.shape.children
                && shape.attributes == entry.shape.attributes
        }) {
            | Some(at) => at + 1,
            | None => {
                self.shapes.push(entry.shape);
                self.shapes.len()
            },
        };
        if let Some(label) = label {
            writeln!(self.text, "{label}:")?;
        }
        writeln!(self.text, "\t.uleb128\t{code}")?;
        self.text.push_str(&entry.values);
        Ok(())
    }

    /// The end of the children of the entry written last that has them.
    pub(super) fn end_children(&mut self) -> fmt::Result {
        writeln!(self.text, "\t.byte\t0")
    }

    /// The contents of `.debug_abbrev`.
    pub(super) fn abbreviations(&self, out: &mut dyn Write) -> fmt::Result {
        for (index, shape) in self.shapes.iter().enumerate() {
            writeln!(out, "\t.uleb128\t{}", index + 1)?;
            writeln!(out, "\t.uleb128\t{}", shape.tag)?;
            writeln!(out, "\t.byte\t{}", u8::from(shape.children))?;
            for &(attribute, form) in &shape.attributes {
                writeln!(out, "\t.uleb128\t{attribute}\n\t.uleb128\t{form}")?;
            }
            writeln!(out, "\t.byte\t0\n\t.byte\t0")?;
        }
        writeln!(out, "\t.byte\t0")
    }
}

impl Entry<'_> {
    fn add(&mut self, attribute: u16, form: u8) {
        self.shape.attributes.push((attribute, form));
    }

    /// A string in the entry (`DW_FORM_string`).
    pub(super) fn string(&mut self, attribute: u16, value: &str) -> fmt::Result {
        self.add(attribute, FORM_STRING);
        writeln!(self.values, "\t.asciz\t{}", super::lines::Quoted(value))
    }

    /// A one-byte constant.
    pub(super) fn byte(&mut self, attribute: u16, value: u8) -> fmt::Result {
        self.add(attribute, FORM_DATA1);
        writeln!(self.values, "\t.byte\t{value}")
    }

    /// A two-byte constant.
    pub(super) fn short(&mut self, attribute: u16, value: u16) -> fmt::Result {
        self.add(attribute, FORM_DATA2);
        writeln!(self.values, "\t.short\t{value}")
    }

    /// An unsigned constant (`DW_FORM_udata`).
    pub(super) fn unsigned(&mut self, attribute: u16, value: u64) -> fmt::Result {
        self.add(attribute, FORM_UDATA);
        writeln!(self.values, "\t.uleb128\t{value}")
    }

    /// A signed constant (`DW_FORM_sdata`).
    pub(super) fn signed(&mut self, attribute: u16, value: i64) -> fmt::Result {
        self.add(attribute, FORM_SDATA);
        writeln!(self.values, "\t.sleb128\t{value}")
    }

    /// A flag that is set by being there.
    pub(super) fn flag(&mut self, attribute: u16) {
        self.add(attribute, FORM_FLAG_PRESENT);
    }

    /// The entry at `label` in this unit (`DW_FORM_ref4`).
    pub(super) fn reference(&mut self, attribute: u16, label: &dyn fmt::Display) -> fmt::Result {
        self.add(attribute, FORM_REF4);
        writeln!(self.values, "\t.long\t{label}-{UNIT}")
    }

    /// The address `symbol` names (`DW_FORM_addr`).
    pub(super) fn address(&mut self, attribute: u16, symbol: &dyn fmt::Display) -> fmt::Result {
        self.add(attribute, FORM_ADDR);
        writeln!(self.values, "\t.quad\t{symbol}")
    }

    /// The distance from `begin` to `end` (`DW_FORM_data4`), which
    /// `DW_AT_high_pc` gives as the size of the code (§2.17.2).
    pub(super) fn length(
        &mut self,
        attribute: u16,
        end: &dyn fmt::Display,
        begin: &dyn fmt::Display,
    ) -> fmt::Result {
        self.add(attribute, FORM_DATA4);
        writeln!(self.values, "\t.long\t{end}-{begin}")
    }

    /// The offset of `label` in its section (`DW_FORM_sec_offset`).
    pub(super) fn offset(&mut self, attribute: u16, label: &dyn fmt::Display) -> fmt::Result {
        self.add(attribute, FORM_SEC_OFFSET);
        writeln!(self.values, "\t.long\t{label}")
    }

    /// A location expression of `bytes` (`DW_FORM_exprloc`), then the
    /// address `symbol` names if there is one, which the expression's last
    /// operation takes as its operand.
    pub(super) fn expression(
        &mut self,
        attribute: u16,
        bytes: &[u8],
        symbol: Option<&dyn fmt::Display>,
    ) -> fmt::Result {
        self.add(attribute, FORM_EXPRLOC);
        let length = bytes.len() + if symbol.is_some() { 8 } else { 0 };
        write!(self.values, "\t.uleb128\t{length}\n\t.byte\t")?;
        for (index, byte) in bytes.iter().enumerate() {
            let separator = if index == 0 { "" } else { "," };
            write!(self.values, "{separator}{byte}")?;
        }
        writeln!(self.values)?;
        if let Some(symbol) = symbol {
            writeln!(self.values, "\t.quad\t{symbol}")?;
        }
        Ok(())
    }
}
