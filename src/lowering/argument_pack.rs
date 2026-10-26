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
//! Copies are lowered only where live code still calls them, after the
//! copies calling them are folded ([`lower_live_copies`]), so a copy
//! requested from a folded branch never emits its calls. The pack belongs
//! to the definition calls reach: a later definition replaces a GNU
//! `extern inline` body, and a call through `*`, `&` or a same-type cast of
//! the function's designator is as direct as one through its name. The
//! function's own body is not lowered when it is inline-only, nor when it is
//! internal unless live code takes its address; an external definition is
//! lowered, and either reports a pack it evaluates as an invalid use, as GCC
//! does.
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
    LoweringOptions,
    Operand,
    module_items::{
        Unit,
        at,
    },
    types::Repr,
};
use crate::{
    builtins::BuiltinOperation,
    diagnostics::format_in,
    ir::{
        FuncId,
        GlobalInit,
        InlineControl,
        InstData,
        Linkage as IrLinkage,
        RelocationTarget,
        Symbol,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            Expression,
            ExpressionType,
            UnaryOperator,
        },
        semantic_analysis::{
            BodyEmission,
            CallFacts,
            Callee,
            FunctionRecord,
            Linkage,
            TypeId,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaSet,
        ArenaVec,
        Bump,
    },
};

/// How deep copies may expand into copies: a function that passes its pack
/// on to itself with more arguments each time would otherwise never stop,
/// as GCC refuses recursive `always_inline` expansion.
const MAX_DEPTH: u32 = 64;

