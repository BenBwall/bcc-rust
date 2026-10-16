//! GNU cleanup calls. Semantic analysis retains each `cleanup` variable as a
//! region from the end of its initializer to the end of its block. Every
//! transition that leaves the region calls its function with the variable's
//! address: falling off the block, `break`, `continue`, `return` (after the
//! returned value is computed), `goto` and the end of a statement
//! expression. Indirect gotos never cross a cleanup region (semantic
//! analysis rejects them), and a goto may enter one only from the
//! variable's own initializer. `vla.rs` walks a transition's left regions
//! innermost first, interleaving VLA releases, and asks this file for each
//! call. The call's result is discarded.
//! GNU extension: GCC manual, "Common Variable Attributes", cleanup.
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Variable-Attributes.html>
//! C99: §6.2.4p5, p. 32; PDF p. 44 (an automatic object's lifetime ends when
//! its block ends).

use super::{
    FunctionLowerer,
    Local,
    LoweringError,
    module_items::at,
    ssa::Op,
};
use crate::{
    ir::{
        InstData,
        ParamAbi,
        Type,
    },
    translation_phases::SourceVectors,
    util::bump::ArenaVec,
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Calls the cleanup function `function` with the address of the
    /// automatic object `variable`, whose storage semantic analysis marked as
    /// address-taken.
    pub(super) fn call_cleanup(
        &mut self,
        variable: usize,
        function: usize,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let address = match self.local(variable, source)? {
            | Local::Slot(slot) => self.inst(InstData::StackAddr { slot }, Type::Ptr),
            | Local::Address(address) => address,
            | Local::Variable(_) =>
                return Err(LoweringError::missing(
                    "a cleanup variable has no storage address",
                    source,
                )),
        };
        // A cleanup function is a direct call, so it reaches a private
        // `always_inline` body as one does.
        let func = self
            .unit
            .direct_callee(function)
            .map_err(|kind| at(kind, source))?;
        let signature = self.unit.module.function_signature(func);
        let mut args = ArenaVec::with_capacity_in(2, self.scratch);
        if let Some(ParamAbi::Result(layout)) = signature.result_param() {
            let object = self.fresh_object(layout.size, layout.align, source)?;
            args.push(object);
        }
        args.push(address);
        let args = self.scratch.alloc_slice_copy(&args);
        _ = self.draft.push(Op::Call(func, args), signature.result);
        Ok(())
    }
}
