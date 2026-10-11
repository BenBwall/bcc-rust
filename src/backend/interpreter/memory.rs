//! Byte-addressed memory with provenance.
//!
//! Memory is a set of objects: stack slots, globals, heap allocations,
//! functions (size zero, so a function's address is unique) and `va_list`
//! cursors. Each object has a fresh base address, a size and an array of
//! cells, one per byte. A cell holds the byte, whether it was ever written
//! with a defined value, and, for the eight bytes of a stored pointer, which
//! fragment of which pointer it is. Loading a pointer whose eight cells are
//! the fragments of one pointer in order gives that pointer back with its
//! provenance; loading it from other bytes treats them as an integer
//! address, as `inttoptr` does.
//!
//! A [`Pointer`] is an address with an optional [`ObjectRef`]. An access
//! through it checks, in order: that it has provenance (null and wild
//! addresses have none), that the object is still live, that the object
//! holds data, that the access lies within the object, that the address is
//! aligned, and for writes that the object is not a constant global.
//!
//! `inttoptr` maps an address back to the live object whose range
//! `[base, base + size]` contains it, if any; the ranges are separated by
//! gaps, so the mapping is unambiguous. This is a deterministic stand-in for
//! a full provenance model: a pointer made from an integer may access the
//! object its address falls in, even if the integer was not derived from
//! that object's address.
//!
//! Freed object records and their cells are reused for later allocations of
//! the same size, with the record's generation advanced, so a long-running
//! program's memory stays bounded by its peak use.

use std::num::NonZeroU32;

use super::{
    trap::{
        Fault,
        UbKind,
        Unsupported,
    },
    value::{
        ObjectRef,
        Pointer,
        RuntimeValue,
    },
};
use crate::{
    ir::{
        Align,
        FuncId,
        Type,
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// The interpreter's memory: every object a program can address.
pub(crate) struct Memory<'a> {
    arena:           &'a Bump,
    objects:         ArenaVec<'a, Object<'a>>,
    /// The base address and index of every live object, sorted by address.
    live:            ArenaVec<'a, (u64, u32)>,
    /// For each storage size, the first freed object of that size; the rest
    /// are chained through [`Object::next_free`].
    free:            ArenaMap<'a, usize, u32>,
    next_address:    u64,
    max_object_size: u64,
}

/// One object of memory.
struct Object<'a> {
    base:       u64,
    generation: u32,
    kind:       ObjectKind,
    cells:      &'a mut [Cell],
    next_free:  Option<NonZeroU32>,
    /// The state of a [`ObjectKind::VaCursor`]; unused for other kinds.
    cursor:     VaCursor,
}

/// What an object is, which decides how a program may use it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ObjectKind {
    /// A freed record waiting for reuse.
    Free,
    /// A function's stack slot, freed when it returns.
    Stack,
    /// A global that the program may write.
    Global,
    /// A `constant` global, such as a string literal; writing it is
    /// undefined.
    ConstantGlobal,
    /// A `malloc` or `calloc` allocation.
    Heap,
    /// A function, whose address `func_addr` takes. It holds no data.
    Function(FuncId),
    /// A `va_list` cursor, which `va_start` and `va_copy` create. Its state
    /// is the object's [`VaCursor`].
    VaCursor,
}

/// The state of a `va_list`: the variadic arguments of one activation of a
/// function, and how many `va_arg` has taken.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) struct VaCursor {
    /// The depth of the frame whose arguments these are.
    pub(crate) depth:      u32,
    /// That frame's activation number, which no other call reuses.
    pub(crate) activation: u64,
    /// How many arguments `va_arg` has taken.
    pub(crate) position:   u32,
}

/// One byte of an object.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Cell {
    /// For a pointer fragment, the provenance of the stored pointer.
    provenance: Option<ObjectRef>,
    byte:       u8,
    /// [`Cell::POISON_TAG`], [`Cell::DATA_TAG`], or [`Cell::FRAGMENT_TAG`]
    /// plus the fragment's index in its pointer.
    tag:        u8,
}

impl Cell {
    /// A defined byte of data.
    const DATA_TAG: u8 = 1;
    /// The first byte of a stored pointer; the others follow in order.
    const FRAGMENT_TAG: u8 = 2;
    const POISON: Self = Self {
        provenance: None,
        byte:       0,
        tag:        Self::POISON_TAG,
    };
    /// A byte never written, or written with poison.
    const POISON_TAG: u8 = 0;

