//! Arena storage for syntax nodes and the validated [`SyntaxTree`] view.

use std::{
    fmt::{
        Debug,
        Formatter,
        Result as FmtResult,
    },
    marker::PhantomData,
    ops::{
        Index,
        IndexMut,
    },
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
        ParenthesizedDeclarator,
        StructDeclaration,
        StructDeclarator,
        StructOrUnionSpecifier,
        TypeName,
        TypeQualifiers,
        TypeSpecifiers,
    },
    syntax::{
        BlockItem,
        ConstantExpressionIndex,
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
        ParenthesizedDeclaratorIndex,
        Statement,
        StatementIndex,
        StatementType,
        StructOrUnionSpecifierIndex,
        SyntaxList,
        TypeNameIndex,
    },
};
use crate::util::arena::{
    Arena,
    ArenaCheckpoint,
};

/// Owns the storage of every syntax domain constructed by parser frames.
///
/// Nodes of every kind share one chunked [`Arena`], which grows a block at a
/// time instead of doubling and copying per-kind vectors. AST nodes refer to
/// each other through 4-byte typed handles and [`SyntaxList`] runs, keeping
/// nested syntax compact and avoiding recursive ownership. Only
/// [`Parser::push_syntax`](super::Parser::push_syntax) and
/// [`Parser::append_syntax`](super::Parser::append_syntax) add nodes, so the
/// parser's running node total stays exact.
///
/// C99: the stored language syntax spans expressions through external
/// definitions, §6.5-§6.9, pp. 67-144; PDF pp. 79-156. Arena storage is an
/// implementation strategy, not a normative C concept.
#[derive(Default)]
pub(super) struct SyntaxStore {
    arena: Arena,
}

#[derive(Clone, Copy)]
pub(super) struct SyntaxStoreCheckpoint(ArenaCheckpoint);

/// Typed handles resolve through the store like slice indices.
macro_rules! syntax_handles {
    ($($handle:ty => $node:ty),* $(,)?) => {$(
        impl Index<$handle> for SyntaxStore {
            type Output = $node;

            fn index(&self, handle: $handle) -> &$node {
                self.arena.get(handle.0)
            }
        }

        impl IndexMut<$handle> for SyntaxStore {
            fn index_mut(&mut self, handle: $handle) -> &mut $node {
                self.arena.get_mut(handle.0)
            }
        }

        impl Index<&$handle> for SyntaxStore {
            type Output = $node;

            fn index(&self, handle: &$handle) -> &$node {
                &self[*handle]
            }
        }
    )*};
}

syntax_handles! {
    TypeNameIndex => TypeName,
    ParenthesizedDeclaratorIndex => ParenthesizedDeclarator,
    DeclarationIndex => Declaration,
    InitializerIndex => Initializer,
    DesignationIndex => Designation,
    ExpressionIndex => Expression,
    ConstantExpressionIndex => Expression,
    StatementIndex => Statement,
    FunctionDefinitionIndex => FunctionDefinition,
    StructOrUnionSpecifierIndex => StructOrUnionSpecifier,
    EnumSpecifierIndex => EnumSpecifier,
}

impl<T: 'static> Index<SyntaxList<T>> for SyntaxStore {
    type Output = [T];

    fn index(&self, index: SyntaxList<T>) -> &[T] {
        self.arena.slice(index.run())
    }
}

impl<T: 'static> IndexMut<SyntaxList<T>> for SyntaxStore {
    fn index_mut(&mut self, index: SyntaxList<T>) -> &mut [T] {
        self.arena.slice_mut(index.run())
    }
}

impl SyntaxStore {
    /// Stores one node and returns its raw handle.
    pub(super) fn push<T: 'static>(&mut self, node: T) -> u32 {
        self.arena.push(node)
    }

    /// Moves `nodes` into one contiguous list.
    pub(super) fn append<T: 'static>(&mut self, nodes: &mut Vec<T>) -> SyntaxList<T> {
        SyntaxList::new(self.arena.extend(nodes))
    }

    /// Marks the store so [`Self::restore`] can discard later nodes.
    pub(super) fn checkpoint(&mut self) -> SyntaxStoreCheckpoint {
        SyntaxStoreCheckpoint(self.arena.checkpoint())
    }

    /// Discards every node added since the most recent checkpoint.
    pub(super) fn restore(&mut self, checkpoint: SyntaxStoreCheckpoint) {
        self.arena.restore(checkpoint.0);
    }

    /// Nodes of every kind.
    pub(super) fn node_count(&self) -> usize {
        self.arena.len()
    }

    /// Nodes of kind `T`.
    #[cfg(test)]
    pub(super) fn count<T: 'static>(&self) -> usize {
        self.arena.count::<T>()
    }

    /// Every node of kind `T`, in the order they were stored.
    pub(super) fn iter<T: 'static>(&self) -> impl Iterator<Item = &T> {
        self.arena.iter::<T>()
    }

    /// The `n`th stored node of kind `T`.
    #[cfg(test)]
    pub(super) fn nth<T: 'static>(&self, n: usize) -> &T {
        self.iter::<T>()
            .nth(n)
            .expect("the syntax store holds that many nodes of this kind")
    }
}

