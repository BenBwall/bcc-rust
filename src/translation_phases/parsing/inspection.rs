//! Deterministic, source-oriented syntax-tree inspection.

use std::{
    collections::HashSet,
    fmt::Write,
};

use super::{
    declaration_syntax::{
        Declarator,
        Designator,
        DesignatorType,
        DirectDeclarator,
        Enumerator,
        InitDeclarator,
        InitializerElement,
        InitializerType,
        ParameterDeclaration,
        StructDeclaration,
        StructDeclarator,
        StructOrUnion,
        TypeQualifiers,
        TypeSpecifiers,
    },
    syntax::{
        BinaryOperator,
        BlockItem,
        Constant,
        ConstantExpressionSlot,
        DeclarationIndex,
        DesignationIndex,
        EnumSpecifierIndex,
        ExpressionIndex,
        ExpressionSlot,
        ExpressionType,
        ExternalDeclaration,
        ForInitializer,
        FunctionDefinitionIndex,
        Identifier,
        InitializerIndex,
        StatementIndex,
        StatementType,
        StorageClass,
        StructOrUnionSpecifierIndex,
        TypeNameIndex,
        UnaryOperator,
    },
    syntax_store::SyntaxTree,
};
use crate::{
    diagnostics::c_quoted,
    translation_phases::{
        Context,
        GetPosition,
        SourceVectors,
        preprocessing::{
            CharacterTokenType,
            IntegerTokenType,
            StringTokenType,
        },
    },
};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InspectionOptions {
    pub(crate) show_locations: bool,
}

