//! Monotone branch relaxation, final section contents and local fixups.

use super::{
    Error,
    Kind,
    Object,
    Reloc,
    Result,
    Section,
    Symbol,
    encoder,
    listing,
};
use crate::util::bump::ArenaVec;

impl<'a> Object<'a> {
    /// Lays out every section as LLVM MC does (`MCAssembler::layout`): one
    /// pass with every local branch short, then relaxation passes until no
    /// offset changes.
    pub(super) fn layout(&mut self) -> Result<()> {
        // The item that defines each label: a branch's target is ahead of it
        // or behind it.
        let mut defined_at = ArenaVec::with_capacity_in(self.symbols.len(), self.arena);
        defined_at.resize(self.symbols.len(), usize::MAX);
        for (index, item) in self.items.iter().enumerate() {
            if let Kind::Label(id) = item.kind {
                defined_at[id] = index;
            }
        }
        _ = self.pass(&defined_at, false)?;
        while self.pass(&defined_at, true)? {}
        // Alias chains may be forward and must remain iterative.
        for _ in 0..self.symbols.len() {
            let mut changed = false;
            for id in 0..self.symbols.len() {
                if let Some(target) = self.symbols[id].alias {
                    let to = self.symbols[target];
                    if self.symbols[id].section != to.section || self.symbols[id].value != to.value
                    {
                        self.symbols[id].section = to.section;
                        self.symbols[id].value = to.value;
                        changed = true;
                    }
                    self.symbols[id].size = to.size;
                    if !self.coff && self.symbols[id].kind == 0 {
                        self.symbols[id].kind = to.kind;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        if self
            .symbols
            .iter()
            .any(|s| s.alias.is_some() && s.section.is_none())
        {
            return Err(Error("unresolved or cyclic alias"));
        }
        Ok(())
    }

    /// One pass of MC's fused relaxation and layout (`relaxOnce`): each item
    /// takes its offset from the items before it in this pass, alignment
    /// padding follows from that offset, and with `relax` a short branch
    /// that cannot reach its target becomes near at once. The distance to a
    /// target behind the branch uses this pass's offsets. A target ahead of
    /// it still has the previous pass's offset, so the distance uses the
    /// branch's previous offset as well, which is what MC's `Stretch`
    /// correction computes. Returns whether any offset changed.
    fn pass(&mut self, defined_at: &[usize], relax: bool) -> Result<bool> {
        let Self {
            coff,
            sections,
            symbols,
            items,
            ..
        } = self;
        for section in sections.iter_mut() {
            section.size = 0;
        }
        let mut changed = false;
        for (index, item) in items.iter_mut().enumerate() {
            let section = &mut sections[item.section];
            let previous = item.offset;
            item.offset = section.size;
            changed |= item.offset != previous;
            let size = match &mut item.kind {
                | Kind::Label(id) => {
                    symbols[*id].section = Some(item.section);
                    symbols[*id].value = item.offset;
                    if section.flags & 0x400 != 0 {
                        symbols[*id].kind = 6;
                    }
                    0
                },
                | Kind::Size(id, size) => {
                    symbols[*id].size = if let Some(size) = size {
                        *size
                    } else {
                        if symbols[*id].section != Some(item.section) {
                            return Err(Error("symbol size outside its section"));
                        }
                        item.offset
                            .checked_sub(symbols[*id].value)
                            .ok_or(Error("negative symbol size"))?
                    };
                    0
                },
                | Kind::Align { align, max, .. } => padding(item.offset, *align, *max),
                | Kind::Zero(count) => *count,
                | Kind::Bytes(bytes) => bytes.len() as u64,
                | Kind::Code(code) => u64::from(code.len),
                | Kind::Value(width, _) | Kind::CoffValue(width, _, _) => u64::from(*width),
                | Kind::Branch {
                    condition,
                    target,
                    near,
                } => {
                    if relax && !*near {
                        let symbol = symbols[*target];
                        if symbol.binding == 0 && symbol.alias.is_some() {
                            return Err(Error("unsupported branch to local alias"));
                        }
                        let local =
                            resolvable(*coff, symbol) && symbol.section == Some(item.section);
                        let from = if defined_at[*target] > index {
                            previous
                        } else {
                            item.offset
                        };
                        let distance = i128::from(symbol.value) - i128::from(from + 2);
                        if !local || i8::try_from(distance).is_err() {
                            *near = true;
                        }
                    }
                    if *near {
                        if condition.is_some() { 6 } else { 5 }
                    } else {
                        2
                    }
                },
                | Kind::Cfi(..) => 0,
            };
            section.size = section
                .size
                .checked_add(size)
                .ok_or(Error("section size overflow"))?;
        }
        Ok(changed)
    }

    pub(super) fn materialize(&mut self) -> Result<()> {
        for section in &mut self.sections {
            if section.kind != 8 {
                section.data.resize(
                    usize::try_from(section.size).map_err(|_| Error("section too large"))?,
                    0,
                );
            }
        }
        for index in 0..self.items.len() {
            let item = &self.items[index];
            let section = item.section;
            let offset = item.offset;
            match &item.kind {
                | Kind::Bytes(bytes) => {
                    if self.sections[section].kind == 8 {
                        return Err(Error("string in a NOBITS section"));
                    }
                    let start = usize::try_from(offset).map_err(|_| Error("section too large"))?;
                    self.sections[section].data[start..start + bytes.len()]
                        .copy_from_slice(bytes.as_bytes());
                },
                | Kind::Code(code) => {
                    if self.sections[section].kind == 8 {
                        return Err(Error("instruction in a NOBITS section"));
                    }
                    let code = *code;
                    let start = usize::try_from(offset).map_err(|_| Error("section too large"))?;
                    self.sections[section].data[start..start + usize::from(code.len)]
                        .copy_from_slice(&code.bytes[..usize::from(code.len)]);
                    if let Some(fixup) = code.fixup {
                        self.fixup(
                            section,
                            offset + u64::from(fixup.at),
                            4,
                            fixup.expression,
                            fixup.kind,
                            fixup.bias,
                        )?;
                    }
                },
                | Kind::Value(width, expression) | Kind::CoffValue(width, expression, _) => {
                    let (width, expression) = (*width, *expression);
                    self.fixup(
                        section,
                        offset,
                        width,
                        expression,
                        if let Kind::CoffValue(_, _, kind) = item.kind {
                            kind
                        } else {
                            match width {
                                | 8 => 1,
                                | 4 => 10,
                                | 2 => 12,
                                | 1 => 14,
                                | _ => return Err(Error("unsupported data relocation width")),
                            }
                        },
                        0,
                    )?;
                },
                | Kind::Branch {
                    condition,
                    target,
                    near,
                } => {
                    if self.sections[section].kind == 8 {
                        return Err(Error("branch in a NOBITS section"));
                    }
                    let target = *target;
                    let local = resolvable(self.coff, self.symbols[target])
                        && self.symbols[target].section == Some(section);
                    let mut code = [0; 6];
                    let len = if *near {
                        let at = if let Some(condition) = condition {
                            code[0] = 0x0F;
                            code[1] = 0x80 + condition;
                            2
                        } else {
                            code[0] = 0xE9;
                            1
                        };
                        let len = at + 4;
                        let distance = if local {
                            i128::from(self.symbols[target].value) - i128::from(offset + len as u64)
                        } else {
                            0
                        };
                        code[at..len].copy_from_slice(
                            &i32::try_from(distance)
                                .map_err(|_| Error("branch exceeds rel32"))?
                                .to_le_bytes(),
                        );
                        len
                    } else {
                        code[0] = condition.map_or(0xEB, |cond| 0x70 + cond);
                        code[1] = i8::try_from(
                            i128::from(self.symbols[target].value) - i128::from(offset + 2),
                        )
                        .map_err(|_| Error("branch exceeds rel8"))?
                        .to_le_bytes()[0];
                        2
                    };
                    let start = usize::try_from(offset).map_err(|_| Error("section too large"))?;
                    self.sections[section].data[start..start + len].copy_from_slice(&code[..len]);
                    if !local {
                        self.sections[section].relocs.push(Reloc {
                            offset: offset + len as u64 - 4,
                            symbol: target,
                            kind:   4,
                            addend: -4,
                            named:  false,
                        });
                    }
                },
                | Kind::Align { align, fill, max } => {
                    let count = padding(offset, *align, *max);
                    if let Some(fill) = fill {
                        if self.sections[section].kind == 8 && *fill != 0 {
                            return Err(Error("nonzero alignment in NOBITS section"));
                        }
                        if self.sections[section].kind != 8 {
                            let start =
                                usize::try_from(offset).map_err(|_| Error("section too large"))?;
                            let end = usize::try_from(offset + count)
                                .map_err(|_| Error("section too large"))?;
                            self.sections[section].data[start..end].fill(*fill);
                        }
                    } else if self.sections[section].kind != 8
                        && self.sections[section].flags & 4 != 0
                    {
                        encoder::nops(&mut self.sections[section].data, offset, count)?;
                    }
                },
                | _ => {},
            }
        }
        Ok(())
    }

    fn fixup(
        &mut self,
        section: usize,
        offset: u64,
        width: u8,
        expression: &'a str,
        kind: u32,
        bias: i64,
    ) -> Result<()> {
        if !self.coff && kind >= 0x0001_0000 {
            return Err(Error("COFF relocation in ELF listing"));
        }
        let explicit_plt = expression.ends_with("@PLT") && kind == 4;
        let expression = listing::expression(if explicit_plt {
            expression
                .strip_suffix("@PLT")
                .ok_or(Error("missing PLT suffix"))?
        } else {
            expression
        })?;
        let mut value = i128::from(expression.addend) + i128::from(bias);
        if let Some(name) = expression.symbol {
            let id = self.symbol(name);
            let symbol = self.symbols[id];
            if symbol.binding == 0 && symbol.section.is_some() && matches!(kind, 9 | 41 | 42) {
                return Err(Error("unsupported GOT reference to local symbol"));
            }
            if matches!(kind, 22 | 23) {
                self.symbols[id].kind = 6;
            }
            if let Some(base) = expression.subtract {
                let base_id = self.symbol(base);
                let base = self.symbols[base_id];
                if symbol.section.is_none() || symbol.section != base.section {
                    return Err(Error("label difference outside one section"));
                }
                value += i128::from(symbol.value) - i128::from(base.value);
            } else if matches!(kind, 2 | 4)
                && resolvable(self.coff, symbol)
                && !explicit_plt
                && symbol.section == Some(section)
            {
                value += i128::from(symbol.value) - i128::from(offset);
                if i32::try_from(value).is_err() {
                    return Err(Error("PC-relative fixup exceeds rel32"));
                }
            } else {
                if self.sections[section].kind == 8 {
                    return Err(Error("relocation in a NOBITS section"));
                }
                self.sections[section].relocs.push(Reloc {
                    offset,
                    symbol: id,
                    kind,
                    addend: i64::try_from(value)
                        .map_err(|_| Error("relocation addend exceeds 64 bits"))?,
                    named: explicit_plt,
                });
                value = 0;
            }
        }
        if self.sections[section].kind == 8 {
            return if value == 0 {
                Ok(())
            } else {
                Err(Error("nonzero value in a NOBITS section"))
            };
        }
        let field_bits = u32::from(width) * 8;
        if value < -(1_i128 << (field_bits - 1)) || value >= (1_i128 << field_bits) {
            return Err(Error("data value or label difference exceeds field width"));
        }
        let at = usize::try_from(offset).map_err(|_| Error("section too large"))?;
        self.sections[section].data[at..at + usize::from(width)]
            .copy_from_slice(&value.to_le_bytes()[..usize::from(width)]);
        Ok(())
    }

    pub(super) fn symbol(&mut self, name: &'a str) -> usize {
        let name = listing::unquote(name);
        if let Some(&id) = self.names.get(name) {
            return id;
        }
        let id = self.symbols.len();
        self.symbols.push(Symbol {
            name,
            section: None,
            value: 0,
            size: 0,
            binding: 0,
            kind: 0,
            visibility: 0,
            alias: None,
        });
        _ = self.names.insert(name, id);
        id
    }

    pub(super) fn section(
        &mut self,
        name: &'a str,
        kind: u32,
        flags: u64,
        group: Option<usize>,
    ) -> Result<usize> {
        if let Some(id) = self
            .sections
            .iter()
            .position(|s| s.name == name && s.group == group)
        {
            if self.sections[id].kind != kind || self.sections[id].flags != flags {
                return Err(Error("inconsistent section attributes"));
            }
            return Ok(id);
        }
        let id = self.sections.len();
        self.sections.push(Section {
            name,
            kind,
            flags,
            align: if name == ".text" { 4 } else { 1 },
            size: 0,
            group,
            data: ArenaVec::new_in(self.arena),
            relocs: ArenaVec::new_in(self.arena),
        });
        Ok(id)
    }
}

/// The padding an alignment item at `offset` emits: up to the next multiple
/// of `align`, or nothing if that takes more than `max` bytes, as
/// `MCAssembler::relaxAlign` computes it (x86's smallest `nop` is one byte,
/// so code alignment needs no rounding).
fn padding(offset: u64, align: u64, max: u64) -> u64 {
    let count = offset.wrapping_neg() & (align - 1);
    if count > max { 0 } else { count }
}

// COFF function-typed and weak symbols retain linker fixups even within a
// section; ordinary labels (including globals) can be resolved by the
// assembler.
fn resolvable(coff: bool, symbol: Symbol<'_>) -> bool {
    if coff {
        symbol.binding != 2 && symbol.kind != 2
    } else {
        symbol.binding == 0
    }
}