    const fn data(byte: u8) -> Self {
        Self {
            provenance: None,
            byte,
            tag: Self::DATA_TAG,
        }
    }
}

/// The gap left after every object, so one object's one-past-the-end address
/// is never another's base.
const OBJECT_GAP: u64 = 16;
/// The least alignment of a base address.
const BASE_ALIGN: u64 = 16;
/// The first base address; lower addresses, null among them, name nothing.
const FIRST_ADDRESS: u64 = 0x1_0000;

impl<'a> Memory<'a> {
    /// Loads a value of type `ty` from `address`.
    pub(crate) fn load(
        &self,
        ty: Type,
        address: RuntimeValue,
        align: Align,
    ) -> Result<RuntimeValue, Fault> {
        let size = stored_size(ty)?;
        let (index, offset) = self.resolve(address, u64::from(size), align, false)?;
        let cells = &self.objects[index].cells[offset..offset + size as usize];
        if cells.iter().any(|cell| cell.tag == Cell::POISON_TAG) {
            return Ok(RuntimeValue::Poison);
        }
        let bits = cells
            .iter()
            .rev()
            .fold(0_u128, |bits, cell| (bits << 8) | u128::from(cell.byte));
        Ok(match ty {
            | Type::I1 if bits > 1 => RuntimeValue::Poison,
            | Type::F32 => RuntimeValue::F32(f32::from_bits(truncate_u32(bits))),
            | Type::F64 => RuntimeValue::F64(f64::from_bits(truncate_u64(bits))),
            | Type::Ptr => {
                let provenance = cells[0].provenance;
                let whole = cells.iter().enumerate().all(|(index, cell)| {
                    usize::from(cell.tag) == usize::from(Cell::FRAGMENT_TAG) + index
                        && cell.provenance == provenance
                });
                let address = truncate_u64(bits);
                RuntimeValue::Ptr(if whole && provenance.is_some() {
                    Pointer {
                        address,
                        provenance,
                    }
                } else {
                    self.pointer_from_address(address)
                })
            },
            | _ => RuntimeValue::Int(bits),
        })
    }

    /// Stores `value` of type `ty` at `address`. Storing poison makes the
    /// bytes poison.
    pub(crate) fn store(
        &mut self,
        ty: Type,
        value: RuntimeValue,
        address: RuntimeValue,
        align: Align,
    ) -> Result<(), Fault> {
        let size = stored_size(ty)?;
        let (index, offset) = self.resolve(address, u64::from(size), align, true)?;
        let cells = &mut self.objects[index].cells[offset..offset + size as usize];
        let (bits, provenance) = match value {
            | RuntimeValue::Poison => {
                cells.fill(Cell::POISON);
                return Ok(());
            },
            | RuntimeValue::Int(bits) => (bits, None),
            | RuntimeValue::F32(value) => (u128::from(value.to_bits()), None),
            | RuntimeValue::F64(value) => (u128::from(value.to_bits()), None),
            | RuntimeValue::Ptr(pointer) => (u128::from(pointer.address), Some(pointer)),
        };
        for (position, cell) in cells.iter_mut().enumerate() {
            let byte = truncate_u8(bits >> (8 * position));
            *cell = match provenance {
                | Some(pointer) => Cell {
                    provenance: pointer.provenance,
                    byte,
                    tag: Cell::FRAGMENT_TAG + truncate_u8(position as u128),
                },
                | None => Cell::data(byte),
            };
        }
        Ok(())
    }

    /// Copies `size` bytes from `source` to `destination`, keeping poison
    /// and pointer provenance. Overlap is undefined unless `may_overlap`.
    pub(crate) fn copy(
        &mut self,
        destination: RuntimeValue,
        source: RuntimeValue,
        size: RuntimeValue,
        align: Align,
        may_overlap: bool,
    ) -> Result<(), Fault> {
        let size = defined_size(size)?;
        if size == 0 {
            return Ok(());
        }
        let (from, from_offset) = self.resolve(source, size, align, false)?;
        let (to, to_offset) = self.resolve(destination, size, align, true)?;
        let len = usize::try_from(size).map_err(|_| UbKind::OutOfBounds)?;
        if from == to {
            if !may_overlap && from_offset < to_offset + len && to_offset < from_offset + len {
                return Err(UbKind::OverlappingCopy.into());
            }
            self.objects[to]
                .cells
                .copy_within(from_offset..from_offset + len, to_offset);
        } else {
            let [source, destination] = self
                .objects
                .get_disjoint_mut([from, to])
                .expect("distinct objects are disjoint");
            destination.cells[to_offset..to_offset + len]
                .copy_from_slice(&source.cells[from_offset..from_offset + len]);
        }
        Ok(())
    }

