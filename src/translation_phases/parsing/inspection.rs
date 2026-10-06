//! Deterministic, source-oriented syntax-tree inspection.
//!
//! Renders the syntax tree that translation phase 7 builds (§5.1.1.2
//! paragraph 1, p. 10; PDF p. 22) for the inspection CLI and tests. The
//! output format is bcc-rust's own; it encodes no rule of the standard
//! beyond the constant types it prints.

use std::fmt::{
    self,
    Display,
    Write,
};

use hashbrown::hash_map::Entry;
use rustc_hash::FxBuildHasher;

use super::{
    ParsedTranslationUnit,
    declaration_syntax::{
        Declaration,
        Declarator,
        Designation,
        Designator,
        DesignatorType,
        DirectDeclarator,
        EnumSpecifier,
        Enumerator,
        InitDeclarator,
        Initializer,
        InitializerElement,
        InitializerType,
        ParameterDeclaration,
        StructDeclaration,
        StructDeclarator,
        StructOrUnion,
        StructOrUnionSpecifier,
        TypeName,
        TypeQualifiers,
        TypeSpecifiers,
    },
    syntax::{
        BinaryOperator,
        BlockItem,
        ConditionalExpression,
        Constant,
        ConstantExpressionSlot,
        Expression,
        ExpressionSlot,
        ExpressionType,
        ExternalDeclaration,
        ForInitializer,
        ForStatement,
        FunctionDefinition,
        Identifier,
        Statement,
        StatementType,
        StorageClass,
        UnaryOperator,
    },
};
use crate::{
    diagnostics::write_c_quoted,
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
    util::bump::{
        ArenaMap,
        ArenaSet,
        ArenaString,
        ArenaVec,
        Bump,
    },
};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InspectionOptions {
    pub(crate) show_locations: bool,
}