enum Work {
    Root(ExternalDeclaration, usize),
    Declaration(DeclarationIndex, usize, &'static str),
    InitDeclarator(InitDeclarator, usize),
    Function(FunctionDefinitionIndex, usize, &'static str),
    Declarator(Declarator, usize, &'static str),
    DirectDeclarator(DirectDeclarator, usize),
    Identifier(Identifier, usize, &'static str),
    Parameter(ParameterDeclaration, usize),
    StructOrUnion(StructOrUnionSpecifierIndex, usize),
    StructDeclaration(StructDeclaration, usize),
    StructDeclarator(StructDeclarator, usize),
    Enum(EnumSpecifierIndex, usize),
    Enumerator(Enumerator, usize),
    Statement(StatementIndex, usize, &'static str),
    Expression(ExpressionIndex, usize, &'static str),
    Missing(SourceVectors, usize, &'static str),
    Initializer(InitializerIndex, usize, &'static str),
    InitializerElement(InitializerElement, usize),
    Designation(DesignationIndex, usize),
    Designator(Designator, usize),
    TypeName(TypeNameIndex, usize, &'static str),
}

impl SyntaxTree {
    #[expect(
        clippy::too_many_lines,
        reason = "One iterative dispatcher keeps traversal order and cycle handling centralized."
    )]
    pub(crate) fn inspect(
        &self,
        roots: &[ExternalDeclaration],
        context: &Context,
        options: InspectionOptions,
    ) -> String {
        let mut output = String::new();
        let mut work = roots
            .iter()
            .copied()
            .enumerate()
            .rev()
            .map(|(ordinal, root)| Work::Root(root, ordinal))
            .collect::<Vec<_>>();
        let mut seen = HashSet::new();

        while let Some(item) = work.pop() {
            match item {
                | Work::Root(root, ordinal) => match root {
                    | ExternalDeclaration::Declaration(index) => {
                        work.push(Work::Declaration(index, 0, "declaration"));
                    },
                    | ExternalDeclaration::RecoveredDeclaration(index) => {
                        work.push(Work::Declaration(index, 0, "recovered-declaration"));
                    },
                    | ExternalDeclaration::FunctionDefinition(index) => {
                        work.push(Work::Function(index, 0, "function-definition"));
                    },
                    | ExternalDeclaration::RecoveredFunctionDefinition(index) =>
                        work.push(Work::Function(index, 0, "recovered-function-definition")),
                    | ExternalDeclaration::Error(source) => Self::line(
                        &mut output,
                        0,
                        &format!("root[{ordinal}] error"),
                        Some(source),
                        context,
                        options,
                    ),
                },
                | Work::Declaration(index, indent, role) => {
                    if !seen.insert((0_u8, index.0, 0_u32, 0_u32, 0_u32)) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "declaration",
                            index.0,
                            context,
                            options,
                        );
                        continue;
                    }
                    let view = self.declaration(index);
                    let declaration = view.syntax();
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "{role}: declaration{} storage={} type={}",
                            if declaration.recovered {
                                " recovered"
                            } else {
                                ""
                            },
                            declaration
                                .declaration_specifiers
                                .storage_class
                                .map_or("none", StorageClass::spelling),
                            self.type_label(
                                declaration.declaration_specifiers.type_specifiers,
                                context,
                            ),
                        ),
                        Some(declaration.source_vectors),
                        context,
                        options,
                    );
                    for init in view.init_declarators().iter().rev() {
                        work.push(Work::InitDeclarator(init.clone(), indent + 1));
                    }
                    Self::push_type_details(
                        &mut work,
                        declaration.declaration_specifiers.type_specifiers,
                        indent + 1,
                    );
                },
                | Work::InitDeclarator(init, indent) => {
                    let name = self
                        .declarator_identifier(init.declarator)
                        .map_or("<abstract>", |identifier| {
                            context.string_cache.at(identifier.name)
                        });
                    Self::line(
                        &mut output,
                        indent,
                        &format!("declarator {name}"),
                        Some(init.source_vectors),
                        context,
                        options,
                    );
                    if let Some(initializer) = init.initializer {
                        work.push(Work::Initializer(initializer, indent + 1, "initializer"));
                    }
                    work.push(Work::Declarator(init.declarator, indent + 1, "shape"));
                },
                | Work::Function(index, indent, role) => {
                    if !seen.insert((1_u8, index.0, 0_u32, 0_u32, 0_u32)) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "function",
                            index.0,
                            context,
                            options,
                        );
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
                            "{role} {name}{} type={}",
                            if function.recovered { " recovered" } else { "" },
                            self.type_label(
                                function.declaration_specifiers.type_specifiers,
                                context
                            ),
                        ),
                        Some(function.source_vectors),
                        context,
                        options,
                    );
                    work.push(Work::Statement(function.body, indent + 1, "body"));
                    for declaration in self
                        .declaration_indices(function.declaration_list)
                        .iter()
                        .rev()
                    {
                        work.push(Work::Declaration(
                            *declaration,
                            indent + 1,
                            "declaration-list",
                        ));
                    }
                    work.push(Work::Declarator(
                        function.declarator,
                        indent + 1,
                        "declarator",
                    ));
                    Self::push_type_details(
                        &mut work,
                        function.declaration_specifiers.type_specifiers,
                        indent + 1,
                    );
                },
                | Work::Declarator(declarator, indent, role) => {
                    let key = (
                        6_u8,
                        declarator.kind.start_index(),
                        declarator.kind.length(),
                        declarator.pointer.type_qualifiers_list.start_index(),
                        declarator.pointer.type_qualifiers_list.length(),
                    );
                    if !seen.insert(key) {
                        Self::line(
                            &mut output,
                            indent,
                            &format!("{role}: declarator (shared)"),
                            None,
                            context,
                            options,
                        );
                        continue;
                    }
                    let pointers = self.pointer_qualifiers(declarator.pointer.type_qualifiers_list);
                    Self::line(
                        &mut output,
                        indent,
                        &format!("{role}: declarator pointer-levels={}", pointers.len()),
                        Some(declarator.source_vectors),
                        context,
                        options,
                    );
                    for direct in self.direct_declarators(declarator.kind).iter().rev() {
                        work.push(Work::DirectDeclarator(*direct, indent + 1));
                    }
                },
                | Work::DirectDeclarator(direct, indent) => match direct {
                    | DirectDeclarator::Identifier(identifier) => {
                        work.push(Work::Identifier(identifier, indent, "identifier"));
                    },
                    | DirectDeclarator::Parenthesized(declarator) => {
                        Self::line(
                            &mut output,
                            indent,
                            "parenthesized-declarator",
                            Some(declarator.source_vectors),
                            context,
                            options,
                        );
                        work.push(Work::Declarator(declarator, indent + 1, "nested"));
                    },
                    | DirectDeclarator::KAndRStyleFunction { parameters } => {
                        Self::line(
                            &mut output,
                            indent,
                            "function identifier-list",
                            None,
                            context,
                            options,
                        );
                        for identifier in self.identifiers(parameters).iter().rev() {
                            work.push(Work::Identifier(*identifier, indent + 1, "parameter"));
                        }
                    },
                    | DirectDeclarator::Array {
                        type_qualifiers,
                        is_static,
                        is_pointer,
                        assignment_expression,
                    } => {
                        Self::line(
                            &mut output,
                            indent,
                            &format!(
                                "array static={is_static} variable-length={is_pointer} \
                                 qualifiers={}",
                                qualifier_list(type_qualifiers)
                            ),
                            None,
                            context,
                            options,
                        );
                        if let Some(expression) = assignment_expression {
                            work.push(Work::Expression(expression, indent + 1, "bound"));
                        }
                    },
                    | DirectDeclarator::Function {
                        parameter_list,
                        is_variadic,
                    } => {
                        Self::line(
                            &mut output,
                            indent,
                            &format!("function variadic={is_variadic}"),
                            None,
                            context,
                            options,
                        );
                        for parameter in self.parameter_declarations(parameter_list).iter().rev() {
                            work.push(Work::Parameter(parameter.clone(), indent + 1));
                        }
                    },
                },
                | Work::Identifier(identifier, indent, role) => Self::line(
                    &mut output,
                    indent,
                    &format!("{role} {}", context.string_cache.at(identifier.name)),
                    Some(identifier.source_vectors),
                    context,
                    options,
                ),
                | Work::Parameter(parameter, indent) => {
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "parameter type={}",
                            self.type_label(
                                parameter.declaration_specifiers.type_specifiers,
                                context,
                            )
                        ),
                        Some(parameter.source_vectors),
                        context,
                        options,
                    );
                    if let Some(declarator) = parameter.declarator {
                        work.push(Work::Declarator(declarator, indent + 1, "declarator"));
                    }
                    Self::push_type_details(
                        &mut work,
                        parameter.declaration_specifiers.type_specifiers,
                        indent + 1,
                    );
                },
                | Work::StructOrUnion(index, indent) => {
                    if !seen.insert((7_u8, index.0, 0_u32, 0_u32, 0_u32)) {
                        continue;
                    }
                    let specifier = self.struct_or_union_specifier(index);
                    let kind = match specifier.struct_or_union {
                        | StructOrUnion::Struct => "struct",
                        | StructOrUnion::Union => "union",
                    };
                    let name = specifier.identifier.map_or("<anonymous>", |identifier| {
                        context.string_cache.at(identifier.name)
                    });
                    Self::line(
                        &mut output,
                        indent,
                        &format!("{kind} {name}"),
                        Some(specifier.source_vectors),
                        context,
                        options,
                    );
                    if let Some(declarations) = specifier.struct_declaration_list {
                        for declaration in self.struct_declarations(declarations).iter().rev() {
                            work.push(Work::StructDeclaration(*declaration, indent + 1));
                        }
                    }
                },
                | Work::StructDeclaration(declaration, indent) => {
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "member-declaration type={} qualifiers={}",
                            self.type_label(declaration.type_specifiers, context),
                            qualifier_list(declaration.type_qualifiers),
                        ),
                        Some(declaration.source_vectors),
                        context,
                        options,
                    );
                    for declarator in self
                        .struct_declarators(declaration.struct_declarator_list)
                        .iter()
                        .rev()
                    {
                        work.push(Work::StructDeclarator(*declarator, indent + 1));
                    }
                    Self::push_type_details(&mut work, declaration.type_specifiers, indent + 1);
                },
                | Work::StructDeclarator(declarator, indent) => {
                    Self::line(
                        &mut output,
                        indent,
                        if declarator.bitfield_width.is_some() {
                            "member bit-field"
                        } else {
                            "member declarator"
                        },
                        Some(declarator.source_vectors),
                        context,
                        options,
                    );
                    if let Some(width) = declarator.bitfield_width {
                        work.push(Work::Expression(width.into(), indent + 1, "width"));
                    }
                    if let Some(declarator) = declarator.declarator {
                        work.push(Work::Declarator(declarator, indent + 1, "declarator"));
                    }
                },
                | Work::Enum(index, indent) => {
                    if !seen.insert((8_u8, index.0, 0_u32, 0_u32, 0_u32)) {
                        continue;
                    }
                    let specifier = self.enum_specifier(index);
                    let name = specifier.name.map_or("<anonymous>", |identifier| {
                        context.string_cache.at(identifier.name)
                    });
                    Self::line(
                        &mut output,
                        indent,
                        &format!("enum {name}"),
                        Some(specifier.source_vectors),
                        context,
                        options,
                    );
                    if let Some(enumerators) = specifier.enumeration_list {
                        for enumerator in self.enumerators(enumerators).iter().rev() {
                            work.push(Work::Enumerator(*enumerator, indent + 1));
                        }
                    }
                },
                | Work::Enumerator(enumerator, indent) => {
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "enumerator {}",
                            context.string_cache.at(enumerator.name.name)
                        ),
                        Some(enumerator.source_vectors),
                        context,
                        options,
                    );
                    if let Some(expression) = enumerator.expression {
                        work.push(Work::Expression(expression.into(), indent + 1, "value"));
                    }
                },
                | Work::Statement(index, indent, role) => {
                    if !seen.insert((2_u8, index.0, 0_u32, 0_u32, 0_u32)) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "statement",
                            index.0,
                            context,
                            options,
                        );
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
                    self.push_statement_children(&mut work, &statement.kind, indent + 1);
                },
                | Work::Expression(index, indent, role) => {
                    if !seen.insert((3_u8, index.0, 0_u32, 0_u32, 0_u32)) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "expression",
                            index.0,
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
                | Work::Missing(source, indent, role) => Self::line(
                    &mut output,
                    indent,
                    &format!("{role}: missing"),
                    Some(source),
                    context,
                    options,
                ),
                | Work::Initializer(index, indent, role) => {
                    if !seen.insert((4_u8, index.0, 0_u32, 0_u32, 0_u32)) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "initializer",
                            index.0,
                            context,
                            options,
                        );
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
                                    work.push(Work::InitializerElement(*element, indent + 1));
                                }
                            }
                        },
                    }
                },
                | Work::InitializerElement(element, indent) => {
                    Self::line(
                        &mut output,
                        indent,
                        "element",
                        Some(element.source_vectors),
                        context,
                        options,
                    );
                    work.push(Work::Initializer(element.initializer, indent + 1, "value"));
                    if let Some(designation) = element.designation {
                        work.push(Work::Designation(designation, indent + 1));
                    }
                },
                | Work::Designation(index, indent) => {
                    if !seen.insert((9_u8, index.0, 0_u32, 0_u32, 0_u32)) {
                        continue;
                    }
                    let designation = self.designation(index);
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "designation{}",
                            if designation.recovered {
                                " recovered"
                            } else {
                                ""
                            }
                        ),
                        Some(designation.source_vectors),
                        context,
                        options,
                    );
                    for designator in self.designators(designation.designators).iter().rev() {
                        work.push(Work::Designator(*designator, indent + 1));
                    }
                },
                | Work::Designator(designator, indent) => {
                    let label = match designator.kind {
                        | DesignatorType::Array(_) => "array-designator".to_owned(),
                        | DesignatorType::Field(identifier) => format!(
                            "field-designator .{}",
                            context.string_cache.at(identifier.name)
                        ),
                        | DesignatorType::Error => "error-designator".to_owned(),
                    };
                    Self::line(
                        &mut output,
                        indent,
                        &format!(
                            "{label}{}",
                            if designator.recovered {
                                " recovered"
                            } else {
                                ""
                            }
                        ),
                        Some(designator.source_vectors),
                        context,
                        options,
                    );
                    if let DesignatorType::Array(expression) = designator.kind {
                        work.push(Work::Expression(expression.into(), indent + 1, "index"));
                    }
                },
                | Work::TypeName(index, indent, role) => {
                    if !seen.insert((5_u8, index.0, 0_u32, 0_u32, 0_u32)) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "type-name",
                            index.0,
                            context,
                            options,
                        );
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
                            self.type_label(
                                type_name.declaration_specifiers.type_specifiers,
                                context,
                            ),
                        ),
                        Some(type_name.source_vectors),
                        context,
                        options,
                    );
                    if let Some(declarator) = type_name.declarator {
                        work.push(Work::Declarator(declarator, indent + 1, "declarator"));
                    }
                    Self::push_type_details(
                        &mut work,
                        type_name.declaration_specifiers.type_specifiers,
                        indent + 1,
                    );
                },
            }
        }
        output
    }

    fn push_type_details(work: &mut Vec<Work>, specifiers: TypeSpecifiers, indent: usize) {
        match specifiers {
            | TypeSpecifiers::StructOrUnion(index) => work.push(Work::StructOrUnion(index, indent)),
            | TypeSpecifiers::Enum(index) => work.push(Work::Enum(index, indent)),
            | _ => {},
        }
    }

    fn type_label(&self, specifiers: TypeSpecifiers, context: &Context) -> String {
        match specifiers {
            | TypeSpecifiers::TypedefName(identifier) => {
                format!("typedef {}", context.string_cache.at(identifier.name))
            },
            | TypeSpecifiers::StructOrUnion(index) => {
                let specifier = self.struct_or_union_specifier(index);
                let kind = match specifier.struct_or_union {
                    | StructOrUnion::Struct => "struct",
                    | StructOrUnion::Union => "union",
                };
                specifier.identifier.map_or_else(
                    || format!("{kind} <anonymous>"),
                    |identifier| format!("{kind} {}", context.string_cache.at(identifier.name)),
                )
            },
            | TypeSpecifiers::Enum(index) => {
                let specifier = self.enum_specifier(index);
                specifier.name.map_or_else(
                    || "enum <anonymous>".to_owned(),
                    |identifier| format!("enum {}", context.string_cache.at(identifier.name)),
                )
            },
            | _ => specifiers.to_string(),
        }
    }

    fn declarator_identifier(&self, mut declarator: Declarator) -> Option<Identifier> {
        let mut seen = HashSet::new();
        loop {
            if !seen.insert((
                declarator.kind.start_index(),
                declarator.kind.length(),
                declarator.pointer.type_qualifiers_list.start_index(),
                declarator.pointer.type_qualifiers_list.length(),
            )) {
                return None;
            }
            let mut nested = None;
            for direct in self.direct_declarators(declarator.kind) {
                match *direct {
                    | DirectDeclarator::Identifier(identifier) => return Some(identifier),
                    | DirectDeclarator::Parenthesized(child) => nested = Some(child),
                    | _ => {},
                }
            }
            declarator = nested?;
        }
    }

    fn push_statement_children(&self, work: &mut Vec<Work>, kind: &StatementType, indent: usize) {
        match *kind {
            | StatementType::Compound { items } => {
                for item in self.block_items(items).iter().rev() {
                    match *item {
                        | BlockItem::Declaration(index) => {
                            work.push(Work::Declaration(index, indent, "block-item"));
                        },
                        | BlockItem::Statement(index) => {
                            work.push(Work::Statement(index, indent, "block-item"));
                        },
                    }
                }
            },
            | StatementType::Expression(slot) => {
                Self::push_slot(work, slot, indent, "expression");
            },
            | StatementType::If {
                condition_expression,
                then_statement,
                else_statement,
            } => {
                if let Some(index) = else_statement {
                    work.push(Work::Statement(index, indent, "else"));
                }
                work.push(Work::Statement(then_statement, indent, "then"));
                Self::push_slot(work, condition_expression, indent, "condition");
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
                work.push(Work::Statement(body_statement, indent, "body"));
                Self::push_slot(work, condition_expression, indent, "condition");
            },
            | StatementType::For {
                initializer,
                condition_expression,
                iteration_expression,
                body_statement,
            } => {
                work.push(Work::Statement(body_statement, indent, "body"));
                if let Some(slot) = iteration_expression {
                    Self::push_slot(work, slot, indent, "iteration");
                }
                if let Some(slot) = condition_expression {
                    Self::push_slot(work, slot, indent, "condition");
                }
                if let Some(initializer) = initializer {
                    match initializer {
                        | ForInitializer::Expression(slot) => {
                            Self::push_slot(work, slot, indent, "initializer");
                        },
                        | ForInitializer::Declaration(index) => {
                            work.push(Work::Declaration(index, indent, "initializer"));
                        },
                    }
                }
            },
            | StatementType::Return(Some(slot)) => {
                Self::push_slot(work, slot, indent, "return-value");
            },
            | StatementType::Label(_, child) | StatementType::Default(child) => {
                work.push(Work::Statement(child, indent, "labeled"));
            },
            | StatementType::Case(expression, child) => {
                work.push(Work::Statement(child, indent, "labeled"));
                match expression {
                    | ConstantExpressionSlot::Parsed(index) => {
                        work.push(Work::Expression(index.into(), indent, "case-value"));
                    },
                    | ConstantExpressionSlot::Missing(source) => {
                        work.push(Work::Missing(source, indent, "case-value"));
                    },
                }
            },
            | StatementType::Return(None)
            | StatementType::Break
            | StatementType::Continue
            | StatementType::Goto(_)
            | StatementType::Null => {},
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
            | ExpressionType::Binary { operator, .. } =>
                format!("binary {}", binary_operator_spelling(*operator)),
            | ExpressionType::Unary { operator, .. } =>
                format!("unary {}", unary_operator_spelling(*operator)),
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
            | ExpressionType::Constant(constant) =>
                format!("constant {}", constant_label(constant)),
            | ExpressionType::StringLiteral(string) => match string {
                | StringTokenType::String(contents) => {
                    format!(
                        "string {}",
                        c_quoted("", '"', context.string_cache.at(*contents))
                    )
                },
                | StringTokenType::WideString(contents) => {
                    format!(
                        "wide-string {}",
                        c_quoted("L", '"', context.string_cache.at(*contents))
                    )
                },
            },
            | ExpressionType::SizeofType(..) => "sizeof type".to_owned(),
            | ExpressionType::SizeofExpr(..) => "sizeof expression".to_owned(),
            | ExpressionType::Cast { .. } => "cast".to_owned(),
            | ExpressionType::Error => "error-expression".to_owned(),
        }
    }

    fn push_slot(work: &mut Vec<Work>, slot: ExpressionSlot, indent: usize, role: &'static str) {
        match slot {
            | ExpressionSlot::Parsed(index) => work.push(Work::Expression(index, indent, role)),
            | ExpressionSlot::Missing(source) => work.push(Work::Missing(source, indent, role)),
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
                for argument in self.expression_indices(*arguments).iter().rev() {
                    work.push(Work::Expression(*argument, indent, "argument"));
                }
                work.push(Work::Expression(*function_expression, indent, "callee"));
            },
            | ExpressionType::DirectMember {
                base_expression, ..
            }
            | ExpressionType::IndirectMember {
                base_expression, ..
            } => {
                work.push(Work::Expression(*base_expression, indent, "base"));
            },
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

    fn shared(
        output: &mut String,
        indent: usize,
        role: &str,
        kind: &str,
        index: u32,
        context: &Context,
        options: InspectionOptions,
    ) {
        Self::line(
            output,
            indent,
            &format!("{role}: {kind}#{index} (shared)"),
            None,
            context,
            options,
        );
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
            // The main file is interned first; anything else names its file
            // so declarations from headers are not mistaken for local ones.
            match context.get_source_vectors(source).first() {
                | Some(vector) if vector.source_file_index != 0 => {
                    let _ = write!(
                        output,
                        " @{}:{}:{}",
                        context.get_source_file(vector.source_file_index).display(),
                        position.line,
                        position.column
                    );
                },
                | _ => {
                    let _ = write!(output, " @{}:{}", position.line, position.column);
                },
            }
        }
        output.push('\n');
    }
}

