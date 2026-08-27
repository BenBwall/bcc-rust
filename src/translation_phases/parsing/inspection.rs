//! Deterministic, source-oriented syntax-tree inspection.

use std::{
    collections::HashSet,
    fmt::Write,
};

use super::{
    BlockItem,
    ConstantExpressionSlot,
    Context,
    DeclarationIndex,
    Declarator,
    DirectDeclarator,
    ExpressionIndex,
    ExpressionSlot,
    ExpressionType,
    ExternalDeclaration,
    ForInitializer,
    FunctionDefinitionIndex,
    GetPosition,
    Identifier,
    InitializerIndex,
    InitializerType,
    SourceVectors,
    StatementIndex,
    StatementType,
    StringTokenType,
    SyntaxTree,
    TypeNameIndex,
};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InspectionOptions {
    pub(crate) show_locations: bool,
}

enum Work {
    Declaration(DeclarationIndex, usize, &'static str),
    Function(FunctionDefinitionIndex, usize),
    Statement(StatementIndex, usize, &'static str),
    Expression(ExpressionIndex, usize, &'static str),
    Initializer(InitializerIndex, usize, &'static str),
    TypeName(TypeNameIndex, usize, &'static str),
}

impl SyntaxTree {
    pub(crate) fn inspect(
        &self,
        roots: &[ExternalDeclaration],
        context: &Context,
        options: InspectionOptions,
    ) -> String {
        let mut output = String::new();
        let mut work = Vec::new();
        for (ordinal, root) in roots.iter().copied().enumerate().rev() {
            match root {
                | ExternalDeclaration::Declaration(index) => {
                    work.push(Work::Declaration(index, 0, "declaration"));
                },
                | ExternalDeclaration::RecoveredDeclaration(index) => {
                    work.push(Work::Declaration(index, 0, "recovered-declaration"));
                },
                | ExternalDeclaration::FunctionDefinition(index) => {
                    work.push(Work::Function(index, 0));
                },
                | ExternalDeclaration::RecoveredFunctionDefinition(index) => {
                    Self::line(
                        &mut output,
                        0,
                        &format!("root[{ordinal}] recovered-function-definition"),
                        None,
                        context,
                        options,
                    );
                    work.push(Work::Function(index, 1));
                },
                | ExternalDeclaration::Error(source) => Self::line(
                    &mut output,
                    0,
                    &format!("root[{ordinal}] error"),
                    Some(source),
                    context,
                    options,
                ),
            }
        }

        let mut seen = HashSet::new();
        while let Some(item) = work.pop() {
            match item {
                | Work::Declaration(index, indent, role) => {
                    if !seen.insert((0_u8, index.0)) {
                        Self::line(
                            &mut output,
                            indent,
                            &format!("{role}: declaration#{} (shared)", index.0),
                            None,
                            context,
                            options,
                        );
                        continue;
                    }
                    let view = self.declaration(index);
                    let declaration = view.syntax();
                    let status = if declaration.recovered {
                        " recovered"
                    } else {
                        ""
                    };
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "{role}: declaration{status} storage={:?} type={}",
                            declaration.declaration_specifiers.storage_class,
                            declaration.declaration_specifiers.type_specifiers
                        ),
                        Some(declaration.source_vectors),
                        context,
                        options,
                    );
                    for init in view.init_declarators().iter().rev() {
                        let name = self
                            .declarator_identifier(init.declarator)
                            .map_or("<abstract>", |identifier| {
                                context.string_cache.at(identifier.name)
                            });
                        Self::line(
                            &mut output,
                            indent + 1,
                            &format!("declarator {name}"),
                            Some(init.source_vectors),
                            context,
                            options,
                        );
                        if let Some(initializer) = init.initializer {
                            work.push(Work::Initializer(initializer, indent + 2, "initializer"));
                        }
                    }
                },
                | Work::Function(index, indent) => {
                    if !seen.insert((1_u8, index.0)) {
                        continue;
                    }
                    let function = self.function_definition(index);
                    let name = self
                        .declarator_identifier(function.declarator)
                        .map_or("<anonymous>", |identifier| {
                            context.string_cache.at(identifier.name)
                        });
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "function-definition {name}{}",
                            if function.recovered { " recovered" } else { "" }
                        ),
                        Some(function.source_vectors),
                        context,
                        options,
                    );
                    work.push(Work::Statement(function.body, indent + 1, "body"));
                },
                | Work::Statement(index, indent, role) => {
                    if !seen.insert((2_u8, index.0)) {
                        continue;
                    }
                    let statement = self.statement(index);
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "{role}: {}{}",
                            Self::statement_label(&statement.kind, context),
                            if statement.recovered {
                                " recovered"
                            } else {
                                ""
                            }
                        ),
                        Some(statement.source_vectors),
                        context,
                        options,
                    );
                    match statement.kind {
                        | StatementType::Compound { items } => {
                            for item in Self::checked_slice(&self.store.block_items, items)
                                .iter()
                                .rev()
                            {
                                match *item {
                                    | BlockItem::Declaration(index) => work
                                        .push(Work::Declaration(index, indent + 1, "block-item")),
                                    | BlockItem::Statement(index) =>
                                        work.push(Work::Statement(index, indent + 1, "block-item")),
                                }
                            }
                        },
                        | StatementType::Expression(slot) => {
                            Self::push_slot(&mut work, slot, indent + 1, "expression");
                        },
                        | StatementType::If {
                            condition_expression,
                            then_statement,
                            else_statement,
                        } => {
                            if let Some(index) = else_statement {
                                work.push(Work::Statement(index, indent + 1, "else"));
                            }
                            work.push(Work::Statement(then_statement, indent + 1, "then"));
                            Self::push_slot(
                                &mut work,
                                condition_expression,
                                indent + 1,
                                "condition",
                            );
                        },
                        | StatementType::Switch {
                            condition_expression,
                            body_statement,
                        }
                        | StatementType::While {
                            condition_expression,
                            body_statement,
                        }
                        | StatementType::DoWhile {
                            condition_expression,
                            body_statement,
                        } => {
                            work.push(Work::Statement(body_statement, indent + 1, "body"));
                            Self::push_slot(
                                &mut work,
                                condition_expression,
                                indent + 1,
                                "condition",
                            );
                        },
                        | StatementType::For {
                            initializer,
                            condition_expression,
                            iteration_expression,
                            body_statement,
                        } => {
                            work.push(Work::Statement(body_statement, indent + 1, "body"));
                            if let Some(slot) = iteration_expression {
                                Self::push_slot(&mut work, slot, indent + 1, "iteration");
                            }
                            if let Some(slot) = condition_expression {
                                Self::push_slot(&mut work, slot, indent + 1, "condition");
                            }
                            if let Some(initializer) = initializer {
                                match initializer {
                                    | ForInitializer::Expression(slot) =>
                                        Self::push_slot(&mut work, slot, indent + 1, "initializer"),
                                    | ForInitializer::Declaration(index) => work
                                        .push(Work::Declaration(index, indent + 1, "initializer")),
                                }
                            }
                        },
                        | StatementType::Return(Some(slot)) => {
                            Self::push_slot(&mut work, slot, indent + 1, "return-value");
                        },
                        | StatementType::Label(_, child) | StatementType::Default(child) => {
                            work.push(Work::Statement(child, indent + 1, "labeled"));
                        },
                        | StatementType::Case(expression, child) => {
                            work.push(Work::Statement(child, indent + 1, "labeled"));
                            if let ConstantExpressionSlot::Parsed(index) = expression {
                                work.push(Work::Expression(index.into(), indent + 1, "case-value"));
                            }
                        },
                        | StatementType::Return(None)
                        | StatementType::Break
                        | StatementType::Continue
                        | StatementType::Goto(_)
                        | StatementType::Null => {},
                    }
                },
                | Work::Expression(index, indent, role) => {
                    if !seen.insert((3_u8, index.0)) {
                        Self::line(
                            &mut output,
                            indent,
                            &format!("{role}: expression#{} (shared)", index.0),
                            None,
                            context,
                            options,
                        );
                        continue;
                    }
                    let expression = self.expression(index);
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "{role}: {}{}",
                            Self::expression_label(&expression.kind, context),
                            if expression.recovered {
                                " recovered"
                            } else {
                                ""
                            }
                        ),
                        Some(expression.source_vectors),
                        context,
                        options,
                    );
                    self.push_expression_children(&mut work, &expression.kind, indent + 1);
                },
                | Work::Initializer(index, indent, role) => {
                    if !seen.insert((4_u8, index.0)) {
                        continue;
                    }
                    let view = self.initializer(index);
                    let initializer = view.syntax();
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "{role}: {}{}",
                            match initializer.kind {
                                | InitializerType::AssignmentExpression(_) => {
                                    "assignment-expression"
                                },
                                | InitializerType::InitializerList(_) => "initializer-list",
                            },
                            if initializer.recovered {
                                " recovered"
                            } else {
                                ""
                            }
                        ),
                        Some(initializer.source_vectors),
                        context,
                        options,
                    );
                    match initializer.kind {
                        | InitializerType::AssignmentExpression(expression) => work.push(
                            Work::Expression(expression, indent + 1, "assignment-expression"),
                        ),
                        | InitializerType::InitializerList(_) => {
                            if let Some(elements) = view.elements() {
                                for element in elements.iter().rev() {
                                    work.push(Work::Initializer(
                                        element.initializer,
                                        indent + 1,
                                        "element",
                                    ));
                                }
                            }
                        },
                    }
                },
                | Work::TypeName(index, indent, role) => {
                    if !seen.insert((5_u8, index.0)) {
                        continue;
                    }
                    let type_name = self.type_name(index);
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "{role}: type-name{} type={}",
                            if type_name.recovered {
                                " recovered"
                            } else {
                                ""
                            },
                            type_name.declaration_specifiers.type_specifiers
                        ),
                        Some(type_name.source_vectors),
                        context,
                        options,
                    );
                },
            }
        }
        output
    }

    fn declarator_identifier(&self, mut declarator: Declarator) -> Option<Identifier> {
        loop {
            let directs = Self::checked_slice(&self.store.direct_declarators, declarator.kind);
            let mut nested = None;
            for direct in directs {
                match *direct {
                    | DirectDeclarator::Identifier(identifier) => return Some(identifier),
                    | DirectDeclarator::Parenthesized(child) => nested = Some(child),
                    | _ => {},
                }
            }
            declarator = nested?;
        }
    }

    fn statement_label(kind: &StatementType, context: &Context) -> String {
        match kind {
            | StatementType::Label(identifier, _) => {
                format!("label {}", context.string_cache.at(identifier.name))
            },
            | StatementType::Case(..) => "case".to_owned(),
            | StatementType::Default(..) => "default".to_owned(),
            | StatementType::Compound { .. } => "compound".to_owned(),
            | StatementType::Expression(..) => "expression".to_owned(),
            | StatementType::If { .. } => "if".to_owned(),
            | StatementType::Switch { .. } => "switch".to_owned(),
            | StatementType::While { .. } => "while".to_owned(),
            | StatementType::DoWhile { .. } => "do-while".to_owned(),
            | StatementType::For { .. } => "for".to_owned(),
            | StatementType::Goto(identifier) => {
                format!("goto {}", context.string_cache.at(identifier.name))
            },
            | StatementType::Continue => "continue".to_owned(),
            | StatementType::Break => "break".to_owned(),
            | StatementType::Return(..) => "return".to_owned(),
            | StatementType::Null => "null".to_owned(),
        }
    }

    fn expression_label(kind: &ExpressionType, context: &Context) -> String {
        match kind {
            | ExpressionType::Parenthesized { .. } => "parenthesized".to_owned(),
            | ExpressionType::Conditional { .. } => "conditional ?:".to_owned(),
            | ExpressionType::Binary { operator, .. } => format!("binary {operator:?}"),
            | ExpressionType::Unary { operator, .. } => format!("unary {operator:?}"),
            | ExpressionType::Call { .. } => "call".to_owned(),
            | ExpressionType::DirectMember { member, .. } => {
                format!("member .{}", context.string_cache.at(member.name))
            },
            | ExpressionType::IndirectMember { member, .. } => {
                format!("member ->{}", context.string_cache.at(member.name))
            },
            | ExpressionType::CompoundLiteral { .. } => "compound-literal".to_owned(),
            | ExpressionType::Identifier(identifier) => {
                format!("identifier {}", context.string_cache.at(identifier.name))
            },
            | ExpressionType::Constant(constant) => format!("constant {constant:?}"),
            | ExpressionType::StringLiteral(string) => match string {
                | StringTokenType::String(contents) => {
                    format!("string {:?}", context.string_cache.at(*contents))
                },
                | StringTokenType::WideString(contents) => {
                    format!("wide-string {:?}", context.string_cache.at(*contents))
                },
            },
            | ExpressionType::SizeofType(..) => "sizeof type".to_owned(),
            | ExpressionType::SizeofExpr(..) => "sizeof expression".to_owned(),
            | ExpressionType::Cast { .. } => "cast".to_owned(),
            | ExpressionType::Error => "error-expression".to_owned(),
        }
    }

    fn push_slot(work: &mut Vec<Work>, slot: ExpressionSlot, indent: usize, role: &'static str) {
        if let ExpressionSlot::Parsed(index) = slot {
            work.push(Work::Expression(index, indent, role));
        }
    }

    fn push_expression_children(&self, work: &mut Vec<Work>, kind: &ExpressionType, indent: usize) {
        match kind {
            | ExpressionType::Parenthesized { expression }
            | ExpressionType::Unary {
                operand_expression: expression,
                ..
            }
            | ExpressionType::SizeofExpr(expression) => {
                work.push(Work::Expression(*expression, indent, "operand"));
            },
            | ExpressionType::Conditional {
                condition_expression,
                then_expression,
                else_expression,
            } => {
                work.push(Work::Expression(*else_expression, indent, "else"));
                work.push(Work::Expression(*then_expression, indent, "then"));
                work.push(Work::Expression(*condition_expression, indent, "condition"));
            },
            | ExpressionType::Binary {
                left_expression,
                right_expression,
                ..
            } => {
                work.push(Work::Expression(*right_expression, indent, "rhs"));
                work.push(Work::Expression(*left_expression, indent, "lhs"));
            },
            | ExpressionType::Call {
                function_expression,
                arguments,
            } => {
                for argument in Self::checked_slice(&self.store.expression_indices, *arguments)
                    .iter()
                    .rev()
                {
                    work.push(Work::Expression(*argument, indent, "argument"));
                }
                work.push(Work::Expression(*function_expression, indent, "callee"));
            },
            | ExpressionType::DirectMember {
                base_expression, ..
            }
            | ExpressionType::IndirectMember {
                base_expression, ..
            } => work.push(Work::Expression(*base_expression, indent, "base")),
            | ExpressionType::CompoundLiteral {
                type_name,
                initializer,
            } => {
                work.push(Work::Initializer(*initializer, indent, "initializer"));
                work.push(Work::TypeName(*type_name, indent, "type"));
            },
            | ExpressionType::Cast {
                target_type,
                operand_expression,
            } => {
                work.push(Work::Expression(*operand_expression, indent, "operand"));
                work.push(Work::TypeName(*target_type, indent, "target-type"));
            },
            | ExpressionType::SizeofType(type_name) => {
                work.push(Work::TypeName(*type_name, indent, "operand-type"));
            },
            | ExpressionType::Identifier(_)
            | ExpressionType::Constant(_)
            | ExpressionType::StringLiteral(_)
            | ExpressionType::Error => {},
        }
    }

    fn line(
        output: &mut String,
        indent: usize,
        text: &str,
        source: Option<SourceVectors>,
        context: &Context,
        options: InspectionOptions,
    ) {
        let _ = write!(output, "{}{}", "  ".repeat(indent), text);
        if options.show_locations
            && let Some(source) = source
            && source.length > 0
        {
            let position = source.position(context);
            let _ = write!(output, " @{}:{}", position.line, position.column);
        }
        output.push('\n');
    }
}