enum Work<'tu> {
    Root(ExternalDeclaration<'tu>, usize),
    Declaration(&'tu Declaration<'tu>, usize, &'static str),
    InitDeclarator(InitDeclarator<'tu>, usize),
    Function(&'tu FunctionDefinition<'tu>, usize, &'static str),
    Declarator(Declarator<'tu>, usize, &'static str),
    DirectDeclarator(DirectDeclarator<'tu>, usize),
    Identifier(Identifier, usize, &'static str),
    Parameter(ParameterDeclaration<'tu>, usize),
    StructOrUnion(&'tu StructOrUnionSpecifier<'tu>, usize),
    StructDeclaration(StructDeclaration<'tu>, usize),
    StructDeclarator(StructDeclarator<'tu>, usize),
    Enum(&'tu EnumSpecifier<'tu>, usize),
    Enumerator(Enumerator<'tu>, usize),
    Statement(&'tu Statement<'tu>, usize, &'static str),
    Expression(&'tu Expression<'tu>, usize, &'static str),
    Missing(SourceVectors, usize, &'static str),
    Initializer(&'tu Initializer<'tu>, usize, &'static str),
    InitializerElement(InitializerElement<'tu>, usize),
    Designation(&'tu Designation<'tu>, usize),
    Designator(Designator<'tu>, usize),
    TypeName(&'tu TypeName<'tu>, usize, &'static str),
}

impl<'tu> ParsedTranslationUnit<'tu> {
    #[expect(
        clippy::too_many_lines,
        reason = "One iterative dispatcher keeps traversal order and cycle handling centralized."
    )]
    pub(crate) fn inspect<'a>(
        &self,
        arena: &'a Bump,
        context: &Context<'_>,
        options: InspectionOptions,
    ) -> &'a str {
        let mut output = ArenaString::new_in(arena);
        // Each working collection has its own arena, so each grows in place.
        let work_arena = Bump::new();
        let declarator_arena = Bump::new();
        let visited_arena = Bump::new();
        let mut literal_scratch = Bump::new();
        let mut work = ArenaVec::with_capacity_in(self.roots.len(), &work_arena);
        work.extend(
            self.roots
                .iter()
                .copied()
                .enumerate()
                .rev()
                .map(|(ordinal, root)| Work::Root(root, ordinal)),
        );
        let mut seen_declarators = ArenaSet::with_hasher_in(FxBuildHasher, &declarator_arena);
        // Nodes reached through references, keyed by kind and address, with
        // the order in which each was first visited.
        let mut visited = ArenaMap::with_hasher_in(FxBuildHasher, &visited_arena);

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
                        format_args!("root[{ordinal}] error"),
                        Some(source),
                        context,
                        options,
                    ),
                },
                | Work::Declaration(declaration, indent, role) => {
                    if let Some(ordinal) = first_visit(&mut visited, 0, declaration) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "declaration",
                            ordinal,
                            context,
                            options,
                        );
                        continue;
                    }
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
                            "{role}: declaration{} storage={} type={} qualifiers={} \
                             function-specifiers={}",
                            if declaration.recovered {
                                " recovered"
                            } else {
                                ""
                            },
                            declaration
                                .declaration_specifiers
                                .storage_class
                                .map_or("none", StorageClass::spelling),
                            Self::type_label(
                                declaration.declaration_specifiers.type_specifiers,
                                context,
                            ),
                            qualifier_list(declaration.declaration_specifiers.type_qualifiers),
                            if declaration
                                .declaration_specifiers
                                .function_specifiers
                                .is_inline
                            {
                                "inline"
                            } else {
                                "none"
                            },
                        ),
                        Some(declaration.source_vectors),
                        context,
                        options,
                    );
                    for init in declaration.init_declarators.iter().rev() {
                        work.push(Work::InitDeclarator(*init, indent + 1));
                    }
                    Self::push_type_details(
                        &mut work,
                        declaration.declaration_specifiers.type_specifiers,
                        indent + 1,
                    );
                },
                | Work::InitDeclarator(init, indent) => {
                    let name = init
                        .declarator
                        .identifier()
                        .map_or("<abstract>", |identifier| {
                            context.string_cache.at(identifier.name)
                        });
                    Self::line(
                        &mut output,
                        indent,
                        format_args!("declarator {name}"),
                        Some(init.source_vectors),
                        context,
                        options,
                    );
                    if let Some(initializer) = init.initializer {
                        work.push(Work::Initializer(initializer, indent + 1, "initializer"));
                    }
                    work.push(Work::Declarator(init.declarator, indent + 1, "shape"));
                },
                | Work::Function(function, indent, role) => {
                    if let Some(ordinal) = first_visit(&mut visited, 1, function) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "function",
                            ordinal,
                            context,
                            options,
                        );
                        continue;
                    }
                    let name = function
                        .declarator
                        .identifier()
                        .map_or("<anonymous>", |identifier| {
                            context.string_cache.at(identifier.name)
                        });
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
                            "{role} {name}{} type={} storage={} qualifiers={} \
                             function-specifiers={}",
                            if function.recovered { " recovered" } else { "" },
                            Self::type_label(
                                function.declaration_specifiers.type_specifiers,
                                context
                            ),
                            function
                                .declaration_specifiers
                                .storage_class
                                .map_or("none", StorageClass::spelling),
                            qualifier_list(function.declaration_specifiers.type_qualifiers),
                            if function
                                .declaration_specifiers
                                .function_specifiers
                                .is_inline
                            {
                                "inline"
                            } else {
                                "none"
                            },
                        ),
                        Some(function.source_vectors),
                        context,
                        options,
                    );
                    work.push(Work::Statement(function.body, indent + 1, "body"));
                    for declaration in function.declaration_list.iter().rev() {
                        work.push(Work::Declaration(
                            declaration,
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
                    if !seen_declarators.insert(declarator_key(declarator)) {
                        Self::line(
                            &mut output,
                            indent,
                            format_args!("{role}: declarator (shared)"),
                            None,
                            context,
                            options,
                        );
                        continue;
                    }
                    let pointers = declarator.pointer.type_qualifiers_list;
                    Self::line(
                        &mut output,
                        indent,
                        format_args!("{role}: declarator pointer-levels={}", pointers.len()),
                        Some(declarator.source_vectors),
                        context,
                        options,
                    );
                    for (level, qualifiers) in pointers.iter().enumerate() {
                        Self::line(
                            &mut output,
                            indent + 1,
                            format_args!(
                                "pointer {level} qualifiers={}",
                                qualifier_list(*qualifiers)
                            ),
                            None,
                            context,
                            options,
                        );
                    }
                    for direct in declarator.kind.iter().rev() {
                        work.push(Work::DirectDeclarator(*direct, indent + 1));
                    }
                },
                | Work::DirectDeclarator(direct, indent) => match direct {
                    | DirectDeclarator::Identifier(identifier) => {
                        work.push(Work::Identifier(identifier, indent, "identifier"));
                    },
                    | DirectDeclarator::Parenthesized(parenthesized) => {
                        let declarator = parenthesized.declarator;
                        let delimiters = parenthesized.delimiters;
                        Self::line(
                            &mut output,
                            indent,
                            "parenthesized-declarator",
                            Some(delimiters),
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
                        for identifier in parameters.iter().rev() {
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
                            format_args!(
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
                            format_args!("function variadic={is_variadic}"),
                            None,
                            context,
                            options,
                        );
                        for parameter in parameter_list.iter().rev() {
                            work.push(Work::Parameter(*parameter, indent + 1));
                        }
                    },
                },
                | Work::Identifier(identifier, indent, role) => Self::line(
                    &mut output,
                    indent,
                    format_args!("{role} {}", context.string_cache.at(identifier.name)),
                    Some(identifier.source_vectors),
                    context,
                    options,
                ),
                | Work::Parameter(parameter, indent) => {
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
                            "parameter type={} storage={} qualifiers={} function-specifiers={}",
                            Self::type_label(
                                parameter.declaration_specifiers.type_specifiers,
                                context,
                            ),
                            parameter
                                .declaration_specifiers
                                .storage_class
                                .map_or("none", StorageClass::spelling),
                            qualifier_list(parameter.declaration_specifiers.type_qualifiers),
                            if parameter
                                .declaration_specifiers
                                .function_specifiers
                                .is_inline
                            {
                                "inline"
                            } else {
                                "none"
                            },
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
                | Work::StructOrUnion(specifier, indent) => {
                    if first_visit(&mut visited, 7, specifier).is_some() {
                        continue;
                    }
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
                        format_args!("{kind} {name}"),
                        Some(specifier.source_vectors),
                        context,
                        options,
                    );
                    if let Some(declarations) = specifier.struct_declaration_list {
                        for declaration in declarations.iter().rev() {
                            work.push(Work::StructDeclaration(*declaration, indent + 1));
                        }
                    }
                },
                | Work::StructDeclaration(declaration, indent) => {
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
                            "member-declaration type={} qualifiers={}",
                            Self::type_label(declaration.type_specifiers, context),
                            qualifier_list(declaration.type_qualifiers),
                        ),
                        Some(declaration.source_vectors),
                        context,
                        options,
                    );
                    for declarator in declaration.struct_declarator_list.iter().rev() {
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
                | Work::Enum(specifier, indent) => {
                    if first_visit(&mut visited, 8, specifier).is_some() {
                        continue;
                    }
                    let name = specifier.name.map_or("<anonymous>", |identifier| {
                        context.string_cache.at(identifier.name)
                    });
                    Self::line(
                        &mut output,
                        indent,
                        format_args!("enum {name}"),
                        Some(specifier.source_vectors),
                        context,
                        options,
                    );
                    if let Some(enumerators) = specifier.enumeration_list {
                        for enumerator in enumerators.iter().rev() {
                            work.push(Work::Enumerator(*enumerator, indent + 1));
                        }
                    }
                },
                | Work::Enumerator(enumerator, indent) => {
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
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
                | Work::Statement(statement, indent, role) => {
                    if let Some(ordinal) = first_visit(&mut visited, 2, statement) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "statement",
                            ordinal,
                            context,
                            options,
                        );
                        continue;
                    }
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
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
                    Self::push_statement_children(&mut work, &statement.kind, indent + 1);
                },
                | Work::Expression(expression, indent, role) => {
                    if let Some(ordinal) = first_visit(&mut visited, 3, expression) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "expression",
                            ordinal,
                            context,
                            options,
                        );
                        continue;
                    }
                    literal_scratch.reset();
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
                            "{role}: {}{}",
                            Self::expression_label(&expression.kind, context, &literal_scratch),
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
                    Self::push_expression_children(&mut work, &expression.kind, indent + 1);
                },
                | Work::Missing(source, indent, role) => Self::line(
                    &mut output,
                    indent,
                    format_args!("{role}: missing"),
                    Some(source),
                    context,
                    options,
                ),
                | Work::Initializer(initializer, indent, role) => {
                    if let Some(ordinal) = first_visit(&mut visited, 4, initializer) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "initializer",
                            ordinal,
                            context,
                            options,
                        );
                        continue;
                    }
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
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
                        | InitializerType::InitializerList(list) => {
                            for element in list.elements.iter().rev() {
                                work.push(Work::InitializerElement(*element, indent + 1));
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
                | Work::Designation(designation, indent) => {
                    if first_visit(&mut visited, 9, designation).is_some() {
                        continue;
                    }
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
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
                    for designator in designation.designators.iter().rev() {
                        work.push(Work::Designator(*designator, indent + 1));
                    }
                },
                | Work::Designator(designator, indent) => {
                    let label = fmt::from_fn(|f| match designator.kind {
                        | DesignatorType::Array(_) => f.write_str("array-designator"),
                        | DesignatorType::Field(identifier) => write!(
                            f,
                            "field-designator .{}",
                            context.string_cache.at(identifier.name)
                        ),
                        | DesignatorType::Error => f.write_str("error-designator"),
                    });
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
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
                | Work::TypeName(type_name, indent, role) => {
                    if let Some(ordinal) = first_visit(&mut visited, 5, type_name) {
                        Self::shared(
                            &mut output,
                            indent,
                            role,
                            "type-name",
                            ordinal,
                            context,
                            options,
                        );
                        continue;
                    }
                    Self::line(
                        &mut output,
                        indent,
                        format_args!(
                            "{role}: type-name{} type={} qualifiers={}",
                            if type_name.recovered {
                                " recovered"
                            } else {
                                ""
                            },
                            Self::type_label(
                                type_name.declaration_specifiers.type_specifiers,
                                context,
                            ),
                            qualifier_list(type_name.declaration_specifiers.type_qualifiers),
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
        output.into_str()
    }

    fn push_type_details(
        work: &mut ArenaVec<'_, Work<'tu>>,
        specifiers: TypeSpecifiers<'tu>,
        indent: usize,
    ) {
        match specifiers {
            | TypeSpecifiers::StructOrUnion(index) => work.push(Work::StructOrUnion(index, indent)),
            | TypeSpecifiers::Enum(index) => work.push(Work::Enum(index, indent)),
            | _ => {},
        }
    }

    fn type_label<'a>(specifiers: TypeSpecifiers<'a>, context: &'a Context<'_>) -> impl Display {
        fmt::from_fn(move |f| match specifiers {
            | TypeSpecifiers::TypedefName(identifier) => {
                write!(f, "typedef {}", context.string_cache.at(identifier.name))
            },
            | TypeSpecifiers::StructOrUnion(specifier) => {
                let kind = match specifier.struct_or_union {
                    | StructOrUnion::Struct => "struct",
                    | StructOrUnion::Union => "union",
                };
                match specifier.identifier {
                    | None => write!(f, "{kind} <anonymous>"),
                    | Some(identifier) =>
                        write!(f, "{kind} {}", context.string_cache.at(identifier.name)),
                }
            },
            | TypeSpecifiers::Enum(specifier) => match specifier.name {
                | None => f.write_str("enum <anonymous>"),
                | Some(identifier) =>
                    write!(f, "enum {}", context.string_cache.at(identifier.name)),
            },
            | _ => write!(f, "{specifiers}"),
        })
    }

    fn push_statement_children(
        work: &mut ArenaVec<'_, Work<'tu>>,
        kind: &StatementType<'tu>,
        indent: usize,
    ) {
        match *kind {
            | StatementType::Compound { items } =>
                for item in items.iter().rev() {
                    match *item {
                        | BlockItem::Declaration(index) => {
                            work.push(Work::Declaration(index, indent, "block-item"));
                        },
                        | BlockItem::Statement(index) => {
                            work.push(Work::Statement(index, indent, "block-item"));
                        },
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
            } => {
                work.push(Work::Statement(body_statement, indent, "body"));
                Self::push_slot(work, condition_expression, indent, "condition");
            },
            | StatementType::DoWhile {
                condition_expression,
                body_statement,
            } => {
                Self::push_slot(work, condition_expression, indent, "condition");
                work.push(Work::Statement(body_statement, indent, "body"));
            },
            | StatementType::For(&ForStatement {
                initializer,
                condition_expression,
                iteration_expression,
                body_statement,
            }) => {
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

    fn statement_label<'a>(kind: &'a StatementType<'tu>, context: &'a Context<'_>) -> impl Display {
        fmt::from_fn(move |f| {
            f.write_str(match kind {
                | StatementType::Label(identifier, _) => {
                    return write!(f, "label {}", context.string_cache.at(identifier.name));
                },
                | StatementType::Goto(identifier) => {
                    return write!(f, "goto {}", context.string_cache.at(identifier.name));
                },
                | StatementType::Case(..) => "case",
                | StatementType::Default(..) => "default",
                | StatementType::Compound { .. } => "compound",
                | StatementType::Expression(..) => "expression",
                | StatementType::If { .. } => "if",
                | StatementType::Switch { .. } => "switch",
                | StatementType::While { .. } => "while",
                | StatementType::DoWhile { .. } => "do-while",
                | StatementType::For(_) => "for",
                | StatementType::Continue => "continue",
                | StatementType::Break => "break",
                | StatementType::Return(..) => "return",
                | StatementType::Null => "null",
            })
        })
    }

    /// Spells a string literal in `scratch`.
    fn expression_label<'a>(
        kind: &'a ExpressionType<'tu>,
        context: &'a Context<'_>,
        scratch: &'a Bump,
    ) -> impl Display {
        fmt::from_fn(move |f| {
            f.write_str(match kind {
                | ExpressionType::Binary { operator, .. } => {
                    return write!(f, "binary {}", binary_operator_spelling(*operator));
                },
                | ExpressionType::Unary { operator, .. } => {
                    return write!(f, "unary {}", unary_operator_spelling(*operator));
                },
                | ExpressionType::DirectMember { member, .. } => {
                    return write!(f, "member .{}", context.string_cache.at(member.name));
                },
                | ExpressionType::IndirectMember { member, .. } => {
                    return write!(f, "member ->{}", context.string_cache.at(member.name));
                },
                | ExpressionType::Identifier(identifier) => {
                    return write!(f, "identifier {}", context.string_cache.at(identifier.name));
                },
                | ExpressionType::Constant(constant) => {
                    return write!(f, "constant {}", constant_label(constant));
                },
                | ExpressionType::StringLiteral(string) => {
                    return match string {
                        | StringTokenType::String(contents) => write!(
                            f,
                            "string {}",
                            context.literal_spelling_in(scratch, scratch, *contents, false)
                        ),
                        | StringTokenType::WideString(contents) => write!(
                            f,
                            "wide-string {}",
                            context.literal_spelling_in(scratch, scratch, *contents, true)
                        ),
                    };
                },
                | ExpressionType::Parenthesized { .. } => "parenthesized",
                | ExpressionType::Conditional(_) => "conditional ?:",
                | ExpressionType::Call { .. } => "call",
                | ExpressionType::CompoundLiteral { .. } => "compound-literal",
                | ExpressionType::SizeofType(..) => "sizeof type",
                | ExpressionType::SizeofExpr(..) => "sizeof expression",
                | ExpressionType::Cast { .. } => "cast",
                | ExpressionType::Error => "error-expression",
            })
        })
    }

    fn push_slot(
        work: &mut ArenaVec<'_, Work<'tu>>,
        slot: ExpressionSlot<'tu>,
        indent: usize,
        role: &'static str,
    ) {
        match slot {
            | ExpressionSlot::Parsed(index) => work.push(Work::Expression(index, indent, role)),
            | ExpressionSlot::Missing(source) => work.push(Work::Missing(source, indent, role)),
        }
    }

    fn push_expression_children(
        work: &mut ArenaVec<'_, Work<'tu>>,
        kind: &ExpressionType<'tu>,
        indent: usize,
    ) {
        match kind {
            | ExpressionType::Parenthesized { expression }
            | ExpressionType::Unary {
                operand_expression: expression,
                ..
            }
            | ExpressionType::SizeofExpr(expression) => {
                work.push(Work::Expression(expression, indent, "operand"));
            },
            | ExpressionType::Conditional(ConditionalExpression {
                condition_expression,
                then_expression,
                else_expression,
            }) => {
                work.push(Work::Expression(else_expression, indent, "else"));
                work.push(Work::Expression(then_expression, indent, "then"));
                work.push(Work::Expression(condition_expression, indent, "condition"));
            },
            | ExpressionType::Binary {
                left_expression,
                right_expression,
                ..
            } => {
                work.push(Work::Expression(right_expression, indent, "rhs"));
                work.push(Work::Expression(left_expression, indent, "lhs"));
            },
            | ExpressionType::Call {
                function_expression,
                arguments,
            } => {
                for argument in arguments.iter().rev() {
                    work.push(Work::Expression(argument, indent, "argument"));
                }
                work.push(Work::Expression(function_expression, indent, "callee"));
            },
            | ExpressionType::DirectMember {
                base_expression, ..
            }
            | ExpressionType::IndirectMember {
                base_expression, ..
            } => {
                work.push(Work::Expression(base_expression, indent, "base"));
            },
            | ExpressionType::CompoundLiteral {
                type_name,
                initializer,
            } => {
                work.push(Work::Initializer(initializer, indent, "initializer"));
                work.push(Work::TypeName(type_name, indent, "type"));
            },
            | ExpressionType::Cast {
                target_type,
                operand_expression,
            } => {
                work.push(Work::Expression(operand_expression, indent, "operand"));
                work.push(Work::TypeName(target_type, indent, "target-type"));
            },
            | ExpressionType::SizeofType(type_name) => {
                work.push(Work::TypeName(type_name, indent, "operand-type"));
            },
            | ExpressionType::Identifier(_)
            | ExpressionType::Constant(_)
            | ExpressionType::StringLiteral(_)
            | ExpressionType::Error => {},
        }
    }

    fn shared(
        output: &mut ArenaString<'_>,
        indent: usize,
        role: &str,
        kind: &str,
        index: impl Display,
        context: &Context<'_>,
        options: InspectionOptions,
    ) {
        Self::line(
            output,
            indent,
            format_args!("{role}: {kind}#{index} (shared)"),
            None,
            context,
            options,
        );
    }

    fn line(
        output: &mut ArenaString<'_>,
        indent: usize,
        text: impl Display,
        source: Option<SourceVectors>,
        context: &Context<'_>,
        options: InspectionOptions,
    ) {
        for _ in 0..indent.min(32) {
            output.push_str("  ");
        }
        if indent > 32 {
            let _ = write!(output, "[depth={indent}] ");
        }
        let _ = write!(output, "{text}");
        if options.show_locations
            && let Some(source) = source
            && source.length() > 0
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
fn qualifier_list(qualifiers: TypeQualifiers) -> impl Display {
    fmt::from_fn(move |f| {
        let mut names = [
            (TypeQualifiers::CONST, "const"),
            (TypeQualifiers::VOLATILE, "volatile"),
            (TypeQualifiers::RESTRICT, "restrict"),
        ]
        .into_iter()
        .filter(|&(flag, _)| qualifiers.contains(flag))
        .map(|(_, name)| name);
        let Some(first) = names.next() else {
            return f.write_str("none");
        };
        f.write_str(first)?;
        for name in names {
            write!(f, " {name}")?;
        }
        Ok(())
    })
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
///
/// C99: integer constant types follow §6.4.4.1 paragraph 5, p. 55;
/// PDF p. 67; a character constant has type `int` and a wide one `wchar_t`
/// (§6.4.4.4 paragraphs 10-11, p. 61; PDF p. 73), and a multi-character
/// constant's value is implementation-defined (§6.4.4.4 paragraph 10).
fn constant_label(constant: &Constant) -> impl Display {
    fmt::from_fn(move |f| match *constant {
        | Constant::Integer(integer) => {
            let (value, type_name) = match integer {
                | IntegerTokenType::Int(value) => (i128::from(value), "int"),
                | IntegerTokenType::Long(value) => (i128::from(value.get()), "long"),
                | IntegerTokenType::LongLong(value) => (i128::from(value.get()), "long long"),
                | IntegerTokenType::UnsignedInt(value) => (i128::from(value), "unsigned int"),
                | IntegerTokenType::UnsignedLong(value) =>
                    (i128::from(value.get()), "unsigned long"),
                | IntegerTokenType::UnsignedLongLong(value) =>
                    (i128::from(value.get()), "unsigned long long"),
            };
            write!(f, "{value} ({type_name})")
        },
        | Constant::Float(float) => write!(f, "{float} ({})", float.type_name()),
        | Constant::Char(CharacterTokenType::Char(c)) => {
            write_c_quoted(f, "", '\'', c.encode_utf8(&mut [0; 4]))?;
            f.write_str(" (int)")
        },
        | Constant::Char(CharacterTokenType::WideChar(c)) => {
            match char::from_u32(c) {
                | Some(c) => write_c_quoted(f, "L", '\'', c.encode_utf8(&mut [0; 4]))?,
                | None => write!(f, "L\'\\x{c:x}\'")?,
            }
            f.write_str(" (wchar_t)")
        },
        | Constant::Char(CharacterTokenType::MultiChar(value)) =>
            write!(f, "{value} (int, multi-character)"),
    })
}

/// Records a node reached through a reference. Returns `None` on the first
/// visit, or the order in which the node was first visited when it is
/// reached again through another parent.
fn first_visit<T>(
    visited: &mut ArenaMap<'_, (u8, usize), usize>,
    kind: u8,
    node: &T,
) -> Option<usize> {
    let ordinal = visited.len();
    match visited.entry((kind, std::ptr::from_ref(node).addr())) {
        | Entry::Occupied(first) => Some(*first.get()),
        | Entry::Vacant(slot) => {
            let _ = slot.insert(ordinal);
            None
        },
    }
}

/// Identifies a declarator by its two lists: where each lives in the
/// translation-unit arena and its length. Every empty list of one kind
/// shares an address, so declarators with the same empty lists share a key.
fn declarator_key(declarator: Declarator<'_>) -> (usize, usize, usize, usize) {
    (
        declarator.kind.as_ptr().addr(),
        declarator.kind.len(),
        declarator.pointer.type_qualifiers_list.as_ptr().addr(),
        declarator.pointer.type_qualifiers_list.len(),
    )
}
