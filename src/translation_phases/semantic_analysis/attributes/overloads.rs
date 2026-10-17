//! Overload declarations keep source names and distinct entity identities.
//! Candidate selection checks arity, conversions and `enable_if` before the
//! chosen candidate alone commits conversions and call diagnostics.
//! C99: §6.5.2.2p2,6-7, pp. 71-72; PDF pp. 83-84 (extended call rules).
//! <https://clang.llvm.org/docs/AttributeReference.html#overloadable>

use super::super::{
    Analyzer,
    Binding,
    BindingKind,
    BindingOrigin,
    Duration,
    Expression,
    ExpressionInfo,
    ExtensionCall,
    FunctionExtension,
    Linkage,
    Namespace,
    ObjectSize,
    PredicateKind,
    Scalar,
    SemanticErrorKind,
    SizeArgument,
    TypeId,
    TypeKind,
};
use crate::{
    translation_phases::parsing::syntax::Identifier,
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaSet,
            ArenaVec,
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(in crate::translation_phases::semantic_analysis) fn bind_overload(
        &mut self,
        name: Identifier,
        mut ty: TypeId,
        kind: BindingKind,
        linkage: Linkage,
        duration: Duration,
        visible: Option<usize>,
    ) -> bool {
        if kind != BindingKind::Function {
            return false;
        }
        let index = self.bindings.len();
        let contract = self.function_extensions.function(index);
        let Some(previous) = visible else {
            return false;
        };
        let old = self.bindings[previous];
        if old.kind != kind {
            return false;
        }
        let enabled = |extension: Option<&FunctionExtension<'_>>| {
            extension.is_some_and(|e| {
                e.overloadable || e.predicates.iter().any(|p| p.kind == PredicateKind::Enable)
            })
        };
        if !enabled(contract)
            && !enabled(self.function_extensions.function(previous))
            && !self.overload_previous.contains_key(&previous)
        {
            let old_sizes = self
                .function_extensions
                .function(previous)
                .map_or(&[][..], |c| c.sizes);
            let sizes = contract.map_or(&[][..], |c| c.sizes);
            if old_sizes
                .iter()
                .map(|s| (s.index, s.mode, s.dynamic))
                .ne(sizes.iter().map(|s| (s.index, s.mode, s.dynamic)))
            {
                self.error(
                    SemanticErrorKind::IncompatibleDeclaration,
                    name.source_vectors,
                    Some(name.name),
                    Some(old.name.source_vectors),
                );
            }
            return false;
        }
        let identity = contract.map_or(name.name, |c| c.identity);
        let mut candidate = Some(previous);
        while let Some(binding) = candidate {
            let prior = self.bindings[binding];
            if self.types.composite(prior.ty, ty).is_some() {
                let previous_contract = self.function_extensions.function(binding);
                let sizes = contract.map_or(&[][..], |c| c.sizes);
                let previous_sizes = previous_contract.map_or(&[][..], |c| c.sizes);
                if sizes.is_empty() == previous_sizes.is_empty()
                    && contract.is_some_and(|c| c.overloadable)
                        != previous_contract.is_some_and(|c| c.overloadable)
                {
                    self.error(
                        SemanticErrorKind::IncompatibleDeclaration,
                        name.source_vectors,
                        Some(name.name),
                        Some(prior.name.source_vectors),
                    );
                    break;
                }
            }
            if self.function_identity(binding) == identity {
                if let Some(composite) = self.types.composite(prior.ty, ty) {
                    ty = composite;
                } else {
                    self.error(
                        SemanticErrorKind::IncompatibleDeclaration,
                        name.source_vectors,
                        Some(name.name),
                        Some(prior.name.source_vectors),
                    );
                }
                break;
            }
            if self.types.composite(prior.ty, ty).is_some()
                && let (Some(a), Some(b)) = (contract, self.function_extensions.function(binding))
                && !a.sizes.is_empty()
                && !b.sizes.is_empty()
                && a.sizes
                    .iter()
                    .map(|s| (s.index, s.mode, s.dynamic))
                    .ne(b.sizes.iter().map(|s| (s.index, s.mode, s.dynamic)))
            {
                self.error(
                    SemanticErrorKind::IncompatibleDeclaration,
                    name.source_vectors,
                    Some(name.name),
                    Some(prior.name.source_vectors),
                );
                break;
            }
            candidate = self.overload_previous.get(&binding).copied();
        }
        self.bindings.push(Binding {
            origin: BindingOrigin::User,
            name,
            ty,
            scope: self.scope,
            kind,
            linkage,
            duration,
            value: None,
        });
        if old.scope == self.scope {
            _ = self.overload_previous.insert(index, previous);
        }
        self.install(name, Namespace::Ordinary, index);
        if linkage != Linkage::None {
            _ = self.external.insert(identity, index);
        }
        true
    }

    pub(in crate::translation_phases::semantic_analysis) fn select_extension_call(
        &mut self,
        expression: &'tu Expression<'tu>,
        function: ExpressionInfo<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ExpressionInfo<'tu> {
        let Some(anchor) = function
            .binding
            .filter(|&b| self.bindings[b].kind == BindingKind::Function)
        else {
            return function;
        };
        if self.function_extensions.function(anchor).is_none()
            && !self.overload_previous.contains_key(&anchor)
        {
            return function;
        }
        let mut viable = ArenaVec::new_in(self.scratch);
        let mut seen = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.scratch);
        let mut candidate = Some(anchor);
        let mut has_extensions = false;
        while let Some(binding) = candidate {
            let contract = self.function_extensions.function(binding);
            has_extensions |= contract.is_some();
            if seen.insert(self.function_identity(binding))
                && let Some(rank) = self.candidate_rank(binding, contract, arguments.as_slice())
            {
                viable.push((binding, rank));
            }
            candidate = self.overload_previous.get(&binding).copied();
        }
        if !has_extensions {
            return function;
        }
        let mut best = viable.iter().filter(|&&(binding, rank)| {
            !viable.iter().any(|&(other, other_rank)| {
                other != binding
                    && self.better_candidate(other, other_rank, binding, rank, arguments.as_slice())
            })
        });
        let Some(&(binding, _)) = best.next() else {
            self.error(
                SemanticErrorKind::NoViableOverload,
                expression.source_vectors,
                Some(self.bindings[anchor].name.name),
                Some(self.bindings[anchor].name.source_vectors),
            );
            return ExpressionInfo {
                ty: self.types.unknown(),
                ..function
            };
        };
        if best.next().is_some() {
            self.error(
                SemanticErrorKind::AmbiguousOverload,
                expression.source_vectors,
                Some(self.bindings[anchor].name.name),
                None,
            );
            return ExpressionInfo {
                ty: self.types.unknown(),
                ..function
            };
        }
        let mut result = function;
        result.binding = Some(binding);
        result.ty = self.bindings[binding].ty;
        let mut sizes = ArenaVec::new_in(self.scratch);
        if let Some(contract) = self.function_extensions.function(binding) {
            for size in contract.sizes {
                if let Some(&argument) = arguments.get(size.index as usize) {
                    let value = match self.infer_object_size(argument, size.mode, size.dynamic) {
                        | ObjectSize::Unknown =>
                            ObjectSize::Constant(if size.mode >= 2 { 0 } else { u64::MAX }),
                        | value => value,
                    };
                    sizes.push(SizeArgument {
                        parameter: size.index,
                        value,
                    });
                }
            }
            if let Some(message) = contract.unavailable {
                self.call_attribute_diagnostic(expression.source_vectors, message, false);
            }
            for predicate in contract.predicates {
                if predicate.kind != PredicateKind::Enable
                    && self
                        .predicate_value(
                            predicate.parameters,
                            predicate.expression,
                            arguments.as_slice(),
                        )
                        .is_some_and(|value| value.value != 0)
                {
                    self.call_attribute_diagnostic(
                        expression.source_vectors,
                        predicate.message,
                        predicate.kind == PredicateKind::Warning,
                    );
                }
            }
        }
        _ = self.function_extensions.calls.insert(
            std::ptr::from_ref(expression).addr(),
            ExtensionCall {
                binding,
                sizes: self.types.tu.alloc_slice_copy(&sizes),
            },
        );
        _ = self.function_extensions.values.insert(
            std::ptr::from_ref(self.selected_designator(function.expression)).addr(),
            binding,
        );
        _ = self
            .function_extensions
            .values
            .insert(std::ptr::from_ref(function.expression).addr(), binding);
        let mut usage = self.bindings[binding].name;
        usage.source_vectors = expression.source_vectors;
        self.record_binding_use(binding, usage);
        result
    }

    /// Clang extends C99 promotions to overload ranking: unlike C++, float
    /// and double promote to long double and the corresponding complex types.
    /// Sequences rank exact, promotion, conversion, real/complex, ellipsis,
    /// then incompatible pointers. Qualification remains an exact match.
    /// C99: §6.3.1.5p1, p. 44; PDF p. 56 (extended conversion ranking).
    fn candidate_rank(
        &mut self,
        binding: usize,
        contract: Option<&FunctionExtension<'tu>>,
        arguments: &[&'tu Expression<'tu>],
    ) -> Option<&'tu [u8]> {
        let TypeKind::Function {
            parameters,
            prototype,
            variadic,
            ..
        } = self.types.nodes[self.bindings[binding].ty.index]
        else {
            return None;
        };
        if prototype
            && (arguments.len() < parameters.len()
                || (!variadic && arguments.len() != parameters.len()))
        {
            return None;
        }
        let mut rank = ArenaVec::new_in(self.scratch);
        for (index, &argument) in arguments.iter().enumerate() {
            let info = self.expression_info(argument);
            let Some(&parameter) = parameters.get(index) else {
                rank.push(4);
                continue;
            };
            let parameter = self.types.non_atomic(parameter);
            if let Some(anchor) = info.binding.filter(|&binding| {
                self.bindings[binding].kind == BindingKind::Function
                    && (self.overload_previous.contains_key(&binding)
                        || self.function_extensions.function(binding).is_some())
            }) && let Some(target) = self.pointer_target(parameter)
                && matches!(self.types.nodes[target.index], TypeKind::Function { .. })
            {
                let (selected, ambiguous) = self.function_value_candidate(anchor, Some(target));
                if selected.is_none() || ambiguous {
                    return None;
                }
                rank.push(0);
                continue;
            }
            let (from, _) = self.value_conversion(info);
            if from.index == parameter.index {
                rank.push(0);
                continue;
            }
            if !self.assignable(parameter, info, from) {
                if self.pointer_target(from).is_some() && self.pointer_target(parameter).is_some() {
                    // Clang keeps incompatible pointer conversions as a last
                    // resort, below even an ellipsis. The chosen call still
                    // receives the ordinary pointer-constraint diagnostic.
                    rank.push(5);
                    continue;
                }
                if self.transparent_union_member(parameter, info).is_some() {
                    rank.push(2);
                    continue;
                }
                return None;
            }
            if let (Some(from), Some(to)) =
                (self.pointer_target(from), self.pointer_target(parameter))
                && self.same_qualification_shape(from, to)
                && !self.null_pointer_constant(info)
            {
                rank.push(0);
                continue;
            }
            let promotion = self.integer_type(from).is_some()
                && self.promote(info, from).index == parameter.index;
            let from_scalar = self.types.nodes[from.index];
            let to_scalar = self.types.nodes[parameter.index];
            let floating_promotion = matches!(
                (from_scalar, to_scalar),
                (
                    TypeKind::Scalar(Scalar::Float),
                    TypeKind::Scalar(Scalar::Double | Scalar::LongDouble | Scalar::Float128)
                ) | (
                    TypeKind::Scalar(Scalar::Double),
                    TypeKind::Scalar(Scalar::LongDouble | Scalar::Float128)
                ) | (
                    TypeKind::Scalar(Scalar::ComplexFloat),
                    TypeKind::Scalar(
                        Scalar::ComplexDouble | Scalar::ComplexLongDouble | Scalar::ComplexFloat128
                    )
                ) | (
                    TypeKind::Scalar(Scalar::ComplexDouble),
                    TypeKind::Scalar(Scalar::ComplexLongDouble | Scalar::ComplexFloat128)
                )
            );
            let complex = |ty| {
                matches!(
                    ty,
                    TypeKind::Scalar(
                        Scalar::ComplexFloat
                            | Scalar::ComplexDouble
                            | Scalar::ComplexLongDouble
                            | Scalar::ComplexFloat128
                    )
                )
            };
            rank.push(if promotion || floating_promotion {
                1
            } else if complex(from_scalar) != complex(to_scalar)
                && !matches!(to_scalar, TypeKind::Scalar(Scalar::Bool))
            {
                3
            } else {
                2
            });
        }
        if let Some(contract) = contract {
            for predicate in contract
                .predicates
                .iter()
                .filter(|p| p.kind == PredicateKind::Enable)
            {
                if self
                    .predicate_value(predicate.parameters, predicate.expression, arguments)?
                    .value
                    == 0
                {
                    return None;
                }
            }
        }
        Some(self.types.tu.alloc_slice_copy(&rank))
    }

    // A conversion must be no worse for every written argument and better
    // for at least one. Extension preference applies only to tied sequences.
    fn better_candidate(
        &mut self,
        a: usize,
        ar: &[u8],
        b: usize,
        br: &[u8],
        arguments: &[&'tu Expression<'tu>],
    ) -> bool {
        // Ill-formed conversions are compared before ordinary sequence ranks,
        // so a better second argument cannot rescue a bad pointer conversion.
        if ar.iter().zip(br).any(|(&a, &b)| a == 5 && b != 5) {
            return false;
        }
        if ar.iter().zip(br).any(|(&a, &b)| a != 5 && b == 5) {
            return true;
        }
        let parameters = |binding: usize| match self.types.nodes[self.bindings[binding].ty.index] {
            | TypeKind::Function { parameters, .. } => parameters,
            | _ => &[],
        };
        let ap = parameters(a);
        let bp = parameters(b);
        let mut better = false;
        for (index, (&a, &b)) in ar.iter().zip(br).enumerate() {
            let order = a.cmp(&b).then_with(|| {
                let (Some(&a), Some(&b)) = (ap.get(index), bp.get(index)) else {
                    return std::cmp::Ordering::Equal;
                };
                let info = self.expression_info(arguments[index]);
                let (from, _) = self.value_conversion(info);
                self.compare_overload_conversion(info, from, a, b)
            });
            if order.is_gt() {
                return false;
            }
            better |= order.is_lt();
        }
        if better {
            return true;
        }
        let a = self.function_extensions.function(a);
        let b = self.function_extensions.function(b);
        let mut ap = a
            .map_or(&[][..], |a| a.predicates)
            .iter()
            .filter(|p| p.kind == PredicateKind::Enable);
        let mut bp = b
            .map_or(&[][..], |b| b.predicates)
            .iter()
            .filter(|p| p.kind == PredicateKind::Enable);
        loop {
            match (ap.next(), bp.next()) {
                | (Some(a), Some(b)) if self.equivalent_predicate(a, b) => {},
                | (Some(_), None) => return true,
                | (_, Some(_)) => return false,
                | (None, None) => break,
            }
        }
        a.map_or(0, |a| a.sizes.len()) > b.map_or(0, |b| b.sizes.len())
    }

    /// Qualification conversions keep exact-match rank; compare their target
    /// qualifier sets rather than treating every added qualifier as a
    /// promotion. Clang's C overloading also inherits the pointer-to-bool
    /// tie-break. C99: §6.3.2.3p1, p. 47; PDF p. 59 (extended conversion
    /// ranking).
    fn compare_overload_conversion(
        &mut self,
        info: ExpressionInfo<'tu>,
        from: TypeId,
        a: TypeId,
        b: TypeId,
    ) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        let from_pointer = self.pointer_target(from).is_some();
        if from_pointer {
            let boolean =
                |ty: TypeId| matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Bool));
            let order = boolean(a).cmp(&boolean(b));
            if !order.is_eq() {
                return order;
            }
        }
        let (Some(mut a), Some(mut b)) = (self.pointer_target(a), self.pointer_target(b)) else {
            return Ordering::Equal;
        };
        if self.null_pointer_constant(info) {
            // A C null pointer constant cast to void * uses pointer
            // conversions, including to qualified void *. As in Clang's
            // C++-based ranking, conversion to void * loses to another
            // pointer target. Integer null constants have no such tie-break.
            if from_pointer {
                let void = |ty: TypeId| {
                    matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Void))
                };
                return void(a).cmp(&void(b));
            }
            return Ordering::Equal;
        }
        if self.pointer_target(from).is_some_and(|source| {
            matches!(
                self.types.nodes[source.index],
                TypeKind::Scalar(Scalar::Void)
            ) && source.index != a.index
        }) {
            return Ordering::Equal;
        }
        let mut a_subset = true;
        let mut b_subset = true;
        loop {
            a_subset &= b.qualifiers.contains(a.qualifiers);
            b_subset &= a.qualifiers.contains(b.qualifiers);
            match (self.types.nodes[a.index], self.types.nodes[b.index]) {
                | (TypeKind::Pointer(inner_a), TypeKind::Pointer(inner_b)) => {
                    a = inner_a;
                    b = inner_b;
                },
                | (TypeKind::Array(inner_a, bound_a), TypeKind::Array(inner_b, bound_b))
                    if bound_a == bound_b =>
                {
                    a = inner_a;
                    b = inner_b;
                },
                | _ => break,
            }
        }
        if a.index != b.index {
            return Ordering::Equal;
        }
        match (a_subset, b_subset) {
            | (true, false) => Ordering::Less,
            | (false, true) => Ordering::Greater,
            | _ => Ordering::Equal,
        }
    }

    /// Array element qualification participates in the same exact-match
    /// sequence as pointer-target qualification, including nested arrays.
    /// C99: §6.7.3p8, p. 109; PDF p. 121 (array qualification).
    fn same_qualification_shape(&self, mut a: TypeId, mut b: TypeId) -> bool {
        loop {
            if a.index == b.index {
                return true;
            }
            match (self.types.nodes[a.index], self.types.nodes[b.index]) {
                | (TypeKind::Pointer(inner_a), TypeKind::Pointer(inner_b)) => {
                    a = inner_a;
                    b = inner_b;
                },
                | (TypeKind::Array(inner_a, bound_a), TypeKind::Array(inner_b, bound_b))
                    if bound_a == bound_b =>
                {
                    a = inner_a;
                    b = inner_b;
                },
                | _ => return false,
            }
        }
    }

    pub(super) fn call_attribute_diagnostic(
        &mut self,
        source: crate::translation_phases::SourceVectors,
        message: super::super::AttributeMessage,
        warning: bool,
    ) {
        if self.tainted {
            return;
        }
        let severity = if warning {
            crate::translation_phases::ErrorSeverity::Warning
        } else {
            crate::translation_phases::ErrorSeverity::Error
        };
        self.semantic_errors += usize::from(!warning);
        self.error_diagnostics += usize::from(!warning);
        let source_vectors = self.context.file_location(source);
        if self.context.withholds(severity, false, source_vectors) {
            return;
        }
        self.context.append_pending_errors([
            crate::translation_phases::TranslationError::Semantic(super::super::SemanticError {
                extension_severity: Some(severity),
                kind: SemanticErrorKind::CallAttribute,
                source_vectors,
                name: Some(message.text),
                previous: Some(message.source),
            }),
        ]);
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// A size-annotated function has no ordinary C function-pointer ABI.
    /// Address contexts select a compatible unannotated member of the set.
    /// C99: §6.3.2.1p4, p. 46; PDF p. 58 (extended function designation).
    pub(in crate::translation_phases::semantic_analysis) fn resolve_function_value(
        &mut self,
        source: ExpressionInfo<'tu>,
        target: Option<TypeId>,
    ) -> ExpressionInfo<'tu> {
        let Some(anchor) = source
            .binding
            .filter(|&binding| self.bindings[binding].kind == BindingKind::Function)
        else {
            return source;
        };
        if self.function_extensions.function(anchor).is_none()
            && !self.overload_previous.contains_key(&anchor)
        {
            return source;
        }
        let addressed = self
            .pointer_target(source.ty)
            .is_some_and(|ty| matches!(self.types.nodes[ty.index], TypeKind::Function { .. }));
        let target = target
            .and_then(|ty| self.pointer_target(ty))
            .filter(|ty| matches!(self.types.nodes[ty.index], TypeKind::Function { .. }));
        let (selected, ambiguous) = self.function_value_candidate(anchor, target);
        if let Some(binding) = selected.filter(|_| !ambiguous) {
            let mut leaf = self.selected_designator(source.expression);
            if let super::super::ExpressionType::Unary {
                operator: super::super::UnaryOperator::AddressOf,
                operand_expression,
            } = leaf.kind
            {
                leaf = self.selected_designator(operand_expression);
            }
            if self.function_extensions.value(leaf).is_none()
                && let Some(message) = self
                    .function_extensions
                    .function(binding)
                    .and_then(|contract| contract.unavailable)
            {
                self.call_attribute_diagnostic(source.expression.source_vectors, message, false);
            }
            _ = self
                .function_extensions
                .values
                .insert(std::ptr::from_ref(leaf).addr(), binding);
            _ = self
                .function_extensions
                .values
                .insert(std::ptr::from_ref(source.expression).addr(), binding);
            let mut usage = self.bindings[binding].name;
            usage.source_vectors = source.expression.source_vectors;
            self.record_binding_use(binding, usage);
            let ty = self.bindings[binding].ty;
            let ty = if addressed {
                self.types.intern(TypeKind::Pointer(ty))
            } else {
                ty
            };
            ExpressionInfo {
                ty,
                binding: Some(binding),
                ..source
            }
        } else if ambiguous && target.is_none() {
            // An enclosing assignment or cast supplies the target signature.
            source
        } else {
            self.error(
                if ambiguous {
                    SemanticErrorKind::AmbiguousOverload
                } else {
                    SemanticErrorKind::InvalidAddressOperand
                },
                source.expression.source_vectors,
                Some(self.bindings[anchor].name.name),
                None,
            );
            ExpressionInfo {
                ty: self.types.unknown(),
                ..source
            }
        }
    }

    /// Speculative address selection must not commit binding uses or deferred
    /// diagnostics while an enclosing overload is still being ranked.
    /// C99: §6.3.2.1p4, p. 46; PDF p. 58 (extended function designation).
    fn function_value_candidate(
        &mut self,
        anchor: usize,
        target: Option<TypeId>,
    ) -> (Option<usize>, bool) {
        let mut selected = None;
        let mut seen = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.scratch);
        let mut candidate = Some(anchor);
        let mut ambiguous = false;
        let mut selected_rank = u8::MAX;
        while let Some(binding) = candidate {
            let contract = self.function_extensions.function(binding);
            if seen.insert(self.function_identity(binding))
                && contract.is_none_or(|contract| {
                    contract.sizes.is_empty()
                        && !contract
                            .predicates
                            .iter()
                            .any(|p| p.kind == PredicateKind::Enable)
                })
                && target.is_none_or(|target| {
                    self.types
                        .composite(target, self.bindings[binding].ty)
                        .is_some()
                })
            {
                let rank = u8::from(
                    target.is_some_and(|target| target.index != self.bindings[binding].ty.index),
                );
                if rank < selected_rank {
                    selected = Some(binding);
                    selected_rank = rank;
                    ambiguous = false;
                } else if rank == selected_rank {
                    ambiguous = true;
                }
            }
            candidate = self.overload_previous.get(&binding).copied();
        }
        (selected, ambiguous)
    }
}