/// Lists every node of one kind for [`SyntaxStore`]'s debug view.
struct DebugNodes<'a, T>(&'a SyntaxStore, PhantomData<T>);

impl<T: Debug + 'static> Debug for DebugNodes<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_list().entries(self.0.iter::<T>()).finish()
    }
}

impl Debug for SyntaxStore {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        fn nodes<T>(store: &SyntaxStore) -> DebugNodes<'_, T> {
            DebugNodes(store, PhantomData)
        }
        f.debug_struct("SyntaxStore")
            .field("type_names", &nodes::<TypeName>(self))
            .field("declarations", &nodes::<Declaration>(self))
            .field("init_declarators", &nodes::<InitDeclarator>(self))
            .field("initializers", &nodes::<Initializer>(self))
            .field("initializer_elements", &nodes::<InitializerElement>(self))
            .field("designations", &nodes::<Designation>(self))
            .field("designators", &nodes::<Designator>(self))
            .field("expressions", &nodes::<Expression>(self))
            .field("expression_indices", &nodes::<ExpressionIndex>(self))
            .field("statements", &nodes::<Statement>(self))
            .field("block_items", &nodes::<BlockItem>(self))
            .field("function_definitions", &nodes::<FunctionDefinition>(self))
            .field("declaration_indices", &nodes::<DeclarationIndex>(self))
            .field("type_qualifiers", &nodes::<TypeQualifiers>(self))
            .field("direct_declarators", &nodes::<DirectDeclarator>(self))
            .field(
                "parenthesized_declarators",
                &nodes::<ParenthesizedDeclarator>(self),
            )
            .field("identifiers", &nodes::<Identifier>(self))
            .field(
                "parameter_declarations",
                &nodes::<ParameterDeclaration>(self),
            )
            .field(
                "struct_or_union_specifiers",
                &nodes::<StructOrUnionSpecifier>(self),
            )
            .field("struct_declarations", &nodes::<StructDeclaration>(self))
            .field("struct_declarators", &nodes::<StructDeclarator>(self))
            .field("enum_specifiers", &nodes::<EnumSpecifier>(self))
            .field("enumerators", &nodes::<Enumerator>(self))
            .finish()
    }
}

/// Validated, read-only access to arena-backed C syntax.
///
/// Typed handles cannot be constructed outside this module. List-bearing
/// views resolve their ranges here so callers never coordinate raw arena
/// positions themselves. The arena checks every handle as it resolves it.
#[derive(Debug)]
pub(crate) struct SyntaxTree {
    store: SyntaxStore,
}

