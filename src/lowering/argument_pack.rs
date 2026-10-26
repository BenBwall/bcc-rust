//! GNU `__builtin_va_arg_pack ()` and `__builtin_va_arg_pack_len ()`: the
//! anonymous arguments of a variadic function, and their number, at a call
//! that inlines it. glibc's `_FORTIFY_SOURCE` wrappers use them when the
//! compiler is GCC: `printf` forwards its arguments to `__printf_chk`, and
//! `open` counts them to choose between `__open_2` and `open`.
//!
//! GCC substitutes both when it inlines the function. Lowering does the
//! substitution instead, once per call: a direct call of a function whose
//! body uses them ([`FunctionRecord::va_arg_pack`]) calls a copy of it
//! specialized for the types of the call's anonymous arguments, after the
//! default argument promotions. The copy takes those arguments as extra
//! parameters; in it, a call whose last argument is the pack passes them on
//! after its own arguments, and the length is a constant. Each copy is
//! internal and `always_inline`, so the back ends expand it at its call as
//! GCC expands the original, and its constant branches are folded as soon
//! as it is lowered, at every optimization level: glibc's `open` wrapper
//! guards a call of a function with GCC's `error` attribute with
//! `__va_arg_pack_len () > 1`, which must not survive when it is false.
//! The function's own body is never lowered when it is inline-only or
//! internal, so a pack it uses never reaches a back end unresolved; an
//! external definition is lowered and reports the invalid use, as GCC does.
//!
//! For `printf (" %d %s\n", 1, "a")` against glibc's
//! `printf (const char *__fmt, ...) { return __printf_chk (1, __fmt,
//! __va_arg_pack ()); }`, the caller calls `printf.va_arg_pack (ptr, i32,
//! ptr)`, whose body calls `__printf_chk (1, fmt, p1, p2)`.
//!
//! GNU extension: GCC manual, "Constructing Calls".
//! <https://gcc.gnu.org/onlinedocs/gcc/Constructing-Calls.html>
//! C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84 (the default argument
//! promotions apply to the arguments after the ellipsis); §6.7.4
//! paragraph 6, pp. 112-113; PDF pp. 124-125 (a call of an inline function
//! may use the inline definition).

use super::{
    FunctionLowerer,
    Item,
    LoweringError,
    LoweringErrorKind,
    Operand,
    module_items::{
        Unit,
        at,
        peel,
    },
    types::Repr,
};
use crate::{
    builtins::BuiltinOperation,
    diagnostics::format_in,
    ir::{
        FuncId,
        InlineControl,
        Linkage as IrLinkage,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::Expression,
        semantic_analysis::{
            BodyEmission,
            CallFacts,
            Callee,
            FunctionRecord,
            Linkage,
            TypeId,
        },
    },
    util::bump::ArenaVec,
};

/// How deep copies may expand into copies: a function that passes its pack
/// on to itself with more arguments each time would otherwise never stop,
/// as GCC refuses recursive `always_inline` expansion.
const MAX_DEPTH: u32 = 64;

/// One specialized copy of a function that uses the pack: its definition,
/// the copy's function, and the types of the anonymous arguments it takes
/// after the named parameters.
#[derive(Clone, Copy, Debug)]
pub(super) struct PackCopy<'a, 'tu> {
    pub(super) record: &'tu FunctionRecord<'tu>,
    pub(super) func:   FuncId,
    pub(super) pack:   &'a [TypeId],
    /// How many copies the call that created this one is nested in.
    pub(super) depth:  u32,
}

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// The arguments a call passes on with `__builtin_va_arg_pack ()`, when
    /// that is its last argument and falls after the callee's named
    /// parameters (an unprototyped callee has none), as GCC recognizes it;
    /// `None` for any other call, whose pack, if any, is then evaluated as
    /// an ordinary operand and refused. Outside a copy, no call supplies the
    /// arguments, and the pack is refused here.
    pub(super) fn forwarded_pack(
        &self,
        call: CallFacts<'tu>,
    ) -> Result<Option<&'tu Expression<'tu>>, LoweringError> {
        let Some(last) = call.arguments.last() else {
            return Ok(None);
        };
        if call.arguments.len() <= call.fixed || (call.prototype && !call.variadic) {
            return Ok(None);
        }
        let Some(record) = self.unit.sema.expressions.get(last.expression.0) else {
            return Ok(None);
        };
        let argument = peel(record.expression);
        let is_pack = self.unit.sema.call_facts(argument).is_some_and(|facts| {
            matches!(
                facts.callee,
                Callee::Builtin {
                    instance: Some(instance),
                    ..
                } if instance.operation == BuiltinOperation::ArgumentPack(false)
            )
        });
        if !is_pack {
            return Ok(None);
        }
        if self.pack.is_none() {
            return Err(at(
                LoweringErrorKind::InvalidArgumentPack(false),
                argument.source_vectors,
            ));
        }
        Ok(Some(argument))
    }

    /// Pushes the anonymous arguments this copy received, for the call that
    /// passes them on. A structure, union or complex argument is copied
    /// again, since the callee may modify the object it receives
    /// (§6.9.1p10) and the pack may be passed on more than once.
    /// C99: §6.9.1 paragraph 10, p. 142; PDF p. 154.
    pub(super) fn forward_pack(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) -> Result<(), LoweringError> {
        let source = expression.source_vectors;
        let pack = self
            .pack
            .ok_or_else(|| at(LoweringErrorKind::InvalidArgumentPack(false), source))?;
        for &item in pack {
            let operand = if matches!(
                self.repr(item.ty, source)?,
                Repr::Aggregate | Repr::Complex { .. }
            ) {
                self.copy_argument(item, source)?
            } else {
                item.operand
            };
            self.push_item(operand, item.ty);
        }
        Ok(())
    }

    /// The value of `__builtin_va_arg_pack_len ()`: the number of anonymous
    /// arguments this copy received, an `int`.
    pub(super) fn pack_length(&mut self, source: SourceVectors) -> Result<Operand, LoweringError> {
        let pack = self
            .pack
            .ok_or_else(|| at(LoweringErrorKind::InvalidArgumentPack(true), source))?;
        let length = i128::try_from(pack.len()).expect("argument counts fit in i128");
        Ok(Operand::Value(self.iconst(crate::ir::Type::I32, length)))
    }

    /// The anonymous arguments of the copy being lowered, as items over its
    /// extra entry parameters, which follow the named ones at `first`.
    pub(super) fn pack_items(
        &mut self,
        pack: &[TypeId],
        first: usize,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let mut items = ArenaVec::with_capacity_in(pack.len(), self.scratch);
        for (index, &ty) in pack.iter().enumerate() {
            let value = *self
                .draft
                .block_params(super::Draft::entry())
                .get(first + index)
                .ok_or_else(|| {
                    LoweringError::missing("a pack argument has no parameter", source)
                })?;
            let operand = match self.repr(ty, source)? {
                | Repr::Aggregate | Repr::Complex { .. } => Operand::Aggregate(value),
                | _ => Operand::Value(value),
            };
            items.push(Item { operand, ty });
        }
        self.pack = Some(self.scratch.alloc_slice_copy(&items));
        Ok(())
    }
}