    /// Sets `size` bytes at `destination` to `byte`, or to poison if `byte`
    /// is poison.
    pub(crate) fn fill(
        &mut self,
        destination: RuntimeValue,
        byte: RuntimeValue,
        size: RuntimeValue,
        align: Align,
    ) -> Result<(), Fault> {
        let size = defined_size(size)?;
        if size == 0 {
            return Ok(());
        }
        let (index, offset) = self.resolve(destination, size, align, true)?;
        let cell = match byte {
            | RuntimeValue::Int(bits) => Cell::data(truncate_u8(bits)),
            | _ => Cell::POISON,
        };
        let len = usize::try_from(size).map_err(|_| UbKind::OutOfBounds)?;
        self.objects[index].cells[offset..offset + len].fill(cell);
        Ok(())
    }

    /// `ptr_add`: offsets `base` by `offset` bytes. With `inbounds`, the
    /// result is poison unless the base and the result both lie within one
    /// live object, at most one past its end, or the offset is zero.
    ///
    /// C99: §6.5.6 paragraph 8, p. 83; PDF p. 95.
    pub(crate) fn ptr_add(
        &self,
        base: RuntimeValue,
        offset: RuntimeValue,
        inbounds: bool,
    ) -> RuntimeValue {
        let (RuntimeValue::Ptr(pointer), RuntimeValue::Int(offset)) = (base, offset) else {
            return RuntimeValue::Poison;
        };
        let offset = truncate_u64(offset) as i64;
        let address = pointer.address.wrapping_add_signed(offset);
        let result = RuntimeValue::Ptr(Pointer {
            address,
            provenance: pointer.provenance,
        });
        if !inbounds || offset == 0 {
            return result;
        }
        let Some(object) = pointer
            .provenance
            .and_then(|provenance| self.live_object(provenance))
        else {
            return RuntimeValue::Poison;
        };
        let end = object.base + object.cells.len() as u64;
        let within = |address: u64| (object.base..=end).contains(&address);
        if within(pointer.address)
            && within(address)
            && pointer.address.checked_add_signed(offset).is_some()
        {
            result
        } else {
            RuntimeValue::Poison
        }
    }

    /// The pointer `inttoptr` makes from `address`: with the provenance of
    /// the live object whose range contains it, or none.
    pub(crate) fn pointer_from_address(&self, address: u64) -> Pointer {
        let after = self.live.partition_point(|&(base, _)| base <= address);
        let provenance = after.checked_sub(1).and_then(|position| {
            let (base, index) = self.live[position];
            let object = &self.objects[index as usize];
            (address - base <= object.cells.len() as u64)
                .then(|| ObjectRef::new(index as usize, object.generation))
        });
        Pointer {
            address,
            provenance,
        }
    }