impl SyntaxTree {
    pub(super) fn new(store: SyntaxStore, roots: &[ExternalDeclaration]) -> Self {
        let tree = Self { store };
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
        for declaration in self.store.iter::<Declaration>() {
            let _ = self.init_declarators(declaration.init_declarators);
            self.validate_specifiers(declaration.declaration_specifiers);
        }
        for definition in self.store.iter::<FunctionDefinition>() {
            self.validate_declarator(definition.declarator);
            for declaration in self.declaration_indices(definition.declaration_list) {
                let _ = self.declaration(*declaration);
            }
            let _ = self.statement(definition.body);
            self.validate_specifiers(definition.declaration_specifiers);
        }
        for init in self.store.iter::<InitDeclarator>() {
            self.validate_declarator(init.declarator);
            if let Some(initializer) = init.initializer {
                let _ = self.initializer(initializer);
            }
        }
        for expression in self.store.iter::<Expression>() {
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
        for expression in self.store.iter::<ExpressionIndex>() {
            let _ = self.expression(*expression);
        }
        for initializer in self.store.iter::<Initializer>() {
            match initializer.kind {
                | InitializerType::AssignmentExpression(expression) => {
                    let _ = self.expression(expression);
                },
                | InitializerType::InitializerList(elements) => {
                    let _ = self.initializer_elements(elements);
                },
            }
        }
        for element in self.store.iter::<InitializerElement>() {
            if let Some(designation) = element.designation {
                let _ = self.designation(designation);
            }
            let _ = self.initializer(element.initializer);
        }
        for designation in self.store.iter::<Designation>() {
            let _ = self.designators(designation.designators);
        }
        for designator in self.store.iter::<Designator>() {
            if let DesignatorType::Array(expression) = designator.kind {
                let _ = self.expression(expression.into());
            }
        }
        for statement in self.store.iter::<Statement>() {
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
        for item in self.store.iter::<BlockItem>() {
            match *item {
                | BlockItem::Declaration(declaration) => {
                    let _ = self.declaration(declaration);
                },
                | BlockItem::Statement(statement) => {
                    let _ = self.statement(statement);
                },
            }
        }
        for declaration in self.store.iter::<DeclarationIndex>() {
            let _ = self.declaration(*declaration);
        }
        for type_name in self.store.iter::<TypeName>() {
            self.validate_specifiers(type_name.declaration_specifiers);
            if let Some(declarator) = type_name.declarator {
                self.validate_declarator(declarator);
            }
        }
        for direct in self.store.iter::<DirectDeclarator>() {
            match *direct {
                | DirectDeclarator::Parenthesized(index) => {
                    self.validate_declarator(self.store[index].declarator);
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
        for parameter in self.store.iter::<ParameterDeclaration>() {
            self.validate_specifiers(parameter.declaration_specifiers);
            if let Some(declarator) = parameter.declarator {
                self.validate_declarator(declarator);
            }
        }
        for specifier in self.store.iter::<StructOrUnionSpecifier>() {
            if let Some(declarations) = specifier.struct_declaration_list {
                let _ = self.struct_declarations(declarations);
            }
        }
        for declaration in self.store.iter::<StructDeclaration>() {
            self.validate_type_specifiers(declaration.type_specifiers);
            let _ = self.struct_declarators(declaration.struct_declarator_list);
        }
        for declarator in self.store.iter::<StructDeclarator>() {
            if let Some(syntax) = declarator.declarator {
                self.validate_declarator(syntax);
            }
            if let Some(expression) = declarator.bitfield_width {
                let _ = self.expression(expression.into());
            }
        }
        for specifier in self.store.iter::<EnumSpecifier>() {
            if let Some(enumerators) = specifier.enumeration_list {
                let _ = self.enumerators(enumerators);
            }
        }
        for enumerator in self.store.iter::<Enumerator>() {
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

    pub(crate) fn declaration(&self, index: DeclarationIndex) -> DeclarationView<'_> {
        DeclarationView {
            tree:        self,
            declaration: &self.store[index],
        }
    }

    pub(crate) fn function_definition(
        &self,
        index: FunctionDefinitionIndex,
    ) -> &FunctionDefinition {
        &self.store[index]
    }

    pub(crate) fn statement(&self, index: StatementIndex) -> &Statement {
        &self.store[index]
    }

    pub(crate) fn expression(&self, index: ExpressionIndex) -> &Expression {
        &self.store[index]
    }

    pub(crate) fn type_name(&self, index: TypeNameIndex) -> &TypeName {
        &self.store[index]
    }

    pub(crate) fn initializer(&self, index: InitializerIndex) -> InitializerView<'_> {
        InitializerView {
            tree:        self,
            initializer: &self.store[index],
        }
    }

    pub(crate) fn designation(&self, index: DesignationIndex) -> &Designation {
        &self.store[index]
    }

    pub(crate) fn struct_or_union_specifier(
        &self,
        index: StructOrUnionSpecifierIndex,
    ) -> &StructOrUnionSpecifier {
        &self.store[index]
    }

    pub(crate) fn enum_specifier(&self, index: EnumSpecifierIndex) -> &EnumSpecifier {
        &self.store[index]
    }

    pub(crate) fn init_declarators(&self, range: SyntaxList<InitDeclarator>) -> &[InitDeclarator] {
        &self.store[range]
    }

    pub(crate) fn initializer_elements(
        &self,
        range: SyntaxList<InitializerElement>,
    ) -> &[InitializerElement] {
        &self.store[range]
    }

    pub(crate) fn designators(&self, range: SyntaxList<Designator>) -> &[Designator] {
        &self.store[range]
    }

    pub(crate) fn expression_indices(
        &self,
        range: SyntaxList<ExpressionIndex>,
    ) -> &[ExpressionIndex] {
        &self.store[range]
    }

    pub(crate) fn block_items(&self, range: SyntaxList<BlockItem>) -> &[BlockItem] {
        &self.store[range]
    }

    pub(crate) fn declaration_indices(
        &self,
        range: SyntaxList<DeclarationIndex>,
    ) -> &[DeclarationIndex] {
        &self.store[range]
    }

    pub(crate) fn pointer_qualifiers(
        &self,
        range: SyntaxList<TypeQualifiers>,
    ) -> &[TypeQualifiers] {
        &self.store[range]
    }

    pub(crate) fn parenthesized_declarator(
        &self,
        index: ParenthesizedDeclaratorIndex,
    ) -> &ParenthesizedDeclarator {
        &self.store[index]
    }

    pub(crate) fn direct_declarators(
        &self,
        range: SyntaxList<DirectDeclarator>,
    ) -> &[DirectDeclarator] {
        &self.store[range]
    }

    pub(crate) fn identifiers(&self, range: SyntaxList<Identifier>) -> &[Identifier] {
        &self.store[range]
    }

    pub(crate) fn parameter_declarations(
        &self,
        range: SyntaxList<ParameterDeclaration>,
    ) -> &[ParameterDeclaration] {
        &self.store[range]
    }

    pub(crate) fn struct_declarations(
        &self,
        range: SyntaxList<StructDeclaration>,
    ) -> &[StructDeclaration] {
        &self.store[range]
    }

    pub(crate) fn struct_declarators(
        &self,
        range: SyntaxList<StructDeclarator>,
    ) -> &[StructDeclarator] {
        &self.store[range]
    }

    pub(crate) fn enumerators(&self, range: SyntaxList<Enumerator>) -> &[Enumerator] {
        &self.store[range]
    }

    pub(crate) fn raw_debug(&self) -> impl Debug + '_ {
        &self.store
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