impl<'a, 'tu> Unit<'a, 'tu, '_> {
    /// The definition of the function `binding` designates, when its body
    /// uses the pack. Like a private inline body, it belongs to the entity:
    /// another entity labelled with the same symbol (glibc's `__REDIRECT`
    /// of `open` to `__open_alias`) is an ordinary function.
    pub(super) fn pack_function(&self, binding: usize) -> Option<&'tu FunctionRecord<'tu>> {
        if self.pack_functions.is_empty() {
            return None;
        }
        let identity = self.sema.entity(binding)?.identity;
        self.pack_functions.get(&identity).copied()
    }

    /// Whether the body of `record` is lowered only as copies for calls:
    /// it uses the pack and is inline-only or internal, so no definition
    /// with an unresolved pack has to be emitted.
    pub(super) fn copies_only(&self, record: &FunctionRecord<'tu>) -> bool {
        record.va_arg_pack
            && record.binding.is_some_and(|binding| {
                self.sema.bindings[binding].linkage == Linkage::Internal
                    || self.sema.function_policy(binding).is_some_and(|policy| {
                        matches!(
                            policy.body,
                            BodyEmission::InlineOnly | BodyEmission::PrivateInline
                        )
                    })
            })
    }

    /// The copy of `record` for a call whose arguments, named and
    /// anonymous and after their conversions, have the types `arguments`,
    /// the first `fixed` of them the named ones. It is declared, and queued
    /// for lowering, at its first call; calls with the same anonymous
    /// argument types share it. `depth` counts the copies the call is in.
    pub(super) fn pack_copy(
        &mut self,
        binding: usize,
        record: &'tu FunctionRecord<'tu>,
        arguments: &[TypeId],
        fixed: usize,
        result: TypeId,
        depth: u32,
    ) -> Result<FuncId, LoweringErrorKind> {
        let pack = &arguments[fixed..];
        if let Some(copy) = self
            .pack_copies
            .iter()
            .find(|copy| std::ptr::eq(copy.record, record) && copy.pack == pack)
        {
            return Ok(copy.func);
        }
        if depth > MAX_DEPTH {
            return Err(LoweringErrorKind::InvalidArgumentPack(false));
        }
        let signature = self.signature_of(arguments.iter().copied(), result, false, self.tables)?;
        let signature = self.module.intern(signature);
        let base = format_in!(self.tables, "{}.va_arg_pack", self.symbol_name(binding));
        let name = self.unique_name(base);
        let func = self
            .module
            .declare_function(name, signature, IrLinkage::Internal);
        self.module.function_mut(func).inlining = InlineControl::Always;
        self.module.function_mut(func).returns_twice = self
            .sema
            .entity_attributes(binding)
            .is_some_and(|attributes| attributes.returns_twice);
        self.pack_copies.push(PackCopy {
            record,
            func,
            pack: self.tables.alloc_slice_copy(pack),
            depth,
        });
        Ok(func)
    }

    /// The next declared copy still to be lowered.
    pub(super) fn next_pack_copy(&mut self) -> Option<PackCopy<'a, 'tu>> {
        let copy = self.pack_copies.get(self.lowered_pack_copies).copied()?;
        self.lowered_pack_copies += 1;
        Some(copy)
    }
}
