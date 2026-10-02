//! Arena storage for syntax nodes and the validated [`SyntaxTree`] view.

use std::{
    fmt::{
        Debug,
        Formatter,
        Result as FmtResult,
    },
    ops::{
        Deref,
        DerefMut,
    },
    slice::Iter as SliceIter,
};

use super::{
    declaration_syntax::{
        Declaration,
        DeclarationSpecifiers,
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
        StructOrUnionSpecifier,
        TypeName,
        TypeQualifiers,
        TypeSpecifiers,
    },
    syntax::{
        BlockItem,
        ConstantExpressionSlot,
        DeclarationIndex,
        DesignationIndex,
        EnumSpecifierIndex,
        Expression,
        ExpressionIndex,
        ExpressionSlot,
        ExpressionType,
        ExternalDeclaration,
        ForInitializer,
        FunctionDefinition,
        FunctionDefinitionIndex,
        Identifier,
        InitializerIndex,
        Statement,
        StatementIndex,
        StatementType,
        StructOrUnionSpecifierIndex,
        SyntaxList,
        SyntaxTreeId,
        TypeNameIndex,
    },
};

/// One syntax arena. It reads and updates in place like a slice, but only
/// [`Parser::push_syntax`](super::Parser::push_syntax) and
/// [`Parser::append_syntax`](super::Parser::append_syntax) add nodes, so the
/// parser's running node total stays exact.
pub(super) struct Arena<T>(pub(super) Vec<T>);

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: Debug> Debug for Arena<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        self.0.fmt(f)
    }
}

impl<T> Deref for Arena<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T> DerefMut for Arena<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

impl<'a, T> IntoIterator for &'a Arena<T> {
    type IntoIter = SliceIter<'a, T>;
    type Item = &'a T;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// Owns arena storage for every syntax domain constructed by parser frames.
///
/// AST nodes refer to these vectors through typed handles and
/// [`VectorSlice`](crate::util::vector_slice::VectorSlice)
/// ranges, keeping nested syntax compact and avoiding recursive ownership.
///
/// C99: the stored language syntax spans expressions through external
/// definitions, §6.5-§6.9, pp. 67-144; PDF pp. 79-156. Arena storage is an
/// implementation strategy, not a normative C concept.
#[derive(Debug, Default)]
pub(super) struct SyntaxStore {
    pub(super) type_names:                 Arena<TypeName>,
    pub(super) declarations:               Arena<Declaration>,
    pub(super) init_declarators:           Arena<InitDeclarator>,
    pub(super) initializers:               Arena<Initializer>,
    pub(super) initializer_elements:       Arena<InitializerElement>,
    pub(super) designations:               Arena<Designation>,
    pub(super) designators:                Arena<Designator>,
    pub(super) expressions:                Arena<Expression>,
    pub(super) expression_indices:         Arena<ExpressionIndex>,
    pub(super) statements:                 Arena<Statement>,
    pub(super) block_items:                Arena<BlockItem>,
    pub(super) function_definitions:       Arena<FunctionDefinition>,
    pub(super) declaration_indices:        Arena<DeclarationIndex>,
    pub(super) type_qualifiers:            Arena<TypeQualifiers>,
    pub(super) direct_declarators:         Arena<DirectDeclarator>,
    pub(super) identifiers:                Arena<Identifier>,
    pub(super) parameter_declarations:     Arena<ParameterDeclaration>,
    pub(super) struct_or_union_specifiers: Arena<StructOrUnionSpecifier>,
    pub(super) struct_declarations:        Arena<StructDeclaration>,
    pub(super) struct_declarators:         Arena<StructDeclarator>,
    pub(super) enum_specifiers:            Arena<EnumSpecifier>,
    pub(super) enumerators:                Arena<Enumerator>,
}

#[derive(Clone, Copy)]
pub(super) struct SyntaxStoreCheckpoint {
    type_names:                 usize,
    declarations:               usize,
    init_declarators:           usize,
    initializers:               usize,
    initializer_elements:       usize,
    designations:               usize,
    designators:                usize,
    expressions:                usize,
    expression_indices:         usize,
    statements:                 usize,
    block_items:                usize,
    function_definitions:       usize,
    declaration_indices:        usize,
    type_qualifiers:            usize,
    direct_declarators:         usize,
    identifiers:                usize,
    parameter_declarations:     usize,
    struct_or_union_specifiers: usize,
    struct_declarations:        usize,
    struct_declarators:         usize,
    enum_specifiers:            usize,
    enumerators:                usize,
}

/// Validated, read-only access to arena-backed C syntax.
///
/// Typed handles cannot be constructed outside this module. List-bearing
/// views resolve their ranges here so callers never coordinate raw vectors or
/// perform unchecked arithmetic themselves.
#[derive(Debug)]
pub(crate) struct SyntaxTree {
    syntax_id: SyntaxTreeId,
    store:     SyntaxStore,
}

impl SyntaxTree {
    pub(super) fn new(
        syntax_id: SyntaxTreeId,
        store: SyntaxStore,
        roots: &[ExternalDeclaration],
    ) -> Self {
        let tree = Self { syntax_id, store };
        tree.validate(roots);
        tree
    }

