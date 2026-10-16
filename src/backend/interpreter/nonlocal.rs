//! Non-local jumps: the environment operations of the external-symbol
//! contract (`builtins::external_contract`), which need the machine's
//! frames rather than a host.
//!
//! `setjmp` writes the saving frame's activation number, its depth, the
//! block and position after the call and the dynamic-allocation mark into
//! the first [`JMP_BUF_WORDS`] words of its `jmp_buf`, and returns 0. `longjmp`
//! reads them back, checks that the frame at that depth is still the same
//! activation and that the position follows a call, discards every newer
//! frame and the dynamic objects allocated since, and resumes the frame
//! after the call with `val` (1 for 0) as its result. Values keep their
//! state as of the `longjmp`, which C allows: an automatic object changed
//! since the `setjmp` is indeterminate unless it is `volatile`, and a
//! `volatile` one lives in memory. MinGW-w64's `mingw_getsp` returns the
//! frame address, which its `_setjmp` ignores here.
//!
//! C99: §7.13.1.1 paragraphs 2-3, p. 243; PDF p. 255;
//! §7.13.2.1 paragraphs 2-4, p. 244; PDF p. 256.

use super::{
    Machine,
    trap::{
        Fault,
        UbKind,
        Unsupported,
    },
    value::{
        Pointer,
        RuntimeValue,
    },
};
use crate::{
    builtins::ExternalContract,
    ir::{
        Align,
        Block,
        Entity,
        InstData,
        Type,
        Value,
    },
};

/// The saved environment's words: activation, frame depth, block,
/// position, dynamic-allocation mark. glibc's `jmp_buf` has 200 bytes and
/// MinGW-w64's 256, so the record fits either.
const JMP_BUF_WORDS: usize = 5;

impl Machine<'_, '_, '_> {
    /// Runs the environment operation `contract` for a call whose arguments
    /// are in `self.arguments`, whose callee declares `call_result`,
    /// putting a returned value into `result`.
    pub(super) fn environment_call(
        &mut self,
        contract: ExternalContract,
        call_result: Option<Type>,
        result: Option<Value>,
    ) -> Result<(), Fault> {
        match contract {
            | ExternalContract::SaveEnvironment => {
                if call_result != Some(Type::I32) || self.argument_types.first() != Some(&Type::Ptr)
                {
                    return Err(UbKind::SignatureMismatch.into());
                }
                self.save_environment(self.arguments[0])?;
                if let Some(result) = result {
                    self.set(result, RuntimeValue::Int(0));
                }
                Ok(())
            },
            | ExternalContract::RestoreEnvironment => {
                if call_result.is_some() || self.argument_types.as_slice() != [Type::Ptr, Type::I32]
                {
                    return Err(UbKind::SignatureMismatch.into());
                }
                let RuntimeValue::Int(value) = self.arguments[1] else {
                    return Err(UbKind::PoisonInHostCall.into());
                };
                self.restore_environment(self.arguments[0], value)
            },
            | ExternalContract::StackPointer => {
                if call_result != Some(Type::Ptr) || !self.argument_types.is_empty() {
                    return Err(UbKind::SignatureMismatch.into());
                }
                if let Some(result) = result {
                    let pointer = Pointer {
                        address:    self.frame().start,
                        provenance: None,
                    };
                    self.set(result, RuntimeValue::Ptr(pointer));
                }
                Ok(())
            },
            | ExternalContract::ReturnsTwice => Err(Fault::Unsupported(Unsupported::ReturnsTwice)),
        }
    }

    /// Records the executing frame and its position after the current call
    /// in the buffer at `buffer`.
    fn save_environment(&mut self, buffer: RuntimeValue) -> Result<(), Fault> {
        let frame = *self.frame();
        let words: [u64; JMP_BUF_WORDS] = [
            frame.activation,
            word(self.frames.len() - 1),
            word(frame.block.index()),
            word(frame.next),
            word(self.dynamic_slots.len()),
        ];
        for (index, word) in words.into_iter().enumerate() {
            let address = self.word(buffer, index);
            self.memory.store(
                Type::I64,
                RuntimeValue::Int(u128::from(word)),
                address,
                Align::BYTE,
            )?;
        }
        Ok(())
    }

    /// Returns from the `setjmp` that filled `buffer` again, with `value`
    /// or 1 for 0, discarding the frames called since.
    fn restore_environment(&mut self, buffer: RuntimeValue, value: u128) -> Result<(), Fault> {
        let mut words = [0_usize; JMP_BUF_WORDS];
        for (index, word) in words.iter_mut().enumerate() {
            let address = self.word(buffer, index);
            let RuntimeValue::Int(bits) = self.memory.load(Type::I64, address, Align::BYTE)? else {
                return Err(UbKind::InvalidJmpBuf.into());
            };
            *word = usize::try_from(bits).map_err(|_| UbKind::InvalidJmpBuf)?;
        }
        let [activation, depth, block, next, mark] = words;
        let Some(&frame) = self.frames.get(depth) else {
            return Err(UbKind::InvalidJmpBuf.into());
        };
        let body = self.body(frame.func);
        // The frame must be the activation that saved the buffer, and the
        // position must follow the call to `setjmp` that has a result.
        let block = Block::new(block);
        let call = (word(activation) == frame.activation && block.index() < body.block_count())
            .then(|| body.block_insts(block))
            .and_then(|insts| next.checked_sub(1).and_then(|last| insts.get(last)))
            .copied()
            .filter(|&inst| {
                matches!(
                    body.inst(inst),
                    InstData::Call { .. } | InstData::CallIndirect { .. }
                )
            })
            .and_then(|inst| body.inst_result(inst))
            .filter(|&result| body.value_type(result) == Type::I32);
        // The saving frame's own dynamic objects end where the next frame's
        // begin.
        let end = self
            .frames
            .get(depth + 1)
            .map_or(self.dynamic_slots.len(), |newer| newer.dynamic_slots);
        let (Some(result), true) = (call, (frame.dynamic_slots..=end).contains(&mark)) else {
            return Err(UbKind::InvalidJmpBuf.into());
        };
        // Objects allocated since the `setjmp` in newer frames, and dynamic
        // objects of the saving frame allocated after it, go away as the
        // stack does on a target.
        while self.frames.len() > depth + 1 {
            _ = self.pop_frame();
        }
        self.release_dynamic(mark);
        let frame = self.frames.last_mut().expect("the saving frame is live");
        frame.block = block;
        frame.next = next;
        let value = if value & u128::from(u32::MAX) == 0 {
            1
        } else {
            value
        };
        self.set(result, RuntimeValue::int(Type::I32, value));
        Ok(())
    }

    /// The address of word `index` of the buffer at `buffer`.
    fn word(&self, buffer: RuntimeValue, index: usize) -> RuntimeValue {
        self.memory
            .ptr_add(buffer, RuntimeValue::Int(8 * index as u128), true)
    }
}

/// A count or index as a saved word; bcc runs only on 64-bit hosts.
fn word(value: usize) -> u64 {
    u64::try_from(value).expect("a 64-bit host")
}