    /// Checks an access of `size` bytes through `address` and returns the
    /// object's index and the offset in it.
    fn resolve(
        &self,
        address: RuntimeValue,
        size: u64,
        align: Align,
        write: bool,
    ) -> Result<(usize, usize), Fault> {
        let RuntimeValue::Ptr(pointer) = address else {
            return Err(UbKind::PoisonAddress.into());
        };
        let Some(provenance) = pointer.provenance else {
            return Err(if pointer.address == 0 {
                UbKind::NullDereference
            } else {
                UbKind::NoProvenance
            }
            .into());
        };
        let object = self
            .live_object(provenance)
            .ok_or(UbKind::DanglingPointer)?;
        match object.kind {
            | ObjectKind::Function(_) | ObjectKind::VaCursor => return Err(UbKind::NotData.into()),
            | ObjectKind::ConstantGlobal if write => return Err(UbKind::WriteToConstant.into()),
            | _ => {},
        }
        let offset = pointer.address.wrapping_sub(object.base);
        let len = object.cells.len() as u64;
        if offset > len || size > len - offset {
            return Err(UbKind::OutOfBounds.into());
        }
        if !pointer.address.is_multiple_of(align.bytes()) {
            return Err(UbKind::MisalignedAccess.into());
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "The offset lies within an object, whose cells are addressable."
        )]
        Ok((provenance.index(), offset as usize))
    }

    /// The object `provenance` names, if that allocation is still live.
    fn live_object(&self, provenance: ObjectRef) -> Option<&Object<'a>> {
        self.objects.get(provenance.index()).filter(|object| {
            object.generation == provenance.generation && object.kind != ObjectKind::Free
        })
    }

    /// The state of the live `va_list` cursor `provenance` names.
    pub(crate) fn cursor(&self, provenance: ObjectRef) -> Option<VaCursor> {
        self.live_object(provenance)
            .filter(|object| object.kind == ObjectKind::VaCursor)
            .map(|object| object.cursor)
    }

    /// Replaces the state of a live `va_list` cursor, to advance it.
    pub(crate) fn set_cursor(&mut self, provenance: ObjectRef, cursor: VaCursor) {
        let object = &mut self.objects[provenance.index()];
        debug_assert!(
            object.kind == ObjectKind::VaCursor,
            "only cursors have cursor state"
        );
        object.cursor = cursor;
    }

    /// The function a pointer is the address of.
    pub(crate) fn function_at(&self, callee: RuntimeValue) -> Result<FuncId, Fault> {
        let RuntimeValue::Ptr(pointer) = callee else {
            return Err(UbKind::CallThroughNonFunction.into());
        };
        let object = pointer
            .provenance
            .and_then(|provenance| self.live_object(provenance));
        match object {
            | Some(&Object {
                base,
                kind: ObjectKind::Function(func),
                ..
            }) if base == pointer.address => Ok(func),
            | _ => Err(UbKind::CallThroughNonFunction.into()),
        }
    }
}

