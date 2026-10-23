//! Microsoft ABI names preserve written nested function parameter qualifiers
//! and array adjustment, while semantic function identity drops that sugar.
//! Sparse metadata follows checked declarators, typedefs and typeof origins.
//! C99: §6.7.5.3p7-8,15, pp. 119-120; PDF pp. 131-132.

use super::super::{
    Analyzer,
    BinaryOperator,
    DirectDeclarator,
    Expression,
    ExpressionType,
    ExtendedType,
    Namespace,
    SyntaxOperand,
    TypeKind,
    UnaryOperator,
    WrittenFunction,
};
use crate::{
    target::Target,
    translation_phases::parsing::declaration_syntax::{
        DeclarationSpecifiers,
        Declarator,
        TypeName,
        TypeQualifiers,
        TypeSpecifiers,
    },
    util::{
        arena_list::ArenaList,
        bump::ArenaVec,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(in crate::translation_phases::semantic_analysis) fn written_function_types(
        &self,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Option<Declarator<'tu>>,
    ) -> ArenaList<'tu, WrittenFunction<'tu>> {
        if self.context.configuration.target() != Target::WindowsMsvc {
            return ArenaList::empty();
        }
        let mut work = ArenaVec::new_in(self.scratch);
        work.push(Step::Declaration(spec, declarator));
        let mut functions = ArenaVec::new_in(self.scratch);
        let mut depth: isize = 0;
        while let Some(step) = work.pop() {
            match step {
                | Step::Declaration(spec, declarator) => {
                    work.push(Step::Base(spec.type_specifiers));
                    if spec.type_qualifiers.contains(TypeQualifiers::ATOMIC) {
                        work.push(Step::Derive);
                    }
                    if let Some(declarator) = declarator {
                        work.push(Step::Declarator(declarator));
                    }
                },
                | Step::Declarator(declarator) => {
                    for level in declarator.pointer.levels {
                        work.push(Step::Derive);
                        if level.qualifiers.contains(TypeQualifiers::ATOMIC) {
                            work.push(Step::Derive);
                        }
                    }
                    for direct in declarator.kind.as_slice().iter().rev() {
                        work.push(Step::Direct(direct));
                    }
                },
                | Step::Direct(direct) => match direct {
                    | DirectDeclarator::Parenthesized(p) =>
                        work.push(Step::Declarator(p.declarator)),
                    | DirectDeclarator::Function { .. }
                    | DirectDeclarator::KAndRStyleFunction { .. } => {
                        if let Some(&(parameters, _)) =
                            self.parameters.get(&std::ptr::from_ref(direct).addr())
                            && let Ok(depth) = usize::try_from(depth)
                        {
                            functions.push(WrittenFunction { depth, parameters });
                        }
                        depth += 1;
                    },
                    | DirectDeclarator::Array { .. } => depth += 1,
                    | _ => {},
                },
                | Step::Derive => depth += 1,
                | Step::Base(base) => match base {
                    | TypeSpecifiers::TypedefName(name) => {
                        if let Some(entry) = self.lookup(Namespace::Ordinary, name.name)
                            && let Some(written) =
                                self.function_extensions.written_types.get(&entry.binding)
                        {
                            functions.extend(
                                written
                                    .iter()
                                    .filter_map(|function| shifted(*function, depth)),
                            );
                        }
                    },
                    | TypeSpecifiers::Extended(extended) => {
                        let operand = match *extended {
                            | ExtendedType::Atomic(name) => {
                                depth += 1;
                                Some(SyntaxOperand::Type(name))
                            },
                            | ExtendedType::Typeof {
                                operand,
                                unqualified,
                            } => {
                                if unqualified
                                    && matches!(
                                        self.types.nodes[self.operand_type(operand).index],
                                        TypeKind::Atomic(_)
                                    )
                                {
                                    depth -= 1;
                                }
                                Some(operand)
                            },
                            | _ => None,
                        };
                        match operand {
                            | Some(SyntaxOperand::Type(name)) => work.push(Step::TypeName(name)),
                            | Some(SyntaxOperand::Expression(expression)) =>
                                work.push(Step::Expression(expression)),
                            | None => {},
                        }
                    },
                    | _ => {},
                },
                | Step::Expression(expression) => {
                    let info = self.expression_info(expression);
                    // A conditional's value can select one address, but its
                    // composite function type still depends on both arms
                    // (C99 §6.5.15p6, pp. 90-91; PDF pp. 102-103).
                    if let Some(selected) = info.selected_expression
                        && !matches!(
                            expression.kind,
                            ExpressionType::Conditional(_) | ExpressionType::OmittedConditional(_)
                        )
                    {
                        work.push(Step::Expression(selected));
                        continue;
                    }
                    match expression.kind {
                        | ExpressionType::Identifier(_) => {
                            if let Some(binding) = info.binding
                                && let Some(written) =
                                    self.function_extensions.written_types.get(&binding)
                            {
                                functions.extend(
                                    written
                                        .iter()
                                        .filter_map(|function| shifted(*function, depth)),
                                );
                            }
                        },
                        | ExpressionType::Parenthesized { expression } =>
                            work.push(Step::Expression(expression)),
                        | ExpressionType::StatementExpression(statement) => {
                            if let Some((_, tail)) = self.statement_expression_tail(statement) {
                                work.push(Step::ValueExpression(tail));
                            }
                        },
                        | ExpressionType::Unary {
                            operator,
                            operand_expression,
                        } => {
                            depth += match operator {
                                | UnaryOperator::AddressOf => 1,
                                | UnaryOperator::Indirection => -1,
                                | _ => 0,
                            };
                            work.push(Step::Expression(operand_expression));
                        },
                        | ExpressionType::Call {
                            function_expression,
                            ..
                        } => {
                            depth -= if matches!(
                                self.types.nodes
                                    [self.expression_info(function_expression).ty.index],
                                TypeKind::Function { .. }
                            ) {
                                1
                            } else {
                                2
                            };
                            work.push(Step::Expression(function_expression));
                        },
                        | ExpressionType::DirectMember {
                            base_expression,
                            member,
                        }
                        | ExpressionType::IndirectMember {
                            base_expression,
                            member,
                        } => {
                            let mut ty = self.expression_info(base_expression).ty;
                            if matches!(expression.kind, ExpressionType::IndirectMember { .. })
                                && let TypeKind::Pointer(inner) = self.types.nodes[ty.index]
                            {
                                ty = inner;
                            }
                            if let TypeKind::Tag(tag) = self.types.nodes[ty.index]
                                && let Some(written) = self
                                    .function_extensions
                                    .written_fields
                                    .get(&(tag, member.name))
                            {
                                functions.extend(
                                    written
                                        .iter()
                                        .filter_map(|function| shifted(*function, depth)),
                                );
                            }
                        },
                        | ExpressionType::Cast { target_type, .. }
                        | ExpressionType::CompoundLiteral {
                            type_name: target_type,
                            ..
                        } => work.push(Step::TypeName(target_type)),
                        | ExpressionType::Conditional(conditional)
                        | ExpressionType::OmittedConditional(conditional) => {
                            let pointer = |expression| {
                                matches!(
                                    self.types.nodes[self.expression_info(expression).ty.index],
                                    TypeKind::Pointer(_)
                                        | TypeKind::Array(_, _)
                                        | TypeKind::Function { .. }
                                )
                            };
                            match (
                                pointer(conditional.then_expression),
                                pointer(conditional.else_expression),
                            ) {
                                | (true, false) =>
                                    work.push(Step::ValueExpression(conditional.then_expression)),
                                | (false, true) =>
                                    work.push(Step::ValueExpression(conditional.else_expression)),
                                | _ => {
                                    work.push(Step::ConditionalThen(
                                        functions.len(),
                                        depth,
                                        conditional.else_expression,
                                    ));
                                    work.push(Step::ValueExpression(conditional.then_expression));
                                },
                            }
                        },
                        | ExpressionType::Binary {
                            operator,
                            left_expression,
                            right_expression,
                        } => {
                            let expression = match operator {
                                | BinaryOperator::Comma => Some(right_expression),
                                | BinaryOperator::Assignment
                                | BinaryOperator::AdditionAssignment
                                | BinaryOperator::SubtractionAssignment => Some(left_expression),
                                | BinaryOperator::Subscript
                                | BinaryOperator::Addition
                                | BinaryOperator::Subtraction => {
                                    if operator == BinaryOperator::Subscript {
                                        depth -= 1;
                                    }
                                    Some(
                                        if matches!(
                                            self.types.nodes
                                                [self.expression_info(left_expression).ty.index],
                                            TypeKind::Pointer(_) | TypeKind::Array(_, _)
                                        ) {
                                            left_expression
                                        } else {
                                            right_expression
                                        },
                                    )
                                },
                                | _ => None,
                            };
                            if let Some(expression) = expression {
                                work.push(if operator == BinaryOperator::Comma {
                                    Step::ValueExpression(expression)
                                } else {
                                    Step::Expression(expression)
                                });
                            }
                        },
                        | _ => {},
                    }
                },
                | Step::ConditionalThen(start, original_depth, otherwise) => {
                    depth = original_depth;
                    work.push(Step::ConditionalMerge(
                        start,
                        functions.len(),
                        original_depth,
                    ));
                    work.push(Step::ValueExpression(otherwise));
                },
                | Step::ConditionalMerge(start, middle, original_depth) => {
                    // Conditional composition preserves identical written
                    // function types; conflicting sugar yields the canonical
                    // function type, even when semantic identities agree.
                    let same = self
                        .same_written_functions(&functions[start..middle], &functions[middle..]);
                    functions.truncate(if same { middle } else { start });
                    depth = original_depth;
                },
                | Step::ValueExpression(expression) => {
                    if matches!(
                        self.types.nodes[self.expression_info(expression).ty.index],
                        TypeKind::Function { .. }
                    ) {
                        depth += 1;
                    }
                    work.push(Step::Expression(expression));
                },
                | Step::TypeName(name) => {
                    if let Some(written) = self
                        .function_extensions
                        .written_type_names
                        .get(&super::super::type_name_identity(name))
                    {
                        functions.extend(
                            written
                                .iter()
                                .filter_map(|function| shifted(*function, depth)),
                        );
                    } else {
                        work.push(Step::Declaration(
                            name.declaration_specifiers,
                            name.declarator,
                        ));
                    }
                },
            }
        }
        ArenaList::copy_from_slice(self.types.tu, &functions)
    }

    pub(in crate::translation_phases::semantic_analysis) fn retain_written_parameter(
        &mut self,
        binding: usize,
        original: super::super::TypeId,
        written: ArenaList<'tu, WrittenFunction<'tu>>,
    ) {
        if written.is_empty() {
            return;
        }
        let written = if matches!(self.types.nodes[original.index], TypeKind::Function { .. }) {
            let mut adjusted = ArenaVec::new_in(self.scratch);
            adjusted.extend(written.iter().map(|function| WrittenFunction {
                depth: function.depth + 1,
                ..*function
            }));
            ArenaList::copy_from_slice(self.types.tu, &adjusted)
        } else {
            written
        };
        _ = self
            .function_extensions
            .written_types
            .insert(binding, written);
    }

    fn same_written_functions(
        &self,
        left: &[WrittenFunction<'tu>],
        right: &[WrittenFunction<'tu>],
    ) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((left, right));
        while let Some((left, right)) = work.pop() {
            if left.len() != right.len() {
                return false;
            }
            for (left, right) in left.iter().zip(right) {
                if left.depth != right.depth || left.parameters.len() != right.parameters.len() {
                    return false;
                }
                for (left, right) in left.parameters.iter().zip(right.parameters) {
                    if !self.same_abi_type(left.original, right.original) {
                        return false;
                    }
                    work.push((
                        left.written_functions.as_slice(),
                        right.written_functions.as_slice(),
                    ));
                }
            }
        }
        true
    }
}

fn shifted(function: WrittenFunction<'_>, depth: isize) -> Option<WrittenFunction<'_>> {
    Some(WrittenFunction {
        depth: function.depth.checked_add_signed(depth)?,
        ..function
    })
}

enum Step<'tu> {
    Declaration(DeclarationSpecifiers<'tu>, Option<Declarator<'tu>>),
    Declarator(Declarator<'tu>),
    Direct(&'tu DirectDeclarator<'tu>),
    Derive,
    Base(TypeSpecifiers<'tu>),
    Expression(&'tu Expression<'tu>),
    ValueExpression(&'tu Expression<'tu>),
    TypeName(&'tu TypeName<'tu>),
    ConditionalThen(usize, isize, &'tu Expression<'tu>),
    ConditionalMerge(usize, usize, isize),
}