/// Lowers the copies live code calls, once every function definition is
/// lowered. Liveness starts where the program can enter: functions other
/// modules can name, `used` ones, inline copies (the LLVM back end emits
/// their bodies), constructors and destructors, and functions static
/// initializers address, and the targets of aliases; it follows the calls
/// and addresses of each body
/// it reaches. A copy is lowered when it is reached, and its branches on
/// its pack's length are folded at once, as GCC folds them when it
/// inlines, at every optimization level, before its own calls are
/// followed. So a copy that only a folded branch calls, or only an
/// internal function that nothing live reaches, is never lowered: its
/// calls (of undefined or `error` functions, or of ever deeper copies) are
/// never emitted, as GCC never expands an inline call it folds away, and it
/// stays an internal declaration that nothing names. The bodies of internal
/// functions left unreached are dropped for the same reason, as GCC drops
/// unused static functions when it optimizes.
///
/// Depths are relaxed to a fixed point: an ordinary function is at depth 0,
/// and a copy one deeper than the shallowest live body calling it. A
/// function's calls are followed again whenever a shallower path to it is
/// found (each copy is lowered once), and a copy is checked against
/// [`MAX_DEPTH`] only on its final, shallowest live depth, whatever order
/// the walk found its callers in. An
/// internal function using the pack whose address live code takes gets its
/// own body, as GCC emits it then: one that evaluates no pack (it only
/// names one in `sizeof` or an unselected generic association) lowers, and
/// any other refuses its pack as an invalid use. C99: §5.1.2.3p5, pp. 13-14;
/// PDF pp. 25-26 (only observable behavior must be preserved). GNU extension:
/// GCC manual, "Constructing Calls".
pub(super) fn lower_live_copies<'ir>(
    unit: &mut Unit<'_, '_, 'ir>,
    scratch: &mut Bump,
    options: &mut LoweringOptions,
    errors: &mut ArenaVec<'_, LoweringError>,
    sources: &mut ArenaVec<'_, (&'ir str, SourceVectors)>,
) {
    if unit.pack_copies.is_empty() && unit.pack_addresses.is_empty() {
        return;
    }
    let tables = unit.tables;
    let module = &unit.module;
    let mut depths = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables);
    let mut buckets = ArenaVec::new_in(tables);
    buckets.resize_with(MAX_DEPTH as usize + 2, || ArenaVec::new_in(tables));
    // A function joins the bucket of each smaller depth found for it.
    let reach = |func: FuncId,
                 depth: u32,
                 depths: &mut ArenaMap<'_, FuncId, u32>,
                 buckets: &mut ArenaVec<'_, ArenaVec<'_, FuncId>>| {
        if depths.get(&func).is_none_or(|&known| depth < known) {
            _ = depths.insert(func, depth);
            buckets[depth as usize].push(func);
        }
    };
    let mut roots = ArenaVec::new_in(tables);
    for (func, function) in module.functions() {
        // An alias, even an internal one, is emitted and needs its target.
        if function.linkage != IrLinkage::Internal
            || function.attributes.used
            || function.attributes.alias.is_some()
            || function.inline_copy_of.is_some()
        {
            roots.push(module.function_definition(func));
        }
    }
    roots.extend(module.init_functions().iter().map(|init| init.func));
    for (_, global) in module.globals() {
        if let Some(GlobalInit::Bytes { relocations, .. }) = global.init {
            roots.extend(
                relocations
                    .iter()
                    .filter_map(|relocation| match relocation.target {
                        | RelocationTarget::Symbol(Symbol::Function(func))
                        | RelocationTarget::Label { func, .. }
                        | RelocationTarget::LabelDifference { func, .. } =>
                            Some(module.function_definition(func)),
                        | _ => None,
                    }),
            );
        }
    }
    for &func in &roots {
        reach(func, 0, &mut depths, &mut buckets);
    }
    // The depth each function's calls were last followed at, and the
    // functions lowered here.
    let mut followed = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables);
    let mut lowered = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables);
    let mut edges = ArenaVec::new_in(tables);
    while let Some(depth) = buckets.iter().position(|bucket| !bucket.is_empty()) {
        let func = buckets[depth].pop().expect("the bucket is not empty");
        let depth = u32::try_from(depth).expect("depths are bounded");
        if depths.get(&func).is_some_and(|&known| known < depth)
            || followed.get(&func).is_some_and(|&done| done <= depth)
        {
            continue;
        }
        if let Some(&index) = unit.pack_copy_index.get(&func) {
            // Too deep for now; a shallower live call may still reach it,
            // and the limit is checked on its final depth below.
            if depth > MAX_DEPTH {
                continue;
            }
            if lowered.insert(func) {
                let copy = unit.pack_copies[index];
                scratch.reset();
                if let Some(binding) = copy.record.binding {
                    sources.push((
                        unit.module.function(func).name,
                        unit.sema.bindings[binding].name.source_vectors,
                    ));
                }
                match FunctionLowerer::lower(unit, copy.record, func, Some(copy), scratch, options)
                {
                    | Ok(()) =>
                        crate::optimizer::fold_constant_branches(&mut unit.module, func, scratch),
                    | Err(error) =>
                        if !errors.contains(&error) {
                            errors.push(error);
                        },
                }
            }
        } else if let Some(&record) = unit.pack_addresses.get(&func)
            && lowered.insert(func)
        {
            // Its address is live, or it is `used`, so its own body is
            // emitted, as GCC emits it: a pack it evaluates is then refused
            // where it is.
            scratch.reset();
            if let Err(error) = FunctionLowerer::lower(unit, record, func, None, scratch, options)
                && !errors.contains(&error)
            {
                errors.push(error);
            }
        }
        _ = followed.insert(func, depth);
        let module = &unit.module;
        edges.clear();
        if let Some(body) = &module.function(func).body {
            for block in body.blocks() {
                for &inst in body.block_insts(block) {
                    if let InstData::Call { func: callee, .. }
                    | InstData::FuncAddr { func: callee } = *body.inst(inst)
                    {
                        edges.push(module.function_definition(callee));
                    }
                }
            }
        }
        for &callee in &edges {
            let next = if unit.pack_copy_index.contains_key(&callee) {
                (depth + 1).min(MAX_DEPTH + 1)
            } else {
                0
            };
            reach(callee, next, &mut depths, &mut buckets);
        }
    }
    // A live copy no live path reaches within the limit.
    for copy in &unit.pack_copies {
        if depths
            .get(&copy.func)
            .is_some_and(|&depth| depth > MAX_DEPTH)
        {
            let error = at(LoweringErrorKind::InvalidArgumentPack(false), copy.source);
            if !errors.contains(&error) {
                errors.push(error);
            }
        }
    }
    let unreached = tables.alloc_slice_fill_iter(
        unit.module
            .functions()
            .filter(|&(func, function)| {
                function.linkage == IrLinkage::Internal
                    && function.body.is_some()
                    && !followed.contains_key(&func)
            })
            .map(|(func, _)| func),
    );
    for &func in &*unreached {
        unit.module.function_mut(func).body = None;
    }
}