    fn validate(&self, roots: &[ExternalDeclaration]) {
        for root in roots {
            match *root {
                | ExternalDeclaration::Declaration(index)
                | ExternalDeclaration::RecoveredDeclaration(index) => {
                    let _ = self.declaration(index);
                },
                | ExternalDeclaration::FunctionDefinition(index)
                | ExternalDeclaration::RecoveredFunctionDefinition(index) => {
                    let _ = self.function_definition(index);
                },
                | ExternalDeclaration::Error(_) => {},
            }
        }
        for declaration in &self.store.declarations {
            let _ = self.init_declarators(declaration.init_declarators);
            self.validate_specifiers(declaration.declaration_specifiers);
        }
        for definition in &self.store.function_definitions {
            self.validate_declarator(definition.declarator);
            for declaration in self.declaration_indices(definition.declaration_list) {
                let _ = self.declaration(*declaration);
            }
            let _ = self.statement(definition.body);
            self.validate_specifiers(definition.declaration_specifiers);
        }
        for init in &self.store.init_declarators {
            self.validate_declarator(init.declarator);
            if let Some(initializer) = init.initializer {
                let _ = self.initializer(initializer);
            }
        }
        for expression in &self.store.expressions {
            match expression.kind {
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Unary {
                    operand_expression: expression,
                    ..
                }
                | ExpressionType::SizeofExpr(expression) => {
                    let _ = self.expression(expression);
                },
                | ExpressionType::Conditional {
                    condition_expression,
                    then_expression,
                    else_expression,
                } => {
                    let _ = self.expression(condition_expression);
                    let _ = self.expression(then_expression);
                    let _ = self.expression(else_expression);
                },
                | ExpressionType::Binary {
                    left_expression,
                    right_expression,
                    ..
                } => {
                    let _ = self.expression(left_expression);
                    let _ = self.expression(right_expression);
                },
                | ExpressionType::Call {
                    function_expression,
                    arguments,
                } => {
                    let _ = self.expression(function_expression);
                    for argument in self.expression_indices(arguments) {
                        let _ = self.expression(*argument);
                    }
                },
                | ExpressionType::DirectMember {
                    base_expression, ..
                }
                | ExpressionType::IndirectMember {
                    base_expression, ..
                } => {
                    let _ = self.expression(base_expression);
                },
                | ExpressionType::CompoundLiteral {
                    type_name,
                    initializer,
                } => {
                    let _ = self.type_name(type_name);
                    let _ = self.initializer(initializer);
                },
                | ExpressionType::SizeofType(type_name) => {
                    let _ = self.type_name(type_name);
                },
                | ExpressionType::Cast {
                    target_type,
                    operand_expression,
                } => {
                    let _ = self.type_name(target_type);
                    let _ = self.expression(operand_expression);
                },
                | ExpressionType::Identifier(_)
                | ExpressionType::Constant(_)
                | ExpressionType::StringLiteral(_)
                | ExpressionType::Error => {},
            }
        }
        for expression in &self.store.expression_indices {
            let _ = self.expression(*expression);
        }
        for initializer in &self.store.initializers {
            match initializer.kind {
                | InitializerType::AssignmentExpression(expression) => {
                    let _ = self.expression(expression);
                },
                | InitializerType::InitializerList(elements) => {
                    let _ = self.initializer_elements(elements);
                },
            }
        }
        for element in &self.store.initializer_elements {
            if let Some(designation) = element.designation {
                let _ = self.designation(designation);
            }
            let _ = self.initializer(element.initializer);
        }
        for designation in &self.store.designations {
            let _ = self.designators(designation.designators);
        }
        for designator in &self.store.designators {
            if let DesignatorType::Array(expression) = designator.kind {
                let _ = self.expression(expression.into());
            }
        }
        for statement in &self.store.statements {
            match statement.kind {
                | StatementType::Compound { items } => {
                    let _ = self.block_items(items);
                },
                | StatementType::Expression(slot) => self.validate_expression_slot(slot),
                | StatementType::If {
                    condition_expression,
                    then_statement,
                    else_statement,
                } => {
                    self.validate_expression_slot(condition_expression);
                    let _ = self.statement(then_statement);
                    if let Some(statement) = else_statement {
                        let _ = self.statement(statement);
                    }
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
                    self.validate_expression_slot(condition_expression);
                    let _ = self.statement(body_statement);
                },
                | StatementType::For {
                    initializer,
                    condition_expression,
                    iteration_expression,
                    body_statement,
                } => {
                    if let Some(initializer) = initializer {
                        match initializer {
                            | ForInitializer::Expression(slot) => {
                                self.validate_expression_slot(slot);
                            },
                            | ForInitializer::Declaration(declaration) => {
                                let _ = self.declaration(declaration);
                            },
                        }
                    }
                    if let Some(slot) = condition_expression {
                        self.validate_expression_slot(slot);
                    }
                    if let Some(slot) = iteration_expression {
                        self.validate_expression_slot(slot);
                    }
                    let _ = self.statement(body_statement);
                },
                | StatementType::Return(slot) =>
                    if let Some(slot) = slot {
                        self.validate_expression_slot(slot);
                    },
                | StatementType::Label(_, child) | StatementType::Default(child) => {
                    let _ = self.statement(child);
                },
                | StatementType::Case(expression, child) => {
                    if let ConstantExpressionSlot::Parsed(expression) = expression {
                        let _ = self.expression(expression.into());
                    }
                    let _ = self.statement(child);
                },
                | StatementType::Break
                | StatementType::Continue
                | StatementType::Goto(_)
                | StatementType::Null => {},
            }
        }
        for item in &self.store.block_items {
            match *item {
                | BlockItem::Declaration(declaration) => {
                    let _ = self.declaration(declaration);
                },
                | BlockItem::Statement(statement) => {
                    let _ = self.statement(statement);
                },
            }
        }
        for declaration in &self.store.declaration_indices {
            let _ = self.declaration(*declaration);
        }
        for type_name in &self.store.type_names {
            self.validate_specifiers(type_name.declaration_specifiers);
            if let Some(declarator) = type_name.declarator {
                self.validate_declarator(declarator);
            }
        }
        for direct in &self.store.direct_declarators {
            match *direct {
                | DirectDeclarator::Parenthesized(declarator) => {
                    self.validate_declarator(declarator);
                },
                | DirectDeclarator::KAndRStyleFunction { parameters } => {
                    let _ = self.identifiers(parameters);
                },
                | DirectDeclarator::Array {
                    assignment_expression,
                    ..
                } =>
                    if let Some(expression) = assignment_expression {
                        let _ = self.expression(expression);
                    },
                | DirectDeclarator::Function { parameter_list, .. } => {
                    let _ = self.parameter_declarations(parameter_list);
                },
                | DirectDeclarator::Identifier(_) => {},
            }
        }
        for parameter in &self.store.parameter_declarations {
            self.validate_specifiers(parameter.declaration_specifiers);
            if let Some(declarator) = parameter.declarator {
                self.validate_declarator(declarator);
            }
        }
        for specifier in &self.store.struct_or_union_specifiers {
            if let Some(declarations) = specifier.struct_declaration_list {
                let _ = self.struct_declarations(declarations);
            }
        }
        for declaration in &self.store.struct_declarations {
            self.validate_type_specifiers(declaration.type_specifiers);
            let _ = self.struct_declarators(declaration.struct_declarator_list);
        }
        for declarator in &self.store.struct_declarators {
            if let Some(syntax) = declarator.declarator {
                self.validate_declarator(syntax);
            }
            if let Some(expression) = declarator.bitfield_width {
                let _ = self.expression(expression.into());
            }
        }
        for specifier in &self.store.enum_specifiers {
            if let Some(enumerators) = specifier.enumeration_list {
                let _ = self.enumerators(enumerators);
            }
        }
        for enumerator in &self.store.enumerators {
            if let Some(expression) = enumerator.expression {
                let _ = self.expression(expression.into());
            }
        }
    }

