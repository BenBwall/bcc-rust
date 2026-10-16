//! Variadic arguments: independent of any target's `va_list` layout before
//! ABI lowering, in the target's layout after it.
//!
//! In a pre-ABI module the interpreter's `va_list` holds a pointer to a
//! cursor object: memory that the program cannot read or write, recording
//! which frame's variadic arguments it walks and how many `va_arg` has
//! taken. `va_start` creates a cursor and stores its address into the
//! `va_list` the operand points to; `va_copy` duplicates one; `va_end` frees
//! it. A cursor whose function has returned, a freed one, and memory that
//! never held one are all invalid.
//!
//! A post-ABI module has no `va_arg`: ABI lowering expanded it into loads
//! from the target's `va_list`. There `va_start` lays out the variadic
//! arguments once per activation as the target's calling convention leaves
//! them, in objects that live until the function returns: for System V, a
//! register save area holding the arguments that went in registers and an
//! overflow area holding the rest, with the `va_list` fields pointing into
//! them; for Win64, one 8-byte slot per argument and a `va_list` pointing to
//! the first. `va_copy` copies the `va_list`'s bytes and `va_end` does nothing.
//!
//! C99: §7.15.1, pp. 249-251; PDF pp. 261-263.

use super::{
    Machine,
    memory::{
        ObjectKind,
        VaCursor,
    },
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
    abi::{
        Convention,
        sysv::{
            ArgumentRegisters,
            va_list,
        },
    },
    ir::{
        Align,
        Profile,
        Signature,
        Type,
    },
    target::Target,
};

/// `pointer` moved by `bytes` within its object.
fn offset(pointer: Pointer, bytes: u64) -> Pointer {
    Pointer {
        address:    pointer.address + bytes,
        provenance: pointer.provenance,
    }
}

/// The bytes a System V overflow-area argument of type `ty` takes.
/// psABI §3.5.7, "The `va_arg` Macro", step 9: arguments are padded to
/// eightbytes.
fn slot_size(ty: Type) -> u64 {
    u64::from(ty.bytes()).next_multiple_of(8)
}

/// The alignment of a System V overflow-area argument of type `ty`.
/// psABI §3.5.7, "The `va_arg` Macro", step 7: 16 for a type aligned to 16,
/// else 8.
fn slot_align(ty: Type) -> u64 {
    if ty.bytes() > 8 { 16 } else { 8 }
}

