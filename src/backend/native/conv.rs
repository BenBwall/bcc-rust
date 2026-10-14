//! The two x86-64 calling conventions as register tables: which registers
//! carry arguments and results, which a call may change, which a function
//! must preserve, and where stack arguments start.
//!
//! ABI lowering (`src/abi`) has already decided how every C argument
//! travels; this only maps the post-ABI signature's scalars onto registers
//! and stack slots.
//!
//! References: System V AMD64 psABI 1.0, §3.2.1 "Registers" (Figure 3.4)
//! and §3.2.3 "Parameter Passing"; Microsoft Learn, "x64 calling
//! convention", sections "Parameter passing" and "Caller/callee saved
//! registers", and "x64 stack usage".

use super::mir::{
    PReg,
    PRegSet,
    RegClass,
};
use crate::target::Target;

/// A calling convention's register tables.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct CallConv {
    pub(crate) kind:             ConvKind,
    /// Integer argument registers in order.
    pub(crate) arguments:        &'static [PReg],
    /// SSE argument registers in order.
    pub(crate) float_arguments:  &'static [PReg],
    /// Registers a call may change.
    pub(crate) caller_saved:     PRegSet,
    /// Registers a function must restore before returning.
    pub(crate) callee_saved:     PRegSet,
    /// The order the allocator tries general registers in: caller-saved
    /// first, since they cost no save, then callee-saved. `%rsp` and the
    /// spill scratch registers are absent.
    pub(crate) allocation:       &'static [PReg],
    /// The same for SSE registers.
    pub(crate) float_allocation: &'static [PReg],
    /// Bytes between the return address and the first stack argument: the
    /// home area the caller reserves for the callee on Win64.
    pub(crate) home_area:        u32,
}

/// Which convention.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ConvKind {
    SystemV,
    Win64,
}

/// The general registers the allocator never assigns: rewritten spill code
/// loads spilled values into them for the one instruction that uses them.
/// Both are caller-saved and carry no argument in either convention.
pub(crate) const SCRATCH: [PReg; 2] = [PReg::R11, PReg::R10];

/// The SSE spill scratch registers. They carry no argument in either
/// convention; Win64 functions that use them save them like any other
/// callee-saved register.
pub(crate) const SCRATCH_XMM: [PReg; 2] = [PReg::Xmm15, PReg::Xmm14];

impl CallConv {
    pub(crate) const SYSTEM_V: Self = Self {
        kind:             ConvKind::SystemV,
        arguments:        &[
            PReg::Rdi,
            PReg::Rsi,
            PReg::Rdx,
            PReg::Rcx,
            PReg::R8,
            PReg::R9,
        ],
        float_arguments:  &[
            PReg::Xmm0,
            PReg::Xmm1,
            PReg::Xmm2,
            PReg::Xmm3,
            PReg::Xmm4,
            PReg::Xmm5,
            PReg::Xmm6,
            PReg::Xmm7,
        ],
        caller_saved:     PRegSet::of(&[
            PReg::Rax,
            PReg::Rcx,
            PReg::Rdx,
            PReg::Rsi,
            PReg::Rdi,
            PReg::R8,
            PReg::R9,
            PReg::R10,
            PReg::R11,
            PReg::Xmm0,
            PReg::Xmm1,
            PReg::Xmm2,
            PReg::Xmm3,
            PReg::Xmm4,
            PReg::Xmm5,
            PReg::Xmm6,
            PReg::Xmm7,
            PReg::Xmm8,
            PReg::Xmm9,
            PReg::Xmm10,
            PReg::Xmm11,
            PReg::Xmm12,
            PReg::Xmm13,
            PReg::Xmm14,
            PReg::Xmm15,
        ]),
        callee_saved:     PRegSet::of(&[
            PReg::Rbx,
            PReg::Rbp,
            PReg::R12,
            PReg::R13,
            PReg::R14,
            PReg::R15,
        ]),
        allocation:       &[
            PReg::Rax,
            PReg::Rcx,
            PReg::Rdx,
            PReg::Rsi,
            PReg::Rdi,
            PReg::R8,
            PReg::R9,
            PReg::Rbx,
            PReg::R12,
            PReg::R13,
            PReg::R14,
            PReg::R15,
            PReg::Rbp,
        ],
        float_allocation: &[
            PReg::Xmm0,
            PReg::Xmm1,
            PReg::Xmm2,
            PReg::Xmm3,
            PReg::Xmm4,
            PReg::Xmm5,
            PReg::Xmm6,
            PReg::Xmm7,
            PReg::Xmm8,
            PReg::Xmm9,
            PReg::Xmm10,
            PReg::Xmm11,
            PReg::Xmm12,
            PReg::Xmm13,
        ],
        home_area:        0,
    };
    pub(crate) const WIN64: Self = Self {
        kind:             ConvKind::Win64,
        arguments:        &[PReg::Rcx, PReg::Rdx, PReg::R8, PReg::R9],
        float_arguments:  &[PReg::Xmm0, PReg::Xmm1, PReg::Xmm2, PReg::Xmm3],
        caller_saved:     PRegSet::of(&[
            PReg::Rax,
            PReg::Rcx,
            PReg::Rdx,
            PReg::R8,
            PReg::R9,
            PReg::R10,
            PReg::R11,
            PReg::Xmm0,
            PReg::Xmm1,
            PReg::Xmm2,
            PReg::Xmm3,
            PReg::Xmm4,
            PReg::Xmm5,
        ]),
        callee_saved:     PRegSet::of(&[
            PReg::Rbx,
            PReg::Rbp,
            PReg::Rdi,
            PReg::Rsi,
            PReg::R12,
            PReg::R13,
            PReg::R14,
            PReg::R15,
            PReg::Xmm6,
            PReg::Xmm7,
            PReg::Xmm8,
            PReg::Xmm9,
            PReg::Xmm10,
            PReg::Xmm11,
            PReg::Xmm12,
            PReg::Xmm13,
            PReg::Xmm14,
            PReg::Xmm15,
        ]),
        allocation:       &[
            PReg::Rax,
            PReg::Rcx,
            PReg::Rdx,
            PReg::R8,
            PReg::R9,
            PReg::Rbx,
            PReg::Rsi,
            PReg::Rdi,
            PReg::R12,
            PReg::R13,
            PReg::R14,
            PReg::R15,
            PReg::Rbp,
        ],
        float_allocation: &[
            PReg::Xmm0,
            PReg::Xmm1,
            PReg::Xmm2,
            PReg::Xmm3,
            PReg::Xmm4,
            PReg::Xmm5,
            PReg::Xmm6,
            PReg::Xmm7,
            PReg::Xmm8,
            PReg::Xmm9,
            PReg::Xmm10,
            PReg::Xmm11,
            PReg::Xmm12,
            PReg::Xmm13,
        ],
        home_area:        32,
    };