    fn validate_specifiers(&self, specifiers: DeclarationSpecifiers) {
        self.validate_type_specifiers(specifiers.type_specifiers);
    }

    fn validate_type_specifiers(&self, specifiers: TypeSpecifiers) {
        match specifiers {
            | TypeSpecifiers::StructOrUnion(index) => {
                let _ = self.struct_or_union_specifier(index);
            },
            | TypeSpecifiers::Enum(index) => {
                let _ = self.enum_specifier(index);
            },
            | _ => {},
        }
    }

    fn validate_declarator(&self, declarator: Declarator) {
        let _ = self.pointer_qualifiers(declarator.pointer.type_qualifiers_list);
        let _ = self.direct_declarators(declarator.kind);
    }

    fn validate_expression_slot(&self, slot: ExpressionSlot) {
        if let ExpressionSlot::Parsed(expression) = slot {
            let _ = self.expression(expression);
        }
    }

    fn checked_handle(
        &self,
        syntax_id: SyntaxTreeId,
        index: u32,
        arena_len: usize,
        kind: &'static str,
    ) -> usize {
        assert_eq!(
            syntax_id, self.syntax_id,
            "{kind} handle belongs to a different syntax tree"
        );
        let index = index as usize;
        assert!(index < arena_len, "validated {kind} handle");
        index
    }