/// One specialized copy of a function that uses the pack: its definition,
/// the copy's function, and the types of the anonymous arguments it takes
/// after the named parameters.
#[derive(Clone, Copy, Debug)]
pub(super) struct PackCopy<'a, 'tu> {
    pub(super) record: &'tu FunctionRecord<'tu>,
    pub(super) func:   FuncId,
    pub(super) pack:   &'a [TypeId],
    /// The first call of it, where a copy nested too deeply is reported.
    pub(super) source: SourceVectors,
}

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// The arguments a call passes on with `__builtin_va_arg_pack ()`, when
    /// that is its last argument and falls after the callee's named
    /// parameters (an unprototyped callee has none), as GCC recognizes it,
    /// through what GCC folds away around it ([`Self::transparent`]);
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
        let argument = self.transparent(record.expression);
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

    /// `expression` without what GCC folds away around a call before it
    /// looks for the pack: parentheses, `__extension__`, a cast or unary `+`
    /// to the operand's own type, and the operand `_Generic` or
    /// `__builtin_choose_expr` selects, whose other operands are not
    /// evaluated. GCC also folds arithmetic such as `0 + pack` and discards
    /// the void left operand of a comma; bcc does not, and refuses those.
    /// C99: §6.5.1p5, p. 67; PDF p. 79 (a parenthesized expression is its
    /// operand); §6.5.3.3p2, p. 79; PDF p. 91 (unary `+`); §6.5.4p4, p. 81;
    /// PDF p. 93 (a cast converts to the named type). C11: §6.5.1.1p3,
    /// p. 79; PDF p. 97 (`_Generic`).
    pub(super) fn transparent(&self, mut expression: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
        loop {
            expression = match expression.kind {
                | ExpressionType::Parenthesized { expression: inner }
                | ExpressionType::Unary {
                    operator: UnaryOperator::Extension,
                    operand_expression: inner,
                } => inner,
                | ExpressionType::Cast {
                    operand_expression, ..
                }
                | ExpressionType::Unary {
                    operator: UnaryOperator::Plus,
                    operand_expression,
                } if self.same_type(expression, operand_expression) => operand_expression,
                | ExpressionType::Generic(_) | ExpressionType::Builtin(_) => {
                    match self
                        .info(expression)
                        .ok()
                        .and_then(|info| info.selected_expression)
                    {
                        | Some(selected) => selected,
                        | None => return expression,
                    }
                },
                | _ => return expression,
            };
        }
    }

    /// Whether `expression` has the unqualified type of `operand`.
    fn same_type(&self, expression: &'tu Expression<'tu>, operand: &'tu Expression<'tu>) -> bool {
        let ty = |expression| self.info(expression).ok().map(|info| info.ty.unqualified());
        ty(expression).is_some() && ty(expression) == ty(operand)
    }

    /// The function a call whose callee is not a plain identifier designates,
    /// when it uses the pack: `(*f)(...)`, `(&f)(...)` and
    /// `((int (*)(int, ...))f)(...)`, a cast to a pointer to `f`'s own type,
    /// call `f` as directly as `f(...)`, so they call its copies too.
    /// C99: §6.5.3.2p4, p. 79; PDF p. 91 (`*` of a function designator is
    /// that designator); §6.3.2.1p4, p. 46; PDF p. 58 (a function
    /// designator converts to a pointer to the function); §6.5.2.2p3,
    /// p. 71; PDF p. 83.
    pub(super) fn packed_callee(&self, call: &'tu Expression<'tu>) -> Option<usize> {
        let ExpressionType::Call {
            function_expression: mut callee,
            ..
        } = call.kind
        else {
            return None;
        };
        loop {
            callee = match self.transparent(callee).kind {
                | ExpressionType::Unary {
                    operator: UnaryOperator::AddressOf | UnaryOperator::Indirection,
                    operand_expression,
                } => operand_expression,
                | ExpressionType::Cast {
                    operand_expression, ..
                } if self.points_to_type_of(self.transparent(callee), operand_expression) =>
                    operand_expression,
                | ExpressionType::Identifier(_) => break,
                | _ => return None,
            };
        }
        let binding = self.info(self.transparent(callee)).ok()?.binding?;
        self.unit.pack_function(binding).map(|_| binding)
    }

    /// Whether the pointer `expression` points to the type of the function
    /// designator `operand`, as a cast of its decayed address does.
    fn points_to_type_of(
        &self,
        expression: &'tu Expression<'tu>,
        operand: &'tu Expression<'tu>,
    ) -> bool {
        let (Ok(pointer), Ok(function)) = (self.info(expression), self.info(operand)) else {
            return false;
        };
        super::types::target_type(&self.unit.sema.types, pointer.ty)
            .is_some_and(|target| target.unqualified() == function.ty.unqualified())
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

impl<'tu> Unit<'_, 'tu, '_> {
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

    /// The copy of `record` for a call at `source` whose arguments, named
    /// and anonymous and after their conversions, have the types
    /// `arguments`, the first `fixed` of them the named ones. It is declared
    /// at its first call, and calls with the same anonymous argument types
    /// share it; [`lower_live_copies`] lowers it if live code still calls
    /// it once lowering ends.
    pub(super) fn pack_copy(
        &mut self,
        binding: usize,
        record: &'tu FunctionRecord<'tu>,
        arguments: &[TypeId],
        fixed: usize,
        result: TypeId,
        source: SourceVectors,
    ) -> Result<FuncId, LoweringErrorKind> {
        let pack = &arguments[fixed..];
        if let Some(copy) = self
            .pack_copies
            .iter()
            .find(|copy| std::ptr::eq(copy.record, record) && copy.pack == pack)
        {
            return Ok(copy.func);
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
        _ = self.pack_copy_index.insert(func, self.pack_copies.len());
        self.pack_copies.push(PackCopy {
            record,
            func,
            pack: self.tables.alloc_slice_copy(pack),
            source,
        });
        Ok(func)
    }
}
