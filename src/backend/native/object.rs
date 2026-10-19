//! In-process object emission for the native emitter's private vocabulary.
//!
//! `encode` reads the emitter's listing once, lays out sections while promoting
//! out-of-range branches, resolves local fixups, then writes ELF64. Sharing
//! this vocabulary keeps prologues, CFI and symbol attributes in `emit.rs`,
//! without coupling the encoder to register allocation. This is deliberately
//! not a general-purpose assembler: unknown forms fail before the caller writes
//! output.
//!
//! Read `encode`, `listing::parse`, `encoder::instruction`, then `elf::write`.
//! `layout` resolves section offsets and fixups. `unwind` implements DWARF
//! call-frame instructions (LSB 5.0 §10.6); `elf` implements the ELF gABI and
//! AMD64 psABI §4.4 relocations.

mod elf;
mod encoder;
mod layout;
mod listing;
mod unwind;

use crate::util::bump::{
    ArenaMap,
    ArenaVec,
    Bump,
};

/// Encodes the private ELF listing, without invoking LLVM or an assembler.
pub(super) fn encode<'a>(text: &'a str, arena: &'a Bump) -> Result<ArenaVec<'a, u8>> {
    let mut object = listing::parse(text, arena)?;
    object.layout()?;
    object.materialize()?;
    unwind::generate(&mut object)?;
    elf::write(&object)
}

/// A closed failure path for vocabulary the encoder cannot represent.
#[derive(Debug, thiserror::Error)]
#[error("native object emission: {0}")]
pub(super) struct Error(pub(super) &'static str);

type Result<T> = std::result::Result<T, Error>;

struct Object<'a> {
    arena:    &'a Bump,
    sections: ArenaVec<'a, Section<'a>>,
    symbols:  ArenaVec<'a, Symbol<'a>>,
    names:    ArenaMap<'a, &'a str, usize>,
    items:    ArenaVec<'a, Item<'a>>,
}

struct Section<'a> {
    name:   &'a str,
    kind:   u32,
    flags:  u64,
    align:  u64,
    size:   u64,
    group:  Option<usize>,
    data:   ArenaVec<'a, u8>,
    relocs: ArenaVec<'a, Reloc>,
}

#[derive(Clone, Copy)]
struct Symbol<'a> {
    name:       &'a str,
    section:    Option<usize>,
    value:      u64,
    size:       u64,
    binding:    u8,
    kind:       u8,
    visibility: u8,
    alias:      Option<usize>,
}

struct Item<'a> {
    section: usize,
    offset:  u64,
    kind:    Kind<'a>,
}

enum Kind<'a> {
    Label(usize),
    Size(usize, Option<u64>),
    Align(u64, Option<u8>),
    Zero(u64),
    Code(encoder::Code<'a>),
    Value(u8, &'a str),
    Branch {
        condition: Option<u8>,
        target:    usize,
        near:      bool,
    },
    Cfi(&'a str, &'a str),
}

#[derive(Clone, Copy)]
struct Reloc {
    offset: u64,
    symbol: usize,
    kind:   u32,
    addend: i64,
    // An explicit @PLT request must retain the named local symbol.
    named:  bool,
}