    fn checked_slice<'a, T>(&self, arena: &'a [T], range: SyntaxList<T>) -> &'a [T] {
        assert_eq!(
            range.syntax_id(),
            self.syntax_id,
            "syntax-list handle belongs to a different syntax tree"
        );
        if range.length() == 0 {
            return &arena[..0];
        }
        let start = range.start_index() as usize;
        let end = start
            .checked_add(range.length() as usize)
            .expect("validated syntax range length overflowed usize");
        arena
            .get(start..end)
            .expect("parser produced an invalid typed syntax range")
    }

    pub(crate) fn declaration(&self, index: DeclarationIndex) -> DeclarationView<'_> {
        let index = self.checked_handle(
            index.1,
            index.0,
            self.store.declarations.len(),
            "declaration",
        );
        let declaration = self
            .store
            .declarations
            .get(index)
            .expect("validated declaration handle");
        DeclarationView {
            tree: self,
            declaration,
        }
    }

    pub(crate) fn function_definition(
        &self,
        index: FunctionDefinitionIndex,
    ) -> &FunctionDefinition {
        let index = self.checked_handle(
            index.1,
            index.0,
            self.store.function_definitions.len(),
            "function-definition",
        );
        self.store
            .function_definitions
            .get(index)
            .expect("validated function-definition handle")
    }

    pub(crate) fn statement(&self, index: StatementIndex) -> &Statement {
        let index = self.checked_handle(index.1, index.0, self.store.statements.len(), "statement");
        self.store
            .statements
            .get(index)
            .expect("validated statement handle")
    }

    pub(crate) fn expression(&self, index: ExpressionIndex) -> &Expression {
        let index =
            self.checked_handle(index.1, index.0, self.store.expressions.len(), "expression");
        self.store
            .expressions
            .get(index)
            .expect("validated expression handle")
    }

    pub(crate) fn type_name(&self, index: TypeNameIndex) -> &TypeName {
        let index = self.checked_handle(index.1, index.0, self.store.type_names.len(), "type-name");
        self.store
            .type_names
            .get(index)
            .expect("validated type-name handle")
    }

    pub(crate) fn initializer(&self, index: InitializerIndex) -> InitializerView<'_> {
        let index = self.checked_handle(
            index.1,
            index.0,
            self.store.initializers.len(),
            "initializer",
        );
        let initializer = self
            .store
            .initializers
            .get(index)
            .expect("validated initializer handle");
        InitializerView {
            tree: self,
            initializer,
        }
    }

    pub(crate) fn designation(&self, index: DesignationIndex) -> &Designation {
        let index = self.checked_handle(
            index.1,
            index.0,
            self.store.designations.len(),
            "designation",
        );
        self.store
            .designations
            .get(index)
            .expect("validated designation handle")
    }

    pub(crate) fn struct_or_union_specifier(
        &self,
        index: StructOrUnionSpecifierIndex,
    ) -> &StructOrUnionSpecifier {
        let index = self.checked_handle(
            index.1,
            index.0,
            self.store.struct_or_union_specifiers.len(),
            "struct-or-union",
        );
        self.store
            .struct_or_union_specifiers
            .get(index)
            .expect("validated struct-or-union handle")
    }

    pub(crate) fn enum_specifier(&self, index: EnumSpecifierIndex) -> &EnumSpecifier {
        let index = self.checked_handle(index.1, index.0, self.store.enum_specifiers.len(), "enum");
        self.store
            .enum_specifiers
            .get(index)
            .expect("validated enum handle")
    }

    pub(crate) fn init_declarators(&self, range: SyntaxList<InitDeclarator>) -> &[InitDeclarator] {
        self.checked_slice(&self.store.init_declarators, range)
    }

    pub(crate) fn initializer_elements(
        &self,
        range: SyntaxList<InitializerElement>,
    ) -> &[InitializerElement] {
        self.checked_slice(&self.store.initializer_elements, range)
    }

    pub(crate) fn designators(&self, range: SyntaxList<Designator>) -> &[Designator] {
        self.checked_slice(&self.store.designators, range)
    }

    pub(crate) fn expression_indices(
        &self,
        range: SyntaxList<ExpressionIndex>,
    ) -> &[ExpressionIndex] {
        self.checked_slice(&self.store.expression_indices, range)
    }

    pub(crate) fn block_items(&self, range: SyntaxList<BlockItem>) -> &[BlockItem] {
        self.checked_slice(&self.store.block_items, range)
    }

    pub(crate) fn declaration_indices(
        &self,
        range: SyntaxList<DeclarationIndex>,
    ) -> &[DeclarationIndex] {
        self.checked_slice(&self.store.declaration_indices, range)
    }

    pub(crate) fn pointer_qualifiers(
        &self,
        range: SyntaxList<TypeQualifiers>,
    ) -> &[TypeQualifiers] {
        self.checked_slice(&self.store.type_qualifiers, range)
    }

    pub(crate) fn direct_declarators(
        &self,
        range: SyntaxList<DirectDeclarator>,
    ) -> &[DirectDeclarator] {
        self.checked_slice(&self.store.direct_declarators, range)
    }

    pub(crate) fn identifiers(&self, range: SyntaxList<Identifier>) -> &[Identifier] {
        self.checked_slice(&self.store.identifiers, range)
    }

    pub(crate) fn parameter_declarations(
        &self,
        range: SyntaxList<ParameterDeclaration>,
    ) -> &[ParameterDeclaration] {
        self.checked_slice(&self.store.parameter_declarations, range)
    }

    pub(crate) fn struct_declarations(
        &self,
        range: SyntaxList<StructDeclaration>,
    ) -> &[StructDeclaration] {
        self.checked_slice(&self.store.struct_declarations, range)
    }

    pub(crate) fn struct_declarators(
        &self,
        range: SyntaxList<StructDeclarator>,
    ) -> &[StructDeclarator] {
        self.checked_slice(&self.store.struct_declarators, range)
    }

    pub(crate) fn enumerators(&self, range: SyntaxList<Enumerator>) -> &[Enumerator] {
        self.checked_slice(&self.store.enumerators, range)
    }

    pub(crate) fn raw_debug(&self) -> impl Debug + '_ {
        &self.store
    }
}

