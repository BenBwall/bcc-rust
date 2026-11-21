//! The NaNs of invalid operations in a function, which take the sign of
//! the compiler's code for it only where that NaN is the value the code
//! produces. GCC and Clang fold such an operation in a function (with
//! trapping math off, and under Clang's default exception behavior), so its
//! NaN is the positive one of the compiler's arithmetic, where the
//! hardware's default NaN has the sign bit set ([`NanSource`]). When that
//! NaN then meets another NaN computed at run time in one operation, the
//! x86 hardware returns its first operand's NaN, and GCC and Clang order
//! the operands of such an operation differently from bcc's lowering (GCC
//! keeps a constant second and commutes others, and LLVM folds an operation
//! with a NaN constant operand to that constant), so no sign is right for
//! both.
//!
//! Semantic analysis therefore folds an invalid operation in a function with
//! the hardware's NaN, as if it were computed at run time, and records the
//! source its compiler would fold it with ([`Analyzer::defer_function_nan`]).
//! When the outermost function ends, [`Analyzer::settle_function_nans`] folds
//! a constant again with that source where its value is final: returned,
//! passed directly to a function that is no builtin, stored by an
//! assignment statement, or initializing an object, provided every read of
//! the object it is stored in, in the function, is final too (or passes its
//! address to a function). A constant operand of an operation computed at
//! run time, and the value of an object read by one, keep the hardware's
//! NaN, as does anything this does not follow. A value that leaves the
//! function and meets a run-time NaN in its caller is not followed.
//! C99: §F.3 paragraph 1, pp. 445-447; PDF pp. 457-459; §F.7.4 paragraph 1,
//! p. 449; PDF p. 461; §5.1.2.3 paragraph 3, p. 13; PDF p. 25 (what the
//! abstract machine leaves unevaluated).

use rustc_hash::FxBuildHasher;

use super::{
    Analyzer,
    BindingKind,
    TypeKind,
    constants::{
        Binary128Value,
        NanSource,
    },
    facts::InitOperation,
};
use crate::{
    translation_phases::parsing::{
        declaration_syntax::{
            Initializer,
            InitializerType,
        },
        syntax::{
            BinaryOperator,
            Expression,
            ExpressionType,
            UnaryOperator,
        },
    },
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// What [`Analyzer::settle_function_nans`] needs of the outermost function
/// being analyzed.
pub(super) struct FunctionNans<'tu, 's> {
    /// Each floating operation folded with the hardware's NaN whose value
    /// holds a NaN, by address, with the source its compiler folds it with.
    folds:               ArenaMap<'s, usize, NanSource>,
    /// The places whose value can be final, in analysis order.
    finals:              ArenaVec<'s, Final<'tu>>,
    /// Each object read: its binding and the address of the identifier.
    reads:               ArenaVec<'s, (usize, usize)>,
    /// The nodes of strict automatic initializers to fold again with the
    /// compiler's NaN, and the object each initializes
    /// ([`Analyzer::settle_strict_initializer`]).
    pub(super) strict:   ArenaVec<'s, (usize, &'tu Expression<'tu>)>,
    /// The parameters, by function name and position, of each function
    /// defined so far that its code reads in an operation computed at run
    /// time, which an inlined call meets the argument with; kept for the
    /// whole translation unit.
    combined_parameters: ArenaMap<'s, (StringCacheId, usize), ()>,
}

/// A place whose value can be final.
#[derive(Clone, Copy)]
enum Final<'tu> {
    /// A returned value, or an argument of a function that is no builtin.
    Value(&'tu Expression<'tu>),
    /// An argument of a builtin or library function with a floating result,
    /// which may compute it inline from its arguments: only an object whose
    /// address it passes is read finally.
    BuiltinArgument(&'tu Expression<'tu>),
    /// The left and right operands of an assignment statement.
    Store(&'tu Expression<'tu>, &'tu Expression<'tu>),
    /// Another expression statement, whose value is discarded, or an
    /// argument of a builtin or library function with another result,
    /// which only observes it: its reads are final, but a constant in it is
    /// not folded again, as the call's own folded value derives from it.
    Observed(&'tu Expression<'tu>),
    /// The initializer of an automatic object.
    Initializer(usize, &'tu Initializer<'tu>),
}