    /// The convention of `target`.
    pub(crate) const fn of(target: Target) -> Self {
        match target {
            | Target::LinuxGnu | Target::LinuxMusl => Self::SYSTEM_V,
            | Target::WindowsGnu | Target::WindowsMsvc => Self::WIN64,
        }
    }

    /// The allocation order of a register class.
    pub(crate) const fn order(self, class: RegClass) -> &'static [PReg] {
        match class {
            | RegClass::Gpr => self.allocation,
            | RegClass::Xmm => self.float_allocation,
        }
    }

    /// Assigns argument positions in order: a register while any of its
    /// class is left, else the next stack slot. System V counts general and
    /// SSE registers separately; Win64 gives every argument a position, whose
    /// register is the general or SSE one of that number.
    pub(crate) fn assigner(self) -> ArgumentAssigner {
        ArgumentAssigner {
            conv:  self,
            gpr:   0,
            xmm:   0,
            stack: 0,
        }
    }
}

/// Where one argument travels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    variant_size_differences,
    reason = "A stack offset is one word; the enum is short-lived."
)]
pub(crate) enum ArgLocation {
    Reg(PReg),
    /// Byte offset from the first stack argument.
    Stack(u32),
}

/// Hands out argument locations for one call or one function's
/// parameters.
pub(crate) struct ArgumentAssigner {
    conv:  CallConv,
    /// General registers used so far (Win64: positions used).
    gpr:   usize,
    /// SSE registers used so far (System V only).
    xmm:   usize,
    /// Stack bytes used so far, Win64's register positions included.
    stack: u32,
}

impl ArgumentAssigner {
    /// The location of the next argument of `class`.
    pub(crate) fn scalar(&mut self, class: RegClass) -> ArgLocation {
        let register = match (self.conv.kind, class) {
            | (ConvKind::Win64, RegClass::Gpr) => self.conv.arguments.get(self.gpr),
            | (ConvKind::Win64, RegClass::Xmm) => self.conv.float_arguments.get(self.gpr),
            | (ConvKind::SystemV, RegClass::Gpr) => self.conv.arguments.get(self.gpr),
            | (ConvKind::SystemV, RegClass::Xmm) => self.conv.float_arguments.get(self.xmm),
        };
        if let Some(&register) = register {
            match (self.conv.kind, class) {
                | (ConvKind::Win64, _) => {
                    // A Win64 position also reserves its home slot.
                    self.gpr += 1;
                    self.stack += 8;
                },
                | (ConvKind::SystemV, RegClass::Gpr) => self.gpr += 1,
                | (ConvKind::SystemV, RegClass::Xmm) => self.xmm += 1,
            }
            return ArgLocation::Reg(register);
        }
        let offset = self.stack_offset();
        self.stack += 8;
        ArgLocation::Stack(offset)
    }

    /// The location of a System V `__int128` argument: two consecutive
    /// general registers, or a 16-byte-aligned stack slot (psABI §3.2.3:
    /// "Arguments of type __int128 ... are split into two halves ... if no
    /// register pair is available the argument is passed in memory").
    pub(crate) fn pair(&mut self) -> Result<(PReg, PReg), u32> {
        if let [low, high] = self
            .conv
            .arguments
            .get(self.gpr..self.gpr + 2)
            .unwrap_or_default()
        {
            self.gpr += 2;
            return Ok((*low, *high));
        }
        Err(self.memory(16, 16))
    }

    /// The stack location of an argument copied whole to the stack (System
    /// V `byval`), aligned to at least eight bytes.
    pub(crate) fn memory(&mut self, size: u32, align: u32) -> u32 {
        let align = align.max(8);
        self.stack = self.stack.next_multiple_of(align);
        let offset = self.stack_offset();
        self.stack += size.next_multiple_of(8);
        offset
    }

    /// Bytes of stack the arguments so far occupy beyond the registers'
    /// home area.
    pub(crate) fn stack_bytes(&self) -> u32 {
        self.stack_offset()
    }

    /// The general (Win64: positional) and SSE registers used so far.
    pub(crate) const fn registers_used(&self) -> (usize, usize) {
        (self.gpr, self.xmm)
    }

    fn stack_offset(&self) -> u32 {
        match self.conv.kind {
            // Win64 counted the four register positions' home slots.
            | ConvKind::Win64 => self.stack.saturating_sub(self.conv.home_area),
            | ConvKind::SystemV => self.stack,
        }
    }
}