impl SyntaxStore {
    pub(super) fn checkpoint(&self) -> SyntaxStoreCheckpoint {
        SyntaxStoreCheckpoint {
            type_names:                 self.type_names.len(),
            declarations:               self.declarations.len(),
            init_declarators:           self.init_declarators.len(),
            initializers:               self.initializers.len(),
            initializer_elements:       self.initializer_elements.len(),
            designations:               self.designations.len(),
            designators:                self.designators.len(),
            expressions:                self.expressions.len(),
            expression_indices:         self.expression_indices.len(),
            statements:                 self.statements.len(),
            block_items:                self.block_items.len(),
            function_definitions:       self.function_definitions.len(),
            declaration_indices:        self.declaration_indices.len(),
            type_qualifiers:            self.type_qualifiers.len(),
            direct_declarators:         self.direct_declarators.len(),
            identifiers:                self.identifiers.len(),
            parameter_declarations:     self.parameter_declarations.len(),
            struct_or_union_specifiers: self.struct_or_union_specifiers.len(),
            struct_declarations:        self.struct_declarations.len(),
            struct_declarators:         self.struct_declarators.len(),
            enum_specifiers:            self.enum_specifiers.len(),
            enumerators:                self.enumerators.len(),
        }
    }

    pub(super) fn restore(&mut self, checkpoint: SyntaxStoreCheckpoint) {
        self.type_names.0.truncate(checkpoint.type_names);
        self.declarations.0.truncate(checkpoint.declarations);
        self.init_declarators
            .0
            .truncate(checkpoint.init_declarators);
        self.initializers.0.truncate(checkpoint.initializers);
        self.initializer_elements
            .0
            .truncate(checkpoint.initializer_elements);
        self.designations.0.truncate(checkpoint.designations);
        self.designators.0.truncate(checkpoint.designators);
        self.expressions.0.truncate(checkpoint.expressions);
        self.expression_indices
            .0
            .truncate(checkpoint.expression_indices);
        self.statements.0.truncate(checkpoint.statements);
        self.block_items.0.truncate(checkpoint.block_items);
        self.function_definitions
            .0
            .truncate(checkpoint.function_definitions);
        self.declaration_indices
            .0
            .truncate(checkpoint.declaration_indices);
        self.type_qualifiers.0.truncate(checkpoint.type_qualifiers);
        self.direct_declarators
            .0
            .truncate(checkpoint.direct_declarators);
        self.identifiers.0.truncate(checkpoint.identifiers);
        self.parameter_declarations
            .0
            .truncate(checkpoint.parameter_declarations);
        self.struct_or_union_specifiers
            .0
            .truncate(checkpoint.struct_or_union_specifiers);
        self.struct_declarations
            .0
            .truncate(checkpoint.struct_declarations);
        self.struct_declarators
            .0
            .truncate(checkpoint.struct_declarators);
        self.enum_specifiers.0.truncate(checkpoint.enum_specifiers);
        self.enumerators.0.truncate(checkpoint.enumerators);
    }

