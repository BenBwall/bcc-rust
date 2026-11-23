//! The runtime functions lowering calls by name on its own: the C library's
//! `pow` for `__builtin_powi` on Windows, libgcc's `__muldc3` and
//! `__cpu_indicator_init`, and `_setjmp` and `longjmp` for a jump out of a
//! nested function. The unit may already use the name. An external
//! function of that name is the runtime's, or one the program puts in its
//! place, so the call reaches it as Clang's and GCC's call by name does:
//! directly when its type is the routine's, else through its address with
//! the routine's type, since IR calls must match their callee. A function
//! with internal linkage, or an object, is something else that only
//! shares the name, as a file that does not include the header may give
//! `pow` to a static function. Lowering refuses the call then, where Clang
//! calls the static function with the routine's arguments.
//! C99: §7.1.3 paragraphs 1-2, p. 166; PDF p. 178.

use super::{
    FunctionLowerer,
    LoweringError,
    LoweringErrorKind,
    errors::{
        RuntimeCaller,
        TakenName,
    },
    module_items::Unit,
    ssa::Op,
};
use crate::{
    ir::{
        FuncId,
        InstData,
        Linkage,
        SigId,
        Signature,
        Symbol,
        Type,
        Value,
    },
    translation_phases::SourceVectors,
};

/// A runtime function as lowering calls it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct RuntimeFunction {
    pub(super) func:    FuncId,
    /// The routine's signature when the unit declares the function with
    /// another type: the call goes through its address.
    pub(super) retyped: Option<SigId>,
}

impl Unit<'_, '_, '_> {
    /// The external function `name` with `signature`, declared on first
    /// use. `caller` names what needs it, for the refusal when the unit
    /// gives the name to a function with internal linkage or an object.
    pub(super) fn runtime_function(
        &mut self,
        name: &'static str,
        signature: Signature<'_>,
        caller: RuntimeCaller,
        source: SourceVectors,
    ) -> Result<RuntimeFunction, LoweringError> {
        let refuse = |taken| LoweringError {
            kind:   LoweringErrorKind::RuntimeName {
                routine: name,
                caller,
                taken,
            },
            source: Some(source),
        };
        let wanted = self.module.intern(signature);
        match self.module.symbol(name) {
            | None => Ok(RuntimeFunction {
                func:    self
                    .module
                    .declare_function(name, wanted, Linkage::External),
                retyped: None,
            }),
            | Some(Symbol::Function(func))
                if self.module.function(func).linkage == Linkage::Internal =>
                Err(refuse(TakenName::InternalFunction)),
            | Some(Symbol::Function(func)) => Ok(RuntimeFunction {
                func,
                retyped: self.retyped(func, wanted),
            }),
            | Some(Symbol::Global(_)) => Err(refuse(TakenName::Object)),
        }
    }

    /// `wanted` when the function `func` is declared with other types than
    /// it gives; attributes that leave the types alone, like `noundef`,
    /// do not count.
    pub(super) fn retyped(&self, func: FuncId, wanted: SigId) -> Option<SigId> {
        let declared = self.module.function_signature(func);
        let wanted_signature = self.module.signature(wanted);
        let same = (
            declared.params,
            declared.result,
            declared.variadic,
            declared.aggregates,
        ) == (
            wanted_signature.params,
            wanted_signature.result,
            wanted_signature.variadic,
            wanted_signature.aggregates,
        );
        (!same).then_some(wanted)
    }
}

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Calls the runtime function `callee` with `args`, giving a value of
    /// type `result`.
    pub(super) fn call_runtime(
        &mut self,
        callee: RuntimeFunction,
        args: &[Value],
        result: Option<Type>,
    ) -> Option<Value> {
        let args = self.scratch.alloc_slice_copy(args);
        match callee.retyped {
            | None => self.draft.push(Op::Call(callee.func, args), result),
            | Some(signature) => {
                let address = self.inst(InstData::FuncAddr { func: callee.func }, Type::Ptr);
                self.draft
                    .push(Op::CallIndirect(signature, address, args), result)
            },
        }
    }
}