/// Lists qualifiers in C spelling, such as `const volatile`, or `none`.
fn qualifier_list(qualifiers: TypeQualifiers) -> String {
    let names: Vec<&str> = [
        (TypeQualifiers::CONST, "const"),
        (TypeQualifiers::VOLATILE, "volatile"),
        (TypeQualifiers::RESTRICT, "restrict"),
    ]
    .into_iter()
    .filter(|&(flag, _)| qualifiers.contains(flag))
    .map(|(_, name)| name)
    .collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(" ")
    }
}

fn binary_operator_spelling(operator: BinaryOperator) -> &'static str {
    match operator {
        | BinaryOperator::Multiplication => "*",
        | BinaryOperator::Division => "/",
        | BinaryOperator::Modulo => "%",
        | BinaryOperator::Addition => "+",
        | BinaryOperator::Subtraction => "-",
        | BinaryOperator::LeftShift => "<<",
        | BinaryOperator::RightShift => ">>",
        | BinaryOperator::LessThan => "<",
        | BinaryOperator::GreaterThan => ">",
        | BinaryOperator::LessThanOrEqual => "<=",
        | BinaryOperator::GreaterThanOrEqual => ">=",
        | BinaryOperator::Equal => "==",
        | BinaryOperator::NotEqual => "!=",
        | BinaryOperator::BitwiseAnd => "&",
        | BinaryOperator::BitwiseXor => "^",
        | BinaryOperator::BitwiseOr => "|",
        | BinaryOperator::LogicalAnd => "&&",
        | BinaryOperator::LogicalOr => "||",
        | BinaryOperator::Comma => ",",
        | BinaryOperator::Subscript => "[]",
        | BinaryOperator::Assignment => "=",
        | BinaryOperator::MultiplicationAssignment => "*=",
        | BinaryOperator::DivisionAssignment => "/=",
        | BinaryOperator::ModuloAssignment => "%=",
        | BinaryOperator::AdditionAssignment => "+=",
        | BinaryOperator::SubtractionAssignment => "-=",
        | BinaryOperator::LeftShiftAssignment => "<<=",
        | BinaryOperator::RightShiftAssignment => ">>=",
        | BinaryOperator::BitwiseAndAssignment => "&=",
        | BinaryOperator::BitwiseXorAssignment => "^=",
        | BinaryOperator::BitwiseOrAssignment => "|=",
    }
}