impl Memory<'_> {
    /// Allocates an object of `size` poison bytes and returns a pointer to
    /// its start.
    pub(crate) fn allocate(
        &mut self,
        kind: ObjectKind,
        size: u64,
        align: Align,
    ) -> Result<Pointer, Fault> {
        if size > self.max_object_size {
            return Err(Fault::Unsupported(Unsupported::ObjectSize));
        }
        let len = usize::try_from(size).map_err(|_| Fault::Unsupported(Unsupported::ObjectSize))?;
        let alignment = align.bytes().max(BASE_ALIGN);
        let base = self
            .next_address
            .checked_next_multiple_of(alignment)
            .ok_or(Fault::Unsupported(Unsupported::AddressSpace))?;
        self.next_address = base
            .checked_add(size)
            .and_then(|end| end.checked_add(OBJECT_GAP))
            .ok_or(Fault::Unsupported(Unsupported::AddressSpace))?;
        let index = if let Some(&head) = self.free.get(&len) {
            let index = head as usize;
            let object = &mut self.objects[index];
            match object.next_free {
                | Some(next) => _ = self.free.insert(len, next.get() - 1),
                | None => _ = self.free.remove(&len),
            }
            object.base = base;
            object.kind = kind;
            object.next_free = None;
            object.cells.fill(Cell::POISON);
            index
        } else {
            if self.objects.len() >= u32::MAX as usize - 1 {
                return Err(Fault::Unsupported(Unsupported::AddressSpace));
            }
            let mut cells = ArenaVec::new_in(self.arena);
            cells
                .try_reserve_exact(len)
                .map_err(|_| Fault::Unsupported(Unsupported::ObjectSize))?;
            cells.resize(len, Cell::POISON);
            self.objects.push(Object {
                base,
                generation: 0,
                kind,
                cells: cells.leak(),
                next_free: None,
                cursor: VaCursor::default(),
            });
            self.objects.len() - 1
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "The object count was checked against u32::MAX above."
        )]
        self.live.push((base, index as u32));
        Ok(Pointer {
            address:    base,
            provenance: Some(ObjectRef::new(index, self.objects[index].generation)),
        })
    }

    /// Frees the live object `provenance` names, so every pointer to it
    /// dangles.
    pub(crate) fn free(&mut self, provenance: ObjectRef) {
        let index = provenance.index();
        let object = &mut self.objects[index];
        debug_assert!(
            object.generation == provenance.generation && object.kind != ObjectKind::Free,
            "only live objects are freed"
        );
        object.generation = object.generation.wrapping_add(1);
        object.kind = ObjectKind::Free;
        let len = object.cells.len();
        let base = object.base;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Object indices fit in u32; `allocate` checks."
        )]
        let raw = index as u32;
        object.next_free = self
            .free
            .insert(len, raw)
            .map(|next| NonZeroU32::new(next + 1).expect("an index plus one is nonzero"));
        if let Ok(position) = self.live.binary_search_by_key(&base, |&(base, _)| base) {
            _ = self.live.remove(position);
        }
    }

    /// `free`: releases a heap allocation. Freeing null does nothing;
    /// freeing anything but the start of a live heap object is undefined.
    ///
    /// C99: §7.20.3.2 paragraph 2, pp. 313-314; PDF pp. 325-326.
    pub(crate) fn free_heap(&mut self, pointer: RuntimeValue) -> Result<(), Fault> {
        let RuntimeValue::Ptr(pointer) = pointer else {
            return Err(UbKind::PoisonInHostCall.into());
        };
        if pointer == Pointer::NULL {
            return Ok(());
        }
        let provenance = pointer.provenance.ok_or(UbKind::InvalidFree)?;
        match self.live_object(provenance) {
            | Some(object) if object.kind == ObjectKind::Heap && object.base == pointer.address => {
                self.free(provenance);
                Ok(())
            },
            | _ => Err(UbKind::InvalidFree.into()),
        }
    }

    /// Writes `bytes` at the start of a fresh object and zeros after them,
    /// as a global's initializer does.
    pub(crate) fn initialize(&mut self, pointer: Pointer, bytes: &[u8]) {
        let index = pointer.provenance.expect("a fresh object").index();
        let mut bytes = bytes.iter().copied();
        for cell in self.objects[index].cells.iter_mut() {
            *cell = Cell::data(bytes.next().unwrap_or(0));
        }
    }

    /// Writes a pointer into a fresh object at `offset`, as a relocation
    /// does, ignoring alignment and constness.
    pub(crate) fn relocate(&mut self, object: Pointer, offset: u64, value: Pointer) {
        let index = object.provenance.expect("a fresh object").index();
        let cells = &mut self.objects[index].cells;
        for position in 0..8 {
            let cell = usize::try_from(offset + position)
                .ok()
                .and_then(|cell| cells.get_mut(cell));
            if let Some(cell) = cell {
                *cell = Cell {
                    provenance: value.provenance,
                    byte:       truncate_u8(u128::from(value.address >> (8 * position))),
                    tag:        Cell::FRAGMENT_TAG + truncate_u8(u128::from(position)),
                };
            }
        }
    }

    /// Appends the bytes of the C string at `pointer` to `out`, without its
    /// terminating null.
    ///
    /// C99: §7.1.1 paragraph 1, p. 164; PDF p. 176.
    pub(crate) fn read_c_string(
        &self,
        pointer: RuntimeValue,
        out: &mut ArenaVec<'_, u8>,
    ) -> Result<(), Fault> {
        let (index, offset) = self.resolve(pointer, 0, Align::BYTE, false)?;
        for cell in &self.objects[index].cells[offset..] {
            if cell.tag == Cell::POISON_TAG {
                return Err(UbKind::PoisonInHostCall.into());
            }
            if cell.byte == 0 {
                return Ok(());
            }
            out.push(cell.byte);
        }
        Err(UbKind::OutOfBounds.into())
    }
}

/// The bytes a load or store of `ty` accesses.
fn stored_size(ty: Type) -> Result<u32, Fault> {
    match ty {
        | Type::F80 | Type::F128 => Err(Fault::Unsupported(Unsupported::WideFloat)),
        | _ => Ok(ty.bytes()),
    }
}

/// The size operand of a `copy` or `fill`.
fn defined_size(size: RuntimeValue) -> Result<u64, Fault> {
    match size {
        | RuntimeValue::Int(bits) => Ok(truncate_u64(bits)),
        | _ => Err(UbKind::PoisonAddress.into()),
    }
}

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
const fn truncate_u8(bits: u128) -> u8 {
    bits as u8
}

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
const fn truncate_u32(bits: u128) -> u32 {
    bits as u32
}

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
pub(super) const fn truncate_u64(bits: u128) -> u64 {
    bits as u64
}

impl<'a> Memory<'a> {
    /// Empty memory whose objects live in `arena`.
    pub(crate) fn new(arena: &'a Bump, max_object_size: u64) -> Self {
        Self {
            arena,
            objects: ArenaVec::new_in(arena),
            live: ArenaVec::new_in(arena),
            free: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            next_address: FIRST_ADDRESS,
            max_object_size,
        }
    }
}