impl<'s> FunctionNans<'_, 's> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            folds:               ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            finals:              ArenaVec::new_in(scratch),
            reads:               ArenaVec::new_in(scratch),
            strict:              ArenaVec::new_in(scratch),
            combined_parameters: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
        }
    }

    fn clear(&mut self) {
        self.folds.clear();
        self.finals.clear();
        self.reads.clear();
        self.strict.clear();
    }
}

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Records that `e`, a floating operation just folded, takes the NaN of
    /// [`Self::deferred_nan_source`] where its value is final, when its
    /// value holds a NaN (`nan`).
    pub(super) fn defer_function_nan(&mut self, e: &'tu Expression<'tu>, nan: bool) {
        if nan && let Some(source) = self.deferred_nan_source() {
            _ = self
                .functions
                .nans
                .folds
                .insert(std::ptr::from_ref(e).addr(), source);
        }
    }

    /// Records `e`, an identifier that names an object, as read.
    pub(super) fn note_function_read(&mut self, binding: usize, e: &'tu Expression<'tu>) {
        if self.functions.current.is_some()
            && matches!(
                self.bindings[binding].kind,
                BindingKind::Object | BindingKind::Parameter
            )
        {
            self.functions
                .nans
                .reads
                .push((binding, std::ptr::from_ref(e).addr()));
        }
    }

    /// Records the arguments of a call to `callee`, whose own expression is
    /// analyzed: a builtin's arguments may meet each other at run time.
    pub(super) fn note_call_arguments(
        &mut self,
        callee: &'tu Expression<'tu>,
        arguments: &[&'tu Expression<'tu>],
    ) {
        if self.functions.current.is_none() {
            return;
        }
        let name = match callee.kind {
            | ExpressionType::Identifier(name) => Some(name.name),
            | _ => None,
        };
        // A builtin or library function whose result is floating may be
        // computed inline from its arguments, as `fmax` is; one with another
        // result (`printf`, `memcpy`, `__builtin_signbit`) only observes them.
        let callee_info = self.expression_info(callee);
        let floating_result = match self.types.nodes[self.types.non_atomic(callee_info.ty).index] {
            | TypeKind::Function { result, .. } => matches!(
                self.types.nodes[self.types.non_atomic(result).index],
                TypeKind::Scalar(scalar) if scalar.floating()
            ),
            | _ => true,
        };
        let builtin = name.is_some_and(|name| {
            let spelling = self.context.string_cache.at(name);
            callee_info
                .binding
                .is_some_and(|binding| self.bindings[binding].origin.is_intrinsic())
                || crate::builtins::lookup(spelling).is_some()
                || crate::builtins::ordinary(spelling).is_some()
        });
        for (index, &argument) in arguments.iter().enumerate() {
            let place = if builtin && floating_result {
                Final::BuiltinArgument(argument)
            } else if builtin {
                Final::Observed(argument)
            } else if name.is_some_and(|name| {
                self.functions
                    .nans
                    .combined_parameters
                    .contains_key(&(name, index))
            }) {
                // Not final: the reads in it stay combined.
                continue;
            } else {
                Final::Value(argument)
            };
            self.functions.nans.finals.push(place);
        }
    }

    /// Records `e`, the operand of a return statement.
    pub(super) fn note_returned_value(&mut self, e: &'tu Expression<'tu>) {
        if self.functions.current.is_some() {
            self.functions.nans.finals.push(Final::Value(e));
        }
    }

    /// Records `e`, the expression of an expression statement whose value is
    /// discarded.
    pub(super) fn note_statement_expression(&mut self, e: &'tu Expression<'tu>) {
        if self.functions.current.is_none() {
            return;
        }
        let mut value = e;
        while let ExpressionType::Parenthesized { expression } = value.kind {
            value = expression;
        }
        let place = match value.kind {
            | ExpressionType::Binary {
                operator: BinaryOperator::Assignment,
                left_expression,
                right_expression,
            } => Final::Store(left_expression, right_expression),
            | _ => Final::Observed(e),
        };
        self.functions.nans.finals.push(place);
    }

    /// Records the initializer of `binding`, an automatic object.
    pub(super) fn note_automatic_initializer(
        &mut self,
        binding: usize,
        initializer: &'tu Initializer<'tu>,
    ) {
        if self.functions.current.is_some()
            && !self.bindings[binding].duration.is_static_or_thread()
        {
            self.functions
                .nans
                .finals
                .push(Final::Initializer(binding, initializer));
        }
    }

    /// Folds again, with the source its compiler folds them with, the NaNs
    /// of the function that just ended whose value is final, once it is the
    /// outermost one, and the strict initializers of objects whose every
    /// read is final; records which of its parameters it reads in an
    /// operation computed at run time.
    pub(super) fn settle_function_nans(&mut self) {
        let mut nans = std::mem::replace(&mut self.functions.nans, FunctionNans::new(self.scratch));
        let combined = self.combined_objects(&nans);
        if let Some(function) = self.functions.current
            && let Some(name) = function.syntax.declarator.identifier()
        {
            for (index, facts) in self.function_records[function.id]
                .parameter_facts
                .iter()
                .enumerate()
            {
                if facts
                    .body_binding
                    .is_some_and(|binding| combined.contains_key(&binding))
                {
                    _ = nans.combined_parameters.insert((name.name, index), ());
                }
            }
        }
        if nans.folds.is_empty() && nans.strict.is_empty() {
            nans.clear();
            self.functions.nans = nans;
            return;
        }
        for &place in &nans.finals {
            match place {
                | Final::Value(e) => self.settle_value(e, &nans.folds),
                | Final::BuiltinArgument(_) | Final::Observed(_) => {},
                | Final::Store(target, value) =>
                    if self
                        .stored_binding(target)
                        .is_some_and(|binding| !combined.contains_key(&binding))
                    {
                        self.settle_value(value, &nans.folds);
                    },
                | Final::Initializer(binding, initializer) =>
                    if !combined.contains_key(&binding) {
                        for value in self.initializer_values(initializer) {
                            self.settle_value(value, &nans.folds);
                        }
                    },
            }
        }
        for &(binding, e) in &nans.strict {
            if !combined.contains_key(&binding) {
                self.refold(e, NanSource::Compiler);
            }
        }
        nans.clear();
        self.functions.nans = nans;
    }

    /// The objects of the function whose value may meet another computed at
    /// run time: those read anywhere but in a final value, or copied into
    /// such an object.
    fn combined_objects(&self, nans: &FunctionNans<'tu, '_>) -> ArenaMap<'s, usize, ()> {
        // The reads that are final, by address, and those that copy a value
        // into another object, with that object's binding.
        let mut final_reads = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut copies = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        for &place in &nans.finals {
            let mut copy = |into: Option<usize>, value: &'tu Expression<'tu>| {
                Self::passed_identifiers(self.scratch, value, true, |read| match into {
                    | Some(binding) => _ = copies.insert(std::ptr::from_ref(read).addr(), binding),
                    | None => {},
                });
            };
            match place {
                | Final::Value(e) | Final::BuiltinArgument(e) | Final::Observed(e) =>
                    Self::passed_identifiers(
                        self.scratch,
                        e,
                        !matches!(place, Final::BuiltinArgument(_)),
                        |read| _ = final_reads.insert(std::ptr::from_ref(read).addr(), ()),
                    ),
                | Final::Store(target, value) => {
                    if let Some(object) = Self::stored_object(target) {
                        _ = final_reads.insert(std::ptr::from_ref(object).addr(), ());
                    }
                    copy(self.stored_binding(target), value);
                },
                | Final::Initializer(binding, initializer) =>
                    for value in self.initializer_values(initializer) {
                        copy(Some(binding), value);
                    },
            }
        }
        let mut combined = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut edges = ArenaVec::new_in(self.scratch);
        for &(binding, read) in &nans.reads {
            if final_reads.contains_key(&read) {
                continue;
            }
            match copies.get(&read) {
                | Some(&into) => edges.push((binding, into)),
                | None => _ = combined.insert(binding, ()),
            }
        }
        // A copy into a combined object combines its source.
        let mut changed = true;
        while changed {
            changed = false;
            for &(from, into) in &edges {
                if combined.contains_key(&into) && combined.insert(from, ()).is_none() {
                    changed = true;
                }
            }
        }
        combined
    }

    /// Calls `found` with each identifier whose value `e` is (`values`), or
    /// whose address it is, through the operations that pass a value on
    /// without meeting another, and the pointer arithmetic that offsets an
    /// address.
    fn passed_identifiers(
        scratch: &Bump,
        e: &'tu Expression<'tu>,
        values: bool,
        mut found: impl FnMut(&'tu Expression<'tu>),
    ) {
        let mut work = ArenaVec::new_in(scratch);
        work.push((e, values));
        while let Some((e, value)) = work.pop() {
            match e.kind {
                | ExpressionType::Identifier(_) =>
                    if value {
                        found(e);
                    },
                | ExpressionType::Unary {
                    operator: UnaryOperator::AddressOf,
                    operand_expression,
                } => work.push((operand_expression, true)),
                | ExpressionType::Binary {
                    operator: BinaryOperator::Addition | BinaryOperator::Subtraction,
                    left_expression,
                    right_expression,
                } if Self::taken_address(e) => {
                    work.push((left_expression, value));
                    work.push((right_expression, value));
                },
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Cast {
                    operand_expression: expression,
                    ..
                }
                | ExpressionType::Unary {
                    operator:
                        UnaryOperator::Plus
                        | UnaryOperator::Minus
                        | UnaryOperator::BitwiseNot
                        | UnaryOperator::Extension
                        | UnaryOperator::Real
                        | UnaryOperator::Imag
                        | UnaryOperator::Indirection,
                    operand_expression: expression,
                }
                | ExpressionType::DirectMember {
                    base_expression: expression,
                    ..
                }
                | ExpressionType::IndirectMember {
                    base_expression: expression,
                    ..
                }
                | ExpressionType::Binary {
                    operator: BinaryOperator::Comma,
                    right_expression: expression,
                    ..
                } => work.push((expression, value)),
                | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    work.push((left_expression, value));
                    work.push((right_expression, value));
                },
                | ExpressionType::Conditional(c) => {
                    work.push((c.then_expression, value));
                    work.push((c.else_expression, value));
                },
                | _ => {},
            }
        }
    }

    /// Whether `e` is an address taken with `&`, through parentheses, casts
    /// and the pointer arithmetic that offsets it (`(char *)&d + 8`).
    fn taken_address(e: &'tu Expression<'tu>) -> bool {
        match e.kind {
            | ExpressionType::Unary {
                operator: UnaryOperator::AddressOf,
                ..
            } => true,
            | ExpressionType::Parenthesized { expression }
            | ExpressionType::Cast {
                operand_expression: expression,
                ..
            } => Self::taken_address(expression),
            | ExpressionType::Binary {
                operator: BinaryOperator::Addition | BinaryOperator::Subtraction,
                left_expression,
                right_expression,
            } => Self::taken_address(left_expression) || Self::taken_address(right_expression),
            | _ => false,
        }
    }

    /// The values `initializer` stores, as its plan lists them.
    fn initializer_values(
        &self,
        initializer: &'tu Initializer<'tu>,
    ) -> ArenaVec<'s, &'tu Expression<'tu>> {
        let mut values = ArenaVec::new_in(self.scratch);
        if let Some(plan) = self.initializers.plan(initializer) {
            values.extend(plan.values.iter().map(|value| value.expression));
        } else if let InitializerType::AssignmentExpression(e) = initializer.kind {
            values.push(e);
        }
        values
    }

    /// The binding of the object an assignment to `target` stores into.
    fn stored_binding(&self, target: &'tu Expression<'tu>) -> Option<usize> {
        Self::stored_object(target).and_then(|object| self.expression_info(object).binding)
    }

    /// The identifier that names the object `target`, the left operand of an
    /// assignment, stores into, or a subobject of; `None` for a store
    /// through a pointer.
    fn stored_object(target: &'tu Expression<'tu>) -> Option<&'tu Expression<'tu>> {
        let mut e = target;
        loop {
            match e.kind {
                | ExpressionType::Identifier(_) => return Some(e),
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::DirectMember {
                    base_expression: expression,
                    ..
                }
                | ExpressionType::Unary {
                    operator: UnaryOperator::Real | UnaryOperator::Imag,
                    operand_expression: expression,
                } => e = expression,
                | _ => return None,
            }
        }
    }

    /// Folds `root`, a final value, again where it derives from an operation
    /// [`Self::defer_function_nan`] recorded, with the source recorded for
    /// each; leaves it as it is when a node that derives from one is one
    /// this does not fold again.
    fn settle_value(&mut self, root: &'tu Expression<'tu>, folds: &ArenaMap<'_, usize, NanSource>) {
        if folds.is_empty() {
            return;
        }
        let mut nodes = ArenaVec::new_in(self.scratch);
        if !self.final_nodes(root, &mut nodes) {
            return;
        }
        let mut derived = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut order = ArenaVec::new_in(self.scratch);
        for &e in &nodes {
            let address = std::ptr::from_ref(e).addr();
            let is_derived = |child: &'tu Expression<'tu>| {
                derived.contains_key(&std::ptr::from_ref(child).addr())
            };
            let from_nan = folds.contains_key(&address)
                || match e.kind {
                    | ExpressionType::Binary {
                        operator: BinaryOperator::Comma,
                        right_expression,
                        ..
                    } => is_derived(right_expression),
                    | ExpressionType::Conditional(c) =>
                        is_derived(c.then_expression) || is_derived(c.else_expression),
                    | ExpressionType::CompoundLiteral { initializer, .. } =>
                        self.literal_store(initializer).is_some_and(is_derived),
                    | _ => {
                        let mut any = false;
                        super::emission::expression_children(self.scratch, e, |child| {
                            any |= is_derived(child);
                        });
                        any
                    },
                };
            if !from_nan {
                continue;
            }
            let info = self.expression_info(e);
            if info.binary128().is_some() {
                if !Self::binary128_refoldable(e) {
                    return;
                }
                order.push(e);
            } else if info.floating().is_some() {
                let literal = matches!(e.kind, ExpressionType::CompoundLiteral { .. })
                    && info.folded_literal();
                // A conversion from binary128 is not folded again.
                let from_binary128 = matches!(e.kind, ExpressionType::Cast { operand_expression, .. }
                    if self.expression_info(operand_expression).binary128().is_some());
                if (!literal && !self.refoldable(e)) || from_binary128 {
                    return;
                }
                order.push(e);
            } else if info.integer.is_some() {
                return;
            }
            _ = derived.insert(address, ());
        }
        for &e in &order {
            let source = folds
                .get(&std::ptr::from_ref(e).addr())
                .copied()
                .unwrap_or(NanSource::Runtime);
            if self.expression_info(e).binary128().is_some() {
                let value = self.binary128_refold(e, source);
                let index = self.expression_indices[&std::ptr::from_ref(e).addr()];
                self.expressions[index].set_binary128(value);
            } else if let ExpressionType::CompoundLiteral { initializer, .. } = e.kind {
                let info = self.expression_info(e);
                let value = self.literal_store(initializer).and_then(|value| {
                    self.converted_floating(self.expression_info(value), info.ty)
                });
                let index = self.expression_indices[&std::ptr::from_ref(e).addr()];
                self.expressions[index].set_floating(value);
            } else {
                self.refold(e, source);
            }
        }
    }

    /// Collects the nodes of `root`, a final value, that its value derives
    /// from without meeting a value computed at run time, each after its
    /// operands: every node of a constant, and the operands that pass a
    /// value on of one computed at run time. False for a constant this does
    /// not follow.
    fn final_nodes(
        &self,
        root: &'tu Expression<'tu>,
        nodes: &mut ArenaVec<'_, &'tu Expression<'tu>>,
    ) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((root, false));
        while let Some((e, done)) = work.pop() {
            if done {
                nodes.push(e);
                continue;
            }
            work.push((e, true));
            let info = self.expression_info(e);
            let constant =
                info.floating().is_some() || info.integer.is_some() || info.binary128().is_some();
            let mut push = |child| work.push((child, false));
            match e.kind {
                | ExpressionType::CompoundLiteral { initializer, .. } => {
                    let Some(plan) = self.initializers.plan(initializer) else {
                        return !constant;
                    };
                    for value in plan.values.iter().rev() {
                        push(value.expression);
                    }
                },
                // Unevaluated operands. C99: §6.5.3.4 paragraph 2, p. 80;
                // PDF p. 92.
                | ExpressionType::SizeofExpr(_)
                | ExpressionType::SizeofType(_)
                | ExpressionType::AlignofExpr(_)
                | ExpressionType::Countof(_) => {},
                | ExpressionType::StatementExpression(_) if constant => return false,
                | ExpressionType::Conditional(c) if constant => {
                    let condition = self.expression_info(c.condition_expression);
                    match Self::constant_truth(condition) {
                        | Some(true) => push(c.then_expression),
                        | Some(false) => push(c.else_expression),
                        | None => {
                            push(c.else_expression);
                            push(c.then_expression);
                        },
                    }
                    push(c.condition_expression);
                },
                | ExpressionType::Generic(_) | ExpressionType::Builtin(_)
                    if constant && info.selected_expression.is_some() =>
                    push(info.selected_expression.expect("checked")),
                | _ if constant => {
                    let mut children = ArenaVec::new_in(self.scratch);
                    super::emission::expression_children(self.scratch, e, |child| {
                        children.push(child);
                    });
                    for &child in children.iter().rev() {
                        push(child);
                    }
                },
                // A value computed at run time passes on these operands.
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Cast {
                    operand_expression: expression,
                    ..
                }
                | ExpressionType::Unary {
                    operator:
                        UnaryOperator::Plus
                        | UnaryOperator::Minus
                        | UnaryOperator::BitwiseNot
                        | UnaryOperator::Extension
                        | UnaryOperator::Real
                        | UnaryOperator::Imag
                        | UnaryOperator::AddressOf
                        | UnaryOperator::Indirection,
                    operand_expression: expression,
                }
                | ExpressionType::DirectMember {
                    base_expression: expression,
                    ..
                }
                | ExpressionType::IndirectMember {
                    base_expression: expression,
                    ..
                }
                | ExpressionType::Binary {
                    operator: BinaryOperator::Comma,
                    right_expression: expression,
                    ..
                } => push(expression),
                | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    push(right_expression);
                    push(left_expression);
                },
                | ExpressionType::Conditional(c) => {
                    push(c.else_expression);
                    push(c.then_expression);
                },
                | _ => {},
            }
        }
        true
    }

    /// Whether [`Self::binary128_refold`] folds `e`, a binary128 constant,
    /// again.
    fn binary128_refoldable(e: &'tu Expression<'tu>) -> bool {
        match e.kind {
            | ExpressionType::Parenthesized { .. } | ExpressionType::Cast { .. } => true,
            | ExpressionType::Unary { operator, .. } =>
                matches!(operator, UnaryOperator::Plus | UnaryOperator::Minus),
            | ExpressionType::Binary { operator, .. } => matches!(
                operator,
                BinaryOperator::Addition
                    | BinaryOperator::Subtraction
                    | BinaryOperator::Multiplication
                    | BinaryOperator::Division
            ),
            | _ => false,
        }
    }

    /// Folds `e`, a binary128 constant, again from its operands' values, as
    /// its analysis folded it but with the NaN of `source` for an invalid
    /// operation.
    fn binary128_refold(
        &self,
        e: &'tu Expression<'tu>,
        source: NanSource,
    ) -> Option<Binary128Value> {
        let info = self.expression_info(e);
        let complex = self.complex_type(self.types.non_atomic(info.ty));
        let real_or_complex = |value: Binary128Value| {
            if complex {
                value
            } else {
                Binary128Value::real(value.real)
            }
        };
        match e.kind {
            | ExpressionType::Parenthesized { expression } =>
                self.expression_info(expression).binary128(),
            | ExpressionType::Cast {
                operand_expression, ..
            } => self
                .binary128_value(self.expression_info(operand_expression))
                .map(real_or_complex),
            | ExpressionType::Unary {
                operator,
                operand_expression,
            } => self
                .binary128_value(self.expression_info(operand_expression))
                .map(|value| {
                    if operator == UnaryOperator::Minus {
                        value.negate()
                    } else {
                        value
                    }
                }),
            | ExpressionType::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                let left = self.expression_info(left_expression);
                let right = self.expression_info(right_expression);
                let domains = u8::from(self.complex_type(self.types.non_atomic(left.ty)))
                    | (u8::from(self.complex_type(self.types.non_atomic(right.ty))) << 1);
                Self::binary128_binary(
                    operator,
                    self.binary128_value(left)?,
                    self.binary128_value(right)?,
                    domains,
                    source,
                )
            },
            | _ => unreachable!("checked by binary128_refoldable"),
        }
    }

    /// The one value a scalar compound literal's `initializer` stores.
    fn literal_store(&self, initializer: &'tu Initializer<'tu>) -> Option<&'tu Expression<'tu>> {
        let plan = self.initializers.plan(initializer)?;
        match plan.operations {
            | [
                InitOperation::Store {
                    value,
                    bit_field: None,
                    ..
                },
            ] => Some(plan.values[value.0 as usize].expression),
            | _ => None,
        }
    }
}
