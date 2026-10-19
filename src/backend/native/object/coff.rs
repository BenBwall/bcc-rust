//! AMD64 PE/COFF objects from the native Windows listing (Microsoft PE format).
//!
//! `encode` shares instruction layout with ELF; `listing` owns COFF directives,
//! `sections` builds section metadata, `unwind` translates SEH event offsets,
//! and `writer` serializes section
//! headers, implicit-addend relocations, symbols and auxiliary records. No
//! external assembler or LLVM API participates in this path.

mod flags;
mod listing;
mod sections;
mod unwind;
mod writer;

use super::{
    Error,
    Object,
    Result,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

pub(super) fn encode<'a>(text: &'a str, gnu: bool, arena: &'a Bump) -> Result<ArenaVec<'a, u8>> {
    let mut coff = listing::parse(text, gnu, arena)?;
    coff.object.layout()?;
    if coff
        .object
        .sections
        .iter()
        .any(|s| s.size > u64::from(u32::MAX))
    {
        return Err(Error("COFF section exceeds 32 bits"));
    }
    coff.object.materialize()?;
    unwind::generate(&mut coff)?;
    writer::write(&coff)
}

struct Coff<'a> {
    object:   Object<'a>,
    gnu:      bool,
    sections: ArenaVec<'a, Attributes>,
    unwind:   ArenaVec<'a, UnwindSections>,
}

#[derive(Clone, Copy)]
struct Attributes {
    characteristics: u32,
    selection:       u8,
    associative:     Option<usize>,
}

struct UnwindSections {
    text:  usize,
    xdata: Option<usize>,
    pdata: Option<usize>,
}
