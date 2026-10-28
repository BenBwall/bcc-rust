//! Linear memory (WebAssembly 2.0 §4.2.8): a byte array in 64 KiB pages,
//! little endian, every access bounds checked (§4.4.7).
//!
//! The bytes are shared until written: copying a machine for a checkpoint
//! shares its memory, and the first store afterwards copies it.

use std::rc::Rc;

use super::{
    Trap,
    TrapKind,
};
use crate::{
    instr::{
        Load,
        Store,
    },
    value::Value,
};

/// Bytes per page.
pub const PAGE: usize = 65_536;
/// The most pages the stepper lets a memory grow to (16 MiB), below the
/// format's own limit, so a runaway `memory.grow` cannot exhaust the
/// browser tab.
pub const MAX_PAGES: u32 = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Memory {
    /// Read through `Deref`; written only through [`Memory::bytes_mut`].
    pub bytes: Rc<Vec<u8>>,
    /// The declared maximum, in pages.
    pub max:   Option<u32>,
}

impl Memory {
    pub fn new(pages: u32, max: Option<u32>) -> Self {
        Self {
            bytes: Rc::new(vec![0; pages as usize * PAGE]),
            max,
        }
    }

    pub fn pages(&self) -> u32 {
        (self.bytes.len() / PAGE) as u32
    }

    /// `memory.grow`: the previous size in pages, or -1 when the memory
    /// cannot grow by `delta` pages.
    pub fn grow(&mut self, delta: u32) -> i32 {
        let previous = self.pages();
        let limit = self.max.unwrap_or(65_536).min(MAX_PAGES);
        match previous.checked_add(delta) {
            | Some(pages) if pages <= limit => {
                self.bytes_mut().resize(pages as usize * PAGE, 0);
                previous as i32
            },
            | _ => -1,
        }
    }

    /// The effective address `base + offset` of a `size`-byte access, if it
    /// is in bounds.
    fn range(&self, base: i32, offset: u32, size: u32) -> Result<(u64, usize), Trap> {
        let address = u64::from(base as u32) + u64::from(offset);
        self.check(address, u64::from(size))?;
        Ok((address, address as usize))
    }

    fn check(&self, address: u64, size: u64) -> Result<(), Trap> {
        if address + size > self.bytes.len() as u64 {
            return Err(Trap::new(
                TrapKind::MemoryOutOfBounds,
                format!(
                    "out of bounds memory access: {size} byte{} at address {address} \
                     (0x{address:x}), but memory ends at {} (0x{:x})",
                    if size == 1 { "" } else { "s" },
                    self.bytes.len(),
                    self.bytes.len()
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn load(&self, load: Load, base: i32, offset: u32) -> Result<(u64, Value), Trap> {
        let size = load.size();
        let (address, start) = self.range(base, offset, size)?;
        let mut raw = [0_u8; 8];
        raw[..size as usize].copy_from_slice(&self.bytes[start..start + size as usize]);
        let bits = u64::from_le_bytes(raw);
        // Sign-extend from the loaded width.
        let shift = 64 - 8 * size;
        let signed = ((bits << shift) as i64) >> shift;
        let value = match load {
            | Load::F32 => Value::F32(bits as u32),
            | Load::F64 => Value::F64(bits),
            | _ if load.ty() == crate::module::ValType::I32 => Value::I32(if load.signed() {
                signed as i32
            } else {
                bits as i32
            }),
            | _ => Value::I64(if load.signed() { signed } else { bits as i64 }),
        };
        Ok((address, value))
    }

    pub(crate) fn store(
        &mut self,
        store: Store,
        base: i32,
        offset: u32,
        value: Value,
    ) -> Result<u64, Trap> {
        let size = store.size() as usize;
        let (address, start) = self.range(base, offset, store.size())?;
        let bits = match value {
            | Value::I32(value) => u64::from(value as u32),
            | Value::I64(value) => value as u64,
            | Value::F32(bits) => u64::from(bits),
            | Value::F64(bits) => bits,
            | Value::FuncRef(_) | Value::ExternRef(_) =>
                return Err(Trap::new(
                    TrapKind::Internal,
                    "stepper error: stored a reference",
                )),
        };
        self.bytes_mut()[start..start + size].copy_from_slice(&bits.to_le_bytes()[..size]);
        Ok(address)
    }

    /// `memory.copy`, which may overlap.
    pub(crate) fn copy(&mut self, destination: u32, source: u32, length: u32) -> Result<(), Trap> {
        self.check(u64::from(source), u64::from(length))?;
        self.check(u64::from(destination), u64::from(length))?;
        let source = source as usize;
        self.bytes_mut()
            .copy_within(source..source + length as usize, destination as usize);
        Ok(())
    }

    pub(crate) fn fill(&mut self, destination: u32, value: u8, length: u32) -> Result<(), Trap> {
        self.check(u64::from(destination), u64::from(length))?;
        let start = destination as usize;
        self.bytes_mut()[start..start + length as usize].fill(value);
        Ok(())
    }

    pub(crate) fn write(&mut self, destination: u32, bytes: &[u8]) -> Result<(), Trap> {
        self.check(u64::from(destination), bytes.len() as u64)?;
        let start = destination as usize;
        self.bytes_mut()[start..start + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    pub(crate) fn read(&self, address: u32, length: u32) -> Result<&[u8], Trap> {
        self.check(u64::from(address), u64::from(length))?;
        let start = address as usize;
        Ok(&self.bytes[start..start + length as usize])
    }

    /// `length` writable bytes at `address`, if they are in bounds.
    pub(crate) fn slice_mut(&mut self, address: u32, length: u32) -> Result<&mut [u8], Trap> {
        self.check(u64::from(address), u64::from(length))?;
        let start = address as usize;
        Ok(&mut self.bytes_mut()[start..start + length as usize])
    }

    /// The bytes, unshared from any checkpoint first.
    fn bytes_mut(&mut self) -> &mut Vec<u8> {
        Rc::make_mut(&mut self.bytes)
    }

    /// Whether this memory still shares its bytes with `other`.
    pub fn shares(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.bytes, &other.bytes)
    }

    pub(crate) fn u32_at(&self, address: u32) -> Result<u32, Trap> {
        let bytes = self.read(address, 4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn set_u32(&mut self, address: u32, value: u32) -> Result<(), Trap> {
        self.write(address, &value.to_le_bytes())
    }

    pub(crate) fn set_u64(&mut self, address: u32, value: u64) -> Result<(), Trap> {
        self.write(address, &value.to_le_bytes())
    }
}