impl Machine<'_, '_, '_> {
    /// `va_start`: points the `va_list` at `list` to a fresh cursor over the
    /// executing frame's variadic arguments.
    ///
    /// C99: §7.15.1.4, p. 251; PDF p. 263.
    pub(super) fn va_start(&mut self, list: RuntimeValue) -> Result<(), Fault> {
        let frame = *self.frame();
        let signature = self.module.function_signature(frame.func);
        if !signature.variadic {
            return Err(UbKind::VaStartOutsideVariadic.into());
        }
        if self.module.profile() == Profile::PostAbi {
            return self.va_start_for_target(list, signature);
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "The stack-depth limit keeps depths far below u32::MAX."
        )]
        let depth = (self.frames.len() - 1) as u32;
        self.new_cursor(
            list,
            VaCursor {
                depth,
                activation: frame.activation,
                position: 0,
            },
        )
    }

    /// `va_arg`: the next variadic argument, which must have type `ty`.
    ///
    /// C99: §7.15.1.1 paragraph 2, pp. 249-250; PDF pp. 261-262.
    pub(super) fn va_arg(&mut self, ty: Type, list: RuntimeValue) -> Result<RuntimeValue, Fault> {
        let (cursor_ref, cursor) = self.cursor(list)?;
        let frame = self.frames[cursor.depth as usize];
        let position = cursor.position as usize;
        if position >= frame.varargs_len {
            return Err(UbKind::VaArgPastEnd.into());
        }
        let (arg_ty, value, _) = self.varargs[frame.varargs + position];
        if arg_ty != ty {
            return Err(UbKind::VaArgTypeMismatch.into());
        }
        self.memory.set_cursor(
            cursor_ref,
            VaCursor {
                position: cursor.position + 1,
                ..cursor
            },
        );
        Ok(value)
    }

    /// `va_copy`: points the `va_list` at `destination` to a copy of the
    /// cursor at `source`.
    ///
    /// C99: §7.15.1.2, p. 250; PDF p. 262.
    pub(super) fn va_copy(
        &mut self,
        destination: RuntimeValue,
        source: RuntimeValue,
    ) -> Result<(), Fault> {
        if self.module.profile() == Profile::PostAbi {
            let size = match self.convention()? {
                | Convention::SystemV => va_list::SIZE,
                | Convention::Win64 => 8,
            };
            let size = RuntimeValue::int(Type::I64, u128::from(size));
            return self
                .memory
                .copy(destination, source, size, Align::BYTE, false);
        }
        let (_, cursor) = self.cursor(source)?;
        self.new_cursor(destination, cursor)
    }

    /// `va_end`: frees the cursor at `list`.
    ///
    /// C99: §7.15.1.3, p. 250; PDF p. 262.
    pub(super) fn va_end(&mut self, list: RuntimeValue) -> Result<(), Fault> {
        if self.module.profile() == Profile::PostAbi {
            return Ok(());
        }
        let (cursor_ref, _) = self.cursor(list)?;
        self.memory.free(cursor_ref);
        Ok(())
    }

    /// Restarts a list's own cursor while reusing argument areas owned by its
    /// activation. They are laid out once, lazily, and freed with frame slots.
    /// C99: §7.15.1.4p3, p. 251; PDF p. 263; §7.15.1.2, p. 250; PDF p. 262.
    fn va_start_for_target(
        &mut self,
        list: RuntimeValue,
        signature: Signature<'_>,
    ) -> Result<(), Fault> {
        let backing = if let Some(backing) = self.frame().va_backing {
            backing
        } else {
            let backing = self.prepare_target_varargs(signature)?;
            if let Some(frame) = self.frames.last_mut() {
                frame.va_backing = Some(backing);
            }
            backing
        };
        match backing {
            | VaBacking::SystemV { save, overflow } => {
                let fixed = ArgumentRegisters::after_fixed(signature);
                let base = list.as_pointer().ok_or(Fault::Ub(UbKind::InvalidVaList))?;
                let field = |at| RuntimeValue::Ptr(offset(base, at));
                let gp_offset = 8 * u128::from(fixed.gp);
                let fp_offset = u128::from(va_list::GP_AREA_SIZE) + 16 * u128::from(fixed.sse);
                self.memory.store(
                    Type::I32,
                    RuntimeValue::int(Type::I32, gp_offset),
                    field(va_list::GP_OFFSET),
                    Align::BYTE,
                )?;
                self.memory.store(
                    Type::I32,
                    RuntimeValue::int(Type::I32, fp_offset),
                    field(va_list::FP_OFFSET),
                    Align::BYTE,
                )?;
                self.memory.store(
                    Type::Ptr,
                    RuntimeValue::Ptr(overflow),
                    field(va_list::OVERFLOW_ARG_AREA),
                    Align::BYTE,
                )?;
                self.memory.store(
                    Type::Ptr,
                    RuntimeValue::Ptr(save),
                    field(va_list::REG_SAVE_AREA),
                    Align::BYTE,
                )
            },
            | VaBacking::Win64(slots) =>
                self.memory
                    .store(Type::Ptr, RuntimeValue::Ptr(slots), list, Align::BYTE),
        }
    }

    /// C99: §7.15.1.1, pp. 249-250; PDF pp. 261-262.
    pub(super) fn va_arg_aggregate(
        &mut self,
        layout: crate::ir::AggregateLayout<'_>,
        list: RuntimeValue,
    ) -> Result<RuntimeValue, Fault> {
        let (cursor_ref, mut cursor) = self.cursor(list)?;
        let frame = self.frames[cursor.depth as usize];
        let position = cursor.position as usize;
        if position >= frame.varargs_len {
            return Err(UbKind::VaArgPastEnd.into());
        }
        let (_, value, abi) = self.varargs[frame.varargs + position];
        if !matches!(abi, Some(crate::ir::ParamAbi::Aggregate(actual)) if actual == layout) {
            return Err(UbKind::VaArgTypeMismatch.into());
        }
        cursor.position += 1;
        self.memory.set_cursor(cursor_ref, cursor);
        let pointer = self.frame_object(layout.size, layout.align)?;
        self.memory.copy(
            RuntimeValue::Ptr(pointer),
            value,
            RuntimeValue::Int(u128::from(layout.size)),
            layout.align,
            false,
        )?;
        Ok(RuntimeValue::Ptr(pointer))
    }

    /// Materializes the target's incoming variadic argument areas once.
    /// C99: §7.15.1.4p3, p. 251; PDF p. 263 (restarts reuse incoming
    /// arguments).
    fn prepare_target_varargs(&mut self, signature: Signature<'_>) -> Result<VaBacking, Fault> {
        let frame = *self.frame();
        let start = frame.varargs;
        let end = start + frame.varargs_len;
        match self.convention()? {
            | Convention::SystemV => {
                // System V psABI §3.5.7: the fixed arguments decide which
                // registers remain for the variadic ones.
                let fixed = ArgumentRegisters::after_fixed(signature);
                let mut scan = fixed;
                let mut overflow_size = 0_u64;
                let mut overflow_align = Align::from_bytes(16).unwrap();
                for index in start..end {
                    let (ty, _, abi) = self.varargs[index];
                    if let Some(crate::ir::ParamAbi::ByVal(layout)) = abi {
                        // Offsets alone cannot align arguments whose backing
                        // object's base has a weaker alignment.
                        if layout.align.bytes() > overflow_align.bytes() {
                            overflow_align = layout.align;
                        }
                        overflow_size = overflow_size.next_multiple_of(layout.align.bytes().max(8))
                            + layout.size.next_multiple_of(8);
                    } else if !scan.take_scalar(ty) {
                        overflow_size =
                            overflow_size.next_multiple_of(slot_align(ty)) + slot_size(ty);
                    }
                }
                let save =
                    self.frame_object(va_list::SAVE_AREA_SIZE, Align::from_bytes(16).unwrap())?;
                let overflow = self.frame_object(overflow_size, overflow_align)?;
                let mut used = fixed;
                let mut overflow_offset = 0_u64;
                for index in start..end {
                    let (ty, value, abi) = self.varargs[index];
                    if let Some(crate::ir::ParamAbi::ByVal(layout)) = abi {
                        overflow_offset =
                            overflow_offset.next_multiple_of(layout.align.bytes().max(8));
                        self.memory.copy(
                            RuntimeValue::Ptr(offset(overflow, overflow_offset)),
                            value,
                            RuntimeValue::Int(u128::from(layout.size)),
                            layout.align,
                            false,
                        )?;
                        overflow_offset += layout.size.next_multiple_of(8);
                        continue;
                    }
                    let before = used;
                    let address = if used.take_scalar(ty) {
                        if used.gp > before.gp {
                            offset(save, 8 * u64::from(before.gp))
                        } else {
                            offset(save, va_list::GP_AREA_SIZE + 16 * u64::from(before.sse))
                        }
                    } else {
                        overflow_offset = overflow_offset.next_multiple_of(slot_align(ty));
                        let address = offset(overflow, overflow_offset);
                        overflow_offset += slot_size(ty);
                        address
                    };
                    self.memory
                        .store(ty, value, RuntimeValue::Ptr(address), Align::BYTE)?;
                }
                Ok(VaBacking::SystemV { save, overflow })
            },
            | Convention::Win64 => {
                // Microsoft x64, "Varargs": one 8-byte slot per argument.
                let slots =
                    self.frame_object(8 * frame.varargs_len as u64, Align::from_bytes(8).unwrap())?;
                for (slot, index) in (start..end).enumerate() {
                    let (ty, value, _) = self.varargs[index];
                    if ty.bytes() > 8 {
                        return Err(Fault::Unsupported(Unsupported::WideVariadic));
                    }
                    self.memory.store(
                        ty,
                        value,
                        RuntimeValue::Ptr(offset(slots, 8 * slot as u64)),
                        Align::BYTE,
                    )?;
                }
                Ok(VaBacking::Win64(slots))
            },
        }
    }

    /// Prepares the `va_list` operand of a host function that reads one:
    /// after ABI lowering, the target's convention, whose layout the host
    /// reads; before it, `None`, with the values the list's cursor has left
    /// in `host_varargs`. A System V list is an array, passed as the address
    /// of the object holding the cursor; a Win64 list is the cursor pointer.
    /// C99: §7.15p3, p. 249; PDF p. 261.
    pub(super) fn host_va_list(&mut self, name: &str) -> Result<Option<Convention>, Fault> {
        self.host_varargs.clear();
        let Some(operand) = super::host::va_list_operand(name) else {
            return Ok(None);
        };
        let convention = self.convention()?;
        if self.module.profile() == Profile::PostAbi {
            return Ok(Some(convention));
        }
        let list = self
            .arguments
            .get(operand)
            .copied()
            .ok_or(Fault::Ub(UbKind::InvalidVaList))?;
        let pointer = match convention {
            | Convention::SystemV => self.memory.load(Type::Ptr, list, Align::BYTE)?,
            | Convention::Win64 => list,
        };
        let (_, cursor) = self.cursor_at(pointer)?;
        let frame = self.frames[cursor.depth as usize];
        let start = frame.varargs + cursor.position as usize;
        for index in start..frame.varargs + frame.varargs_len {
            let (_, value, _) = self.varargs[index];
            self.host_varargs.push(value);
        }
        Ok(None)
    }

    /// The calling convention of the module's target, whose `va_list` a
    /// post-ABI module uses.
    fn convention(&self) -> Result<Convention, Fault> {
        Target::parse(self.module.triple())
            .map(Convention::of)
            .ok_or(Fault::Unsupported(Unsupported::VaListTarget))
    }

    /// A zero-filled object retaining the required argument alignment, alive
    /// until the executing frame returns. Win64 indirect arguments already
    /// point to aligned caller copies; only their pointer slots need eight.
    /// C99: §7.15.1.1p2, pp. 249-250; PDF pp. 261-262.
    fn frame_object(&mut self, size: u64, align: Align) -> Result<Pointer, Fault> {
        let pointer = self.memory.allocate(ObjectKind::Stack, size, align)?;
        self.memory.initialize(pointer, &[]);
        self.slots.push(pointer);
        Ok(pointer)
    }

    /// Allocates a cursor and stores its address at `list`.
    fn new_cursor(&mut self, list: RuntimeValue, cursor: VaCursor) -> Result<(), Fault> {
        let pointer = self.memory.allocate(ObjectKind::VaCursor, 0, Align::BYTE)?;
        let cursor_ref = pointer.provenance.expect("fresh objects have provenance");
        self.memory.set_cursor(cursor_ref, cursor);
        self.memory
            .store(Type::Ptr, RuntimeValue::Ptr(pointer), list, Align::BYTE)
    }

    /// The live cursor the `va_list` at `list` points to, whose frame must
    /// still be the activation that made it.
    fn cursor(&self, list: RuntimeValue) -> Result<(ObjectRef, VaCursor), Fault> {
        self.cursor_at(self.memory.load(Type::Ptr, list, Align::BYTE)?)
    }

    /// The live cursor `pointer` designates.
    fn cursor_at(&self, pointer: RuntimeValue) -> Result<(ObjectRef, VaCursor), Fault> {
        let invalid = || Fault::Ub(UbKind::InvalidVaList);
        let pointer = pointer.as_pointer().ok_or_else(invalid)?;
        let cursor_ref = pointer.provenance.ok_or_else(invalid)?;
        let cursor = self.memory.cursor(cursor_ref).ok_or_else(invalid)?;
        let alive = self
            .frames
            .get(cursor.depth as usize)
            .is_some_and(|frame| frame.activation == cursor.activation);
        if alive {
            Ok((cursor_ref, cursor))
        } else {
            Err(invalid())
        }
    }
}

/// Incoming argument storage shared by this activation's independent lists.
#[derive(Clone, Copy, Debug)]
pub(super) enum VaBacking {
    SystemV {
        save:     Pointer,
        overflow: Pointer,
    },
    Win64(Pointer),
}
