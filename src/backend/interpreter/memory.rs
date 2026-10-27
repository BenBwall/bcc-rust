//! Byte-addressed memory with provenance.
//!
//! Memory is a set of objects: stack slots, globals, heap allocations,
//! functions (size zero, so a function's address is unique), each function's
//! labels (one byte per label, holding no data) and `va_list` cursors. Each
//! object has a fresh base address, a size and an array of cells, with backing
//! capacity rounded up to a power of two for reuse. Logical bounds remain the
//! requested size. A cell holds the byte, whether it was ever written with a
//! defined value, and, for the eight bytes of a stored pointer, which fragment
//! of which pointer it is. Loading a pointer whose eight cells are the
//! fragments of one pointer in order gives that pointer back with its
//! provenance; loading it from other bytes treats them as an integer address,
//! as `inttoptr` does. Unresolved globals retain their declared bounds for
//! pointer arithmetic, but have no backing cells; any data access reports the
//! missing symbol.
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
        GlobalId,
        Type,
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
    x87::X87,
};

/// The interpreter's memory: every object a program can address.
pub(crate) struct Memory<'a> {
    arena:            &'a Bump,
    objects:          ArenaVec<'a, Object<'a>>,
    /// The base address and index of every live object, sorted by address.
    live:             ArenaVec<'a, (u64, u32)>,
    /// For each power-of-two capacity, the first freed object; the rest
    /// are chained through [`Object::next_free`].
    free:             ArenaMap<'a, usize, u32>,
    next_address:     u64,
    max_object_size:  u64,
    /// The `FILE` objects of `stdin`, `stdout` and `stderr`, once made.
    standard_streams: [Option<Pointer>; 3],
}

