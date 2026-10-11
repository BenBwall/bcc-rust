//! Variadic arguments, independent of any target's `va_list` layout.
//!
//! The interpreter's `va_list` holds a pointer to a cursor object: memory
//! that the program cannot read or write, recording which frame's variadic
//! arguments it walks and how many `va_arg` has taken. `va_start` creates a
//! cursor and stores its address into the `va_list` the operand points to;
//! `va_copy` duplicates one; `va_end` frees it. A cursor whose function has
//! returned, a freed one, and memory that never held one are all invalid.
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
    },
    value::{
        ObjectRef,
        RuntimeValue,
    },
};
use crate::ir::{
    Align,
    Type,
};

impl Machine<'_, '_, '_> {
    /// `va_start`: points the `va_list` at `list` to a fresh cursor over the
    /// executing frame's variadic arguments.
    ///
    /// C99: §7.15.1.4, p. 251; PDF p. 263.
    pub(super) fn va_start(&mut self, list: RuntimeValue) -> Result<(), Fault> {
        let frame = *self.frame();
        if !self.module.function_signature(frame.func).variadic {
            return Err(UbKind::VaStartOutsideVariadic.into());
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
        let (arg_ty, value) = self.varargs[frame.varargs + position];
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
        let (_, cursor) = self.cursor(source)?;
        self.new_cursor(destination, cursor)
    }

    /// `va_end`: frees the cursor at `list`.
    ///
    /// C99: §7.15.1.3, p. 250; PDF p. 262.
    pub(super) fn va_end(&mut self, list: RuntimeValue) -> Result<(), Fault> {
        let (cursor_ref, _) = self.cursor(list)?;
        self.memory.free(cursor_ref);
        Ok(())
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
        let invalid = || Fault::Ub(UbKind::InvalidVaList);
        let pointer = self
            .memory
            .load(Type::Ptr, list, Align::BYTE)?
            .as_pointer()
            .ok_or_else(invalid)?;
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