    pub(super) fn node_count(&self) -> usize {
        self.type_names.len()
            + self.declarations.len()
            + self.init_declarators.len()
            + self.initializers.len()
            + self.initializer_elements.len()
            + self.designations.len()
            + self.designators.len()
            + self.expressions.len()
            + self.expression_indices.len()
            + self.statements.len()
            + self.block_items.len()
            + self.function_definitions.len()
            + self.declaration_indices.len()
            + self.type_qualifiers.len()
            + self.direct_declarators.len()
            + self.identifiers.len()
            + self.parameter_declarations.len()
            + self.struct_or_union_specifiers.len()
            + self.struct_declarations.len()
            + self.struct_declarators.len()
            + self.enum_specifiers.len()
            + self.enumerators.len()
    }
}

pub(crate) struct DeclarationView<'a> {
    tree:        &'a SyntaxTree,
    declaration: &'a Declaration,
}

impl<'a> DeclarationView<'a> {
    pub(crate) fn syntax(&self) -> &'a Declaration {
        self.declaration
    }

    pub(crate) fn init_declarators(&self) -> &'a [InitDeclarator] {
        self.tree
            .init_declarators(self.declaration.init_declarators)
    }
}

pub(crate) struct InitializerView<'a> {
    tree:        &'a SyntaxTree,
    initializer: &'a Initializer,
}

impl<'a> InitializerView<'a> {
    pub(crate) fn syntax(&self) -> &'a Initializer {
        self.initializer
    }

    pub(crate) fn elements(&self) -> Option<&'a [InitializerElement]> {
        let InitializerType::InitializerList(elements) = self.initializer.kind else {
            return None;
        };
        Some(self.tree.initializer_elements(elements))
    }
}
