//! Section construction and format-specific unwind section conventions.

use std::fmt::Write as _;

use super::{
    super::{
        Object,
        Section,
    },
    Attributes,
    Coff,
    Error,
    Result,
};
use crate::util::bump::{
    ArenaMap,
    ArenaString,
    ArenaVec,
    Bump,
};

impl<'a> Coff<'a> {
    pub(super) fn new(arena: &'a Bump, gnu: bool) -> Self {
        Self {
            object: Object {
                arena,
                coff: true,
                sections: ArenaVec::new_in(arena),
                symbols: ArenaVec::new_in(arena),
                names: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
                items: ArenaVec::new_in(arena),
            },
            gnu,
            sections: ArenaVec::new_in(arena),
            named: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            unwind: ArenaVec::new_in(arena),
            unwound: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
        }
    }

    pub(super) fn section(
        &mut self,
        name: &'a str,
        characteristics: u32,
        group: Option<usize>,
        selection: u8,
    ) -> Result<usize> {
        if let Some(&id) = self.named.get(&(name, group)) {
            if self.sections[id].characteristics != characteristics
                || self.sections[id].selection != selection
            {
                return Err(Error("inconsistent COFF section attributes"));
            }
            return Ok(id);
        }
        Ok(self.append_section(name, characteristics, group, selection, None))
    }

    fn append_section(
        &mut self,
        name: &'a str,
        characteristics: u32,
        group: Option<usize>,
        selection: u8,
        associative: Option<usize>,
    ) -> usize {
        let id = self.object.sections.len();
        _ = self.named.entry((name, group)).or_insert(id);
        self.object.sections.push(Section {
            name,
            kind: if characteristics & 0x80 != 0 { 8 } else { 1 },
            flags: if characteristics & 0x20 != 0 { 4 } else { 0 },
            align: if matches!(name, ".text" | ".data" | ".bss") && group.is_none() {
                4
            } else {
                1
            },
            size: 0,
            group,
            data: ArenaVec::new_in(self.object.arena),
            relocs: ArenaVec::new_in(self.object.arena),
        });
        self.sections.push(Attributes {
            characteristics,
            selection,
            associative,
        });
        id
    }

    pub(super) fn unwind_section(&mut self, text: usize, name: &'static str) -> Result<usize> {
        let comdat = self.object.sections[text].group.is_some();
        if comdat && self.gnu {
            // MinGW uses independently discardable suffixed sections, with
            // no explicit key symbol. MSVC uses associative COMDAT sections.
            let suffix = self.object.sections[text]
                .name
                .split_once('$')
                .map_or("", |(_, s)| s);
            let mut section_name = ArenaString::new_in(self.object.arena);
            write!(&mut section_name, "{name}${suffix}")
                .map_err(|_| Error("COFF unwind section name formatting failed"))?;
            let id = self.section(section_name.into_str(), 0x4000_1040, None, 2)?;
            self.object.sections[id].align = 4;
            return Ok(id);
        }
        let id = self.append_section(
            name,
            0x4000_0040 | if comdat { 0x1000 } else { 0 },
            None,
            if comdat { 5 } else { 0 },
            comdat.then_some(text),
        );
        self.object.sections[id].align = 4;
        Ok(id)
    }
}