/// One object of memory.
struct Object<'a> {
    base:       u64,
    generation: u32,
    kind:       ObjectKind,
    /// Logical bounds, independent of the reusable backing capacity.
    size:       usize,
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
    /// A declared object with no definition. Its address has provenance,
    /// but it has no backing cells and every data access faults.
    UnresolvedGlobal(GlobalId),
    /// A `constant` global, such as a string literal; writing it is
    /// undefined.
    ConstantGlobal,
    /// A `malloc` or `calloc` allocation.
    Heap,
    /// A function, whose address `func_addr` takes. It holds no data.
    Function(FuncId),
    /// The addresses of a function's labels: label `n` is the object's
    /// byte `n`, so label differences are small offsets and a label address
    /// plus a difference names another label. It holds no data.
    /// GNU extension: GCC "Labels as Values".
    Labels(FuncId),
    /// A `va_list` cursor, which `va_start` and `va_copy` create. Its state
    /// is the object's [`VaCursor`].
    VaCursor,
    /// A C library stream, the `FILE` a `FILE *` points to, numbered for the
    /// host; it holds no data the program may access. Streams 0, 1 and 2 are
    /// `stdin`, `stdout` and `stderr`.
    /// C99: §7.19.3 paragraphs 1 and 7, pp. 264-265; PDF pp. 276-277.
    Stream(u32),
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
        let size = ty.bytes();
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
            | Type::F16 => RuntimeValue::F16(truncate_u16(bits)),
            | Type::F32 => RuntimeValue::F32(f32::from_bits(truncate_u32(bits))),
            | Type::F64 => RuntimeValue::F64(f64::from_bits(truncate_u64(bits))),
            | Type::F80 => RuntimeValue::F80(X87::from_bits(bits)),
            | Type::F128 => RuntimeValue::F128(bits),
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
        let size = ty.bytes();
        let (index, offset) = self.resolve(address, u64::from(size), align, true)?;
        let cells = &mut self.objects[index].cells[offset..offset + size as usize];
        let (bits, provenance) = match value {
            | RuntimeValue::Poison => {
                cells.fill(Cell::POISON);
                return Ok(());
            },
            | RuntimeValue::Int(bits) | RuntimeValue::F128(bits) => (bits, None),
            | RuntimeValue::F16(bits) => (u128::from(bits), None),
            | RuntimeValue::F32(value) => (u128::from(value.to_bits()), None),
            | RuntimeValue::F64(value) => (u128::from(value.to_bits()), None),
            | RuntimeValue::F80(value) => (value.to_bits(), None),
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

    /// Executes one libatomic buffer operation as one interpreter step. The
    /// interpreter is single-threaded; its trace is a sequentially consistent
    /// execution admitted by every source ordering.
    pub(super) fn atomic_bytes(
        &mut self,
        op: crate::ir::AtomicOp,
        args: &[RuntimeValue],
    ) -> Result<RuntimeValue, Fault> {
        use crate::ir::AtomicOp as A;
        let size = *args.last().expect("verified byte atomic size");
        let count = defined_size(size)?;
        let align = Align::BYTE;
        let copy = |this: &mut Self, to, from| this.copy(to, from, size, align, true);
        match op {
            | A::LoadBytes => copy(self, args[1], args[0])?,
            | A::StoreBytes => copy(self, args[0], args[1])?,
            | A::ExchangeBytes => {
                let temp = self.allocate(ObjectKind::Heap, count, align)?;
                let temp_value = RuntimeValue::Ptr(temp);
                copy(self, temp_value, args[0])?;
                copy(self, args[0], args[1])?;
                copy(self, args[2], temp_value)?;
                self.free(temp.provenance.expect("temporary"));
            },
            | A::CompareExchangeBytes => {
                let (object, object_offset) = self.resolve(args[0], count, align, false)?;
                let (expected, expected_offset) = self.resolve(args[1], count, align, false)?;
                let count = usize::try_from(count).map_err(|_| UbKind::OutOfBounds)?;
                let equal = self.objects[object].cells[object_offset..object_offset + count]
                    .iter()
                    .zip(&self.objects[expected].cells[expected_offset..expected_offset + count])
                    .all(|(a, b)| a.byte == b.byte);
                if equal {
                    copy(self, args[0], args[2])?;
                } else {
                    copy(self, args[1], args[0])?;
                }
                return Ok(RuntimeValue::Int(u128::from(equal)));
            },
            | _ => unreachable!("byte atomic operation"),
        }
        Ok(RuntimeValue::Poison)
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
        let end = object.base + object.size as u64;
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
            (address - base <= object.size as u64)
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
            | ObjectKind::UnresolvedGlobal(global) => return Err(Fault::UnknownGlobal(global)),
            | ObjectKind::Function(_)
            | ObjectKind::Labels(_)
            | ObjectKind::VaCursor
            | ObjectKind::Stream(_) => return Err(UbKind::NotData.into()),
            | ObjectKind::ConstantGlobal if write => return Err(UbKind::WriteToConstant.into()),
            | _ => {},
        }
        let offset = pointer.address.wrapping_sub(object.base);
        let len = object.size as u64;
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

    /// The index of the label of `func` that `target` addresses, if it
    /// addresses one: an `indirect_br` may only continue at such a label.
    pub(crate) fn label_at(&self, target: RuntimeValue, func: FuncId) -> Result<usize, Fault> {
        let RuntimeValue::Ptr(pointer) = target else {
            return Err(UbKind::BranchOnPoison.into());
        };
        let object = pointer
            .provenance
            .and_then(|provenance| self.live_object(provenance));
        match object {
            | Some(&Object {
                base,
                size,
                kind: ObjectKind::Labels(owner),
                ..
            }) if owner == func
                && pointer
                    .address
                    .checked_sub(base)
                    .is_some_and(|offset| offset < size as u64) =>
                usize::try_from(pointer.address - base).map_err(|_| UbKind::IndirectBranch.into()),
            | _ => Err(UbKind::IndirectBranch.into()),
        }
    }
}