fn unary_operator_spelling(operator: UnaryOperator) -> &'static str {
    match operator {
        | UnaryOperator::AddressOf => "&",
        | UnaryOperator::Indirection => "*",
        | UnaryOperator::Plus => "+",
        | UnaryOperator::Minus => "-",
        | UnaryOperator::BitwiseNot => "~",
        | UnaryOperator::LogicalNot => "!",
        | UnaryOperator::PreIncrement => "++ (prefix)",
        | UnaryOperator::PreDecrement => "-- (prefix)",
        | UnaryOperator::PostIncrement => "++ (postfix)",
        | UnaryOperator::PostDecrement => "-- (postfix)",
    }
}

/// Renders a constant's value and C type. Floating values print exactly:
/// `float` and `double` as the shortest decimal that round-trips, `long
/// double` in hexadecimal.
fn constant_label(constant: &Constant) -> String {
    match *constant {
        | Constant::Integer(integer) => {
            let (value, type_name) = match integer {
                | IntegerTokenType::Int(value) => (i128::from(value), "int"),
                | IntegerTokenType::Long(value) => (i128::from(value), "long"),
                | IntegerTokenType::LongLong(value) => (i128::from(value), "long long"),
                | IntegerTokenType::UnsignedInt(value) => (i128::from(value), "unsigned int"),
                | IntegerTokenType::UnsignedLong(value) => (i128::from(value), "unsigned long"),
                | IntegerTokenType::UnsignedLongLong(value) =>
                    (i128::from(value), "unsigned long long"),
            };
            format!("{value} ({type_name})")
        },
        | Constant::Float(float) => format!("{float} ({})", float.type_name()),
        | Constant::Char(CharacterTokenType::Char(c)) =>
            format!("{} (int)", c_quoted("", '\'', &c.to_string())),
        | Constant::Char(CharacterTokenType::WideChar(c)) =>
            format!("{} (wchar_t)", c_quoted("L", '\'', &c.to_string())),
        | Constant::Char(CharacterTokenType::MultiChar(value)) =>
            format!("{value} (int, multi-character)"),
    }
}
