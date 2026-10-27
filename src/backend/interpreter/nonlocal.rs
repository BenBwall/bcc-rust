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
//! GCC's `__builtin_setjmp` (`sjlj_setjmp`) keeps the same record in the
//! three words of its five-word buffer that the frame address (word 0) and
//! the stack position (word 2) leave free: word 1 holds the block and the
//! position after the instruction, word 3 the activation, and word 4 the
//! depth and the dynamic-allocation mark. `sjlj_longjmp` resumes it with 1.
//! GNU extension: GCC manual, "Nonlocal Gotos".
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
        Opcode,
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
                if call_result != Some(Type::I32) || !save_arguments(&self.argument_types) {
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
            *word = self.load_word(buffer, index)?;
        }
        let [activation, depth, block, next, mark] = words;
        let saved = Saved {
            activation,
            depth,
            block,
            next,
            mark,
        };
        self.resume(saved, value, |inst| {
            matches!(inst, InstData::Call { .. } | InstData::CallIndirect { .. })
        })
    }

    /// Runs `sjlj_setjmp`: records the executing frame and its position
    /// after the instruction in words 1, 3 and 4 of the buffer at `buffer`.
    pub(super) fn save_builtin_environment(&mut self, buffer: RuntimeValue) -> Result<(), Fault> {
        let frame = *self.frame();
        let pack = |high: usize, low: usize| -> Result<u64, Fault> {
            let (Ok(high), Ok(low)) = (u32::try_from(high), u32::try_from(low)) else {
                return Err(Fault::Unsupported(Unsupported::AddressSpace));
            };
            Ok(u64::from(high) << 32 | u64::from(low))
        };
        let words = [
            (1, pack(frame.block.index(), frame.next)?),
            (3, frame.activation),
            (4, pack(self.frames.len() - 1, self.dynamic_slots.len())?),
        ];
        for (index, word) in words {
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

    /// Runs `sjlj_longjmp`: resumes the `sjlj_setjmp` that filled `buffer`
    /// with 1, discarding the frames called since.
    pub(super) fn restore_builtin_environment(
        &mut self,
        buffer: RuntimeValue,
    ) -> Result<(), Fault> {
        let resume = self.load_word(buffer, 1)?;
        let activation = self.load_word(buffer, 3)?;
        let depth = self.load_word(buffer, 4)?;
        let low = |word: usize| word & usize::try_from(u32::MAX).expect("a 64-bit host");
        let saved = Saved {
            activation,
            depth: depth >> 32,
            block: resume >> 32,
            next: low(resume),
            mark: low(depth),
        };
        self.resume(saved, 1, |inst| {
            matches!(
                inst,
                InstData::Unary {
                    opcode: Opcode::SjljSetjmp,
                    ..
                }
            )
        })
    }

    /// Resumes the frame `saved` describes after the instruction that saved
    /// it, which `saving` accepts and whose `i32` result becomes `value`, or
    /// 1 for 0; traps when the frame is no longer that activation or the
    /// record is corrupt.
    fn resume(
        &mut self,
        saved: Saved,
        value: u128,
        saving: impl Fn(&InstData) -> bool,
    ) -> Result<(), Fault> {
        let Saved {
            activation,
            depth,
            block,
            next,
            mark,
        } = saved;
        let Some(&frame) = self.frames.get(depth) else {
            return Err(UbKind::InvalidJmpBuf.into());
        };
        let body = self.body(frame.func);
        // The frame must be the activation that saved the buffer, and the
        // position must follow the saving instruction, which has a result.
        // The block word is untrusted: range-check it before it becomes a
        // `Block`, whose constructor only accepts valid indices.
        let block = (block < body.block_count()).then(|| Block::new(block));
        let call = block
            .filter(|_| word(activation) == frame.activation)
            .map(|block| body.block_insts(block))
            .and_then(|insts| next.checked_sub(1).and_then(|last| insts.get(last)))
            .copied()
            .filter(|&inst| saving(body.inst(inst)))
            .and_then(|inst| body.inst_result(inst))
            .filter(|&result| body.value_type(result) == Type::I32);
        // The saving frame's own dynamic objects end where the next frame's
        // begin.
        let end = self
            .frames
            .get(depth + 1)
            .map_or(self.dynamic_slots.len(), |newer| newer.dynamic_slots);
        let (Some(block), Some(result), true) =
            (block, call, (frame.dynamic_slots..=end).contains(&mark))
        else {
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

    /// Word `index` of the buffer at `buffer`, which must be an integer.
    fn load_word(&mut self, buffer: RuntimeValue, index: usize) -> Result<usize, Fault> {
        let address = self.word(buffer, index);
        let RuntimeValue::Int(bits) = self.memory.load(Type::I64, address, Align::BYTE)? else {
            return Err(UbKind::InvalidJmpBuf.into());
        };
        Ok(usize::try_from(bits).map_err(|_| UbKind::InvalidJmpBuf)?)
    }

    /// The address of word `index` of the buffer at `buffer`.
    fn word(&self, buffer: RuntimeValue, index: usize) -> RuntimeValue {
        self.memory
            .ptr_add(buffer, RuntimeValue::Int(8 * index as u128), true)
    }
}

/// A saved environment: the saving frame's activation and depth, the block
/// and position after the saving instruction, and the dynamic-allocation
/// mark.
#[derive(Clone, Copy)]
struct Saved {
    activation: usize,
    depth:      usize,
    block:      usize,
    next:       usize,
    mark:       usize,
}

/// Whether `types` are the arguments of a `setjmp`-family call on an x86-64
/// target. One spelling has different arities across C libraries (glibc's
/// `_setjmp(env)`, MinGW-w64's `_setjmp(env, frame)`), so the check accepts
/// the three shapes the family has rather than one per name: the buffer
/// alone (`setjmp`, glibc `_setjmp`), with an `int` signal-mask flag
/// (`sigsetjmp`, `__sigsetjmp`), or with a frame pointer (MinGW-w64
/// `_setjmp`, `_setjmpex`, `__mingw_setjmp`, `__intrinsic_setjmp(ex)`).
/// 32-bit MSVC's variadic `_setjmp3` has no x86-64 form and is refused.
fn save_arguments(types: &[Type]) -> bool {
    matches!(types, [Type::Ptr] | [Type::Ptr, Type::I32 | Type::Ptr])
}

/// A count or index as a saved word; bcc runs only on 64-bit hosts.
fn word(value: usize) -> u64 {
    u64::try_from(value).expect("a 64-bit host")
}