impl Memory<'_> {
    #[cfg(test)]
    pub(super) fn backing_cells(&self) -> usize {
        self.objects.iter().map(|object| object.cells.len()).sum()
    }

    /// Allocates an object of `size` poison bytes and returns a pointer to
    /// its start. Freed storage is reused by capacity class, while logical
    /// bounds, addresses and generations describe this allocation alone.
    /// Unresolved globals reserve only an address range and object record.
    /// C99: §7.20.3.3 paragraph 2, p. 314; PDF p. 326 (indeterminate bytes);
    /// §6.2.4 paragraph 2, p. 32; PDF p. 44 (object lifetime).
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
        let capacity = if len == 0 || matches!(kind, ObjectKind::UnresolvedGlobal(_)) {
            0
        } else {
            len.checked_next_power_of_two()
                .ok_or(Fault::Unsupported(Unsupported::ObjectSize))?
        };
        let alignment = align.bytes().max(BASE_ALIGN);
        let base = self
            .next_address
            .checked_next_multiple_of(alignment)
            .ok_or(Fault::Unsupported(Unsupported::AddressSpace))?;
        self.next_address = base
            .checked_add(size)
            .and_then(|end| end.checked_add(OBJECT_GAP))
            .ok_or(Fault::Unsupported(Unsupported::AddressSpace))?;
        let index = if let Some(&head) = self.free.get(&capacity) {
            let index = head as usize;
            let object = &mut self.objects[index];
            match object.next_free {
                | Some(next) => _ = self.free.insert(capacity, next.get() - 1),
                | None => _ = self.free.remove(&capacity),
            }
            object.base = base;
            object.kind = kind;
            object.size = len;
            object.next_free = None;
            object.cells.fill(Cell::POISON);
            index
        } else {
            if self.objects.len() >= u32::MAX as usize - 1 {
                return Err(Fault::Unsupported(Unsupported::AddressSpace));
            }
            let mut cells = ArenaVec::new_in(self.arena);
            cells
                .try_reserve_exact(capacity)
                .map_err(|_| Fault::Unsupported(Unsupported::ObjectSize))?;
            cells.resize(capacity, Cell::POISON);
            self.objects.push(Object {
                base,
                generation: 0,
                kind,
                size: len,
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

    /// The address the next allocation starts from. Addresses only grow, so
    /// it separates every object allocated so far from every later one.
    pub(crate) const fn next_address(&self) -> u64 {
        self.next_address
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

    /// The number of the live stream `pointer` points to the start of.
    pub(crate) fn stream_at(&self, pointer: RuntimeValue) -> Option<u32> {
        let pointer = pointer.as_pointer()?;
        match self.live_object(pointer.provenance?)? {
            | &Object {
                base,
                kind: ObjectKind::Stream(stream),
                ..
            } if base == pointer.address => Some(stream),
            | _ => None,
        }
    }

    /// The `FILE` object of standard stream `stream` (0 to 2), made the
    /// first time anything asks for it.
    pub(crate) fn standard_stream(&mut self, stream: u32) -> Result<Pointer, Fault> {
        let slot = usize::try_from(stream).map_err(|_| UbKind::InvalidStream)?;
        if let Some(Some(pointer)) = self.standard_streams.get(slot) {
            return Ok(*pointer);
        }
        let pointer = self.allocate(ObjectKind::Stream(stream), 0, Align::BYTE)?;
        *self
            .standard_streams
            .get_mut(slot)
            .ok_or(UbKind::InvalidStream)? = Some(pointer);
        Ok(pointer)
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

    /// The size of the live heap allocation `pointer` points to the start
    /// of, as `realloc` needs it; anything else is undefined, as for `free`.
    ///
    /// C99: §7.20.3.4 paragraphs 2-3, p. 314; PDF p. 326.
    pub(crate) fn heap_size(&self, pointer: RuntimeValue) -> Result<u64, Fault> {
        let RuntimeValue::Ptr(pointer) = pointer else {
            return Err(UbKind::PoisonInHostCall.into());
        };
        let provenance = pointer.provenance.ok_or(UbKind::InvalidFree)?;
        match self.live_object(provenance) {
            | Some(object) if object.kind == ObjectKind::Heap && object.base == pointer.address =>
                Ok(object.size as u64),
            | _ => Err(UbKind::InvalidFree.into()),
        }
    }

    /// Writes `bytes` at the start of a fresh object and zeros after them,
    /// as a global's initializer does.
    pub(crate) fn initialize(&mut self, pointer: Pointer, bytes: &[u8]) {
        let index = pointer.provenance.expect("a fresh object").index();
        let mut bytes = bytes.iter().copied();
        let object = &mut self.objects[index];
        for cell in &mut object.cells[..object.size] {
            *cell = Cell::data(bytes.next().unwrap_or(0));
        }
    }

    /// Writes a pointer into a fresh object at `offset`, as a relocation
    /// does, ignoring alignment and constness.
    pub(crate) fn relocate(&mut self, object: Pointer, offset: u64, value: Pointer) {
        let index = object.provenance.expect("a fresh object").index();
        let object = &mut self.objects[index];
        let cells = &mut object.cells[..object.size];
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

    /// Writes defined bytes into a fresh object at `offset`, as a label
    /// difference relocation does, ignoring alignment and constness.
    pub(crate) fn initialize_bytes(&mut self, object: Pointer, offset: u64, bytes: &[u8]) {
        let index = object.provenance.expect("a fresh object").index();
        let object = &mut self.objects[index];
        let cells = &mut object.cells[..object.size];
        for (position, &byte) in bytes.iter().enumerate() {
            let cell = usize::try_from(offset + position as u64)
                .ok()
                .and_then(|cell| cells.get_mut(cell));
            if let Some(cell) = cell {
                *cell = Cell::data(byte);
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
        let object = &self.objects[index];
        for cell in &object.cells[offset..object.size] {
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
pub(super) const fn truncate_u16(bits: u128) -> u16 {
    bits as u16
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
            standard_streams: [None; 3],
        }
    }
}
