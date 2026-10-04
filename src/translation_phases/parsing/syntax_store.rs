//! Arena storage for syntax nodes and the validated [`SyntaxTree`] view.
//!
//! The syntax tree is moving, one domain at a time, from typed handles into
//! references allocated in the translation-unit arena. Until the last domain
//! moves, the store keeps the remaining nodes in its handle arena, and
//! [`StoredNode`] lets nodes that already hold references live there.

use std::{
    fmt::{
        Debug,
        Formatter,
        Result as FmtResult,
    },
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Index,
};

use super::{
    declaration_syntax::{
        Declaration,
        DeclarationSpecifiers,
        Declarator,
        Designation,
        Designator,
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
        DeclarationIndex,
        DesignationIndex,
        EnumSpecifierIndex,
        Expression,
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
use crate::util::{
    arena::{
        Arena,
        ArenaCheckpoint,
    },
    bump::ArenaVec,
};

/// A node kind the store keeps in its handle arena.
///
/// # Safety
///
/// `Erased` must be `Self` with every lifetime replaced by `'static`, so the
/// two types share one layout and the arena can key its type check on
/// `Erased`.
pub(super) unsafe trait StoredNode<'tu>: Sized {
    type Erased: 'static;
}

macro_rules! stored_nodes {
    ($($node:ident),* ; $($plain:ident),* $(,)?) => {
        $(
            // SAFETY: `Erased` is the same type with `'tu` replaced by
            // `'static`.
            unsafe impl<'tu> StoredNode<'tu> for $node<'tu> {
                type Erased = $node<'static>;
            }
        )*
        $(
            // SAFETY: the type has no lifetime parameter.
            unsafe impl StoredNode<'_> for $plain {
                type Erased = $plain;
            }
        )*
    };
}

stored_nodes! {
    TypeName,
    ParenthesizedDeclarator,
    Declaration,
    InitDeclarator,
    Initializer,
    Designation,
    Designator,
    Statement,
    FunctionDefinition,
    StructOrUnionSpecifier,
    StructDeclaration,
    StructDeclarator,
    EnumSpecifier,
    Enumerator,
    DirectDeclarator,
    ParameterDeclaration;
    InitializerElement,
    BlockItem,
    DeclarationIndex,
    TypeQualifiers,
    Identifier,
}

/// Moves `node` into its lifetime-erased type.
fn erase<'tu, T: StoredNode<'tu>>(node: T) -> T::Erased {
    const {
        assert!(
            size_of::<T>() == size_of::<T::Erased>(),
            "a node and its erased type share one layout"
        );
    }
    let node = ManuallyDrop::new(node);
    // SAFETY: `StoredNode` guarantees that `T::Erased` is `T` with its
    // lifetimes replaced, so it has the same layout. The store hands the
    // value out again only as `T`, borrowed for no longer than `'tu`.
    unsafe { std::mem::transmute_copy::<T, T::Erased>(&node) }
}

/// Owns the nodes of the syntax domains that still use typed handles.
///
/// Nodes of every kind share one chunked [`Arena`], which grows a block at a
/// time instead of doubling and copying per-kind vectors. AST nodes refer to
/// these nodes through 4-byte typed handles and [`SyntaxList`] runs. Only
/// [`Parser::push_syntax`](super::Parser::push_syntax),
/// [`Parser::append_syntax`](super::Parser::append_syntax), and the
/// translation-unit arena allocations recorded through [`Self::record`] add
/// nodes, so the parser's running node total stays exact.
///
/// C99: the stored language syntax spans expressions through external
/// definitions, §6.5-§6.9, pp. 67-144; PDF pp. 79-156. Arena storage is an
/// implementation strategy, not a normative C concept.
#[derive(Default)]
pub(super) struct SyntaxStore<'tu> {
    arena:      Arena,
    /// Nodes allocated in the translation-unit arena instead.
    tree_nodes: usize,
    #[cfg(test)]
    log:        SyntaxLog<'tu>,
    _tree:      PhantomData<&'tu ()>,
}

/// Every node allocated in the translation-unit arena, by kind, so tests can
/// count and visit them in allocation order.
#[cfg(test)]
#[derive(Default)]
pub(super) struct SyntaxLog<'tu> {
    expressions: Vec<&'tu Expression<'tu>>,
}

/// A node kind that tests can count and visit.
#[cfg(test)]
pub(super) trait CountedNode<'tu>: Sized + 'tu {
    fn nodes<'a>(store: &'a SyntaxStore<'tu>) -> Vec<&'a Self>;
}

#[cfg(test)]
impl<'tu, T: StoredNode<'tu> + 'tu> CountedNode<'tu> for T {
    fn nodes<'a>(store: &'a SyntaxStore<'tu>) -> Vec<&'a Self> {
        store.stored::<T>().collect()
    }
}

/// A node kind allocated in the translation-unit arena.
pub(super) trait TreeNode<'tu>: Sized + 'tu {
    /// Records `node` in the test log.
    #[cfg(test)]
    fn log(log: &mut SyntaxLog<'tu>, node: &'tu Self);
}

macro_rules! tree_nodes {
    ($($node:ident => $field:ident),* $(,)?) => {$(
        impl<'tu> TreeNode<'tu> for $node<'tu> {
            #[cfg(test)]
            fn log(log: &mut SyntaxLog<'tu>, node: &'tu Self) {
                log.$field.push(node);
            }
        }

        #[cfg(test)]
        impl<'tu> CountedNode<'tu> for $node<'tu> {
            fn nodes<'a>(store: &'a SyntaxStore<'tu>) -> Vec<&'a Self> {
                store.log.$field.iter().map(|node| -> &'a Self { node }).collect()
            }
        }
    )*};
}

tree_nodes! {
    Expression => expressions,
}

/// Call arguments are lists of expression references.
impl<'tu> TreeNode<'tu> for &'tu Expression<'tu> {
    #[cfg(test)]
    fn log(_: &mut SyntaxLog<'tu>, _: &'tu Self) {}
}

#[derive(Clone, Copy)]
pub(super) struct SyntaxStoreCheckpoint(ArenaCheckpoint);

/// Typed handles resolve through the store like slice indices.
macro_rules! syntax_handles {
    ($($handle:ty => $node:ident),* $(,)?) => {$(
        impl<'tu> Index<$handle> for SyntaxStore<'tu> {
            type Output = $node<'tu>;

            fn index(&self, handle: $handle) -> &$node<'tu> {
                self.arena.get::<$node<'static>>(handle.0)
            }
        }

        impl<'tu> Index<&$handle> for SyntaxStore<'tu> {
            type Output = $node<'tu>;

            fn index(&self, handle: &$handle) -> &$node<'tu> {
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
    StatementIndex => Statement,
    FunctionDefinitionIndex => FunctionDefinition,
    StructOrUnionSpecifierIndex => StructOrUnionSpecifier,
    EnumSpecifierIndex => EnumSpecifier,
}

impl<'tu, T: StoredNode<'tu> + 'tu> Index<SyntaxList<T>> for SyntaxStore<'tu> {
    type Output = [T];

    fn index(&self, index: SyntaxList<T>) -> &[T] {
        let erased = self.arena.slice::<T::Erased>(index.run());
        // SAFETY: `StoredNode` guarantees that `T::Erased` and `T` share one
        // layout; the values were stored as `T` with lifetime `'tu`, which
        // outlives this borrow of the store.
        unsafe { std::slice::from_raw_parts(erased.as_ptr().cast::<T>(), erased.len()) }
    }
}

/// Test-only identity indexing, so tests read a node the same way whether
/// its domain still uses handles or already uses references.
#[cfg(test)]
impl<'tu, T: TreeNode<'tu>> Index<&'tu T> for SyntaxStore<'tu> {
    type Output = T;

    fn index(&self, index: &'tu T) -> &T {
        index
    }
}

#[cfg(test)]
impl<'tu, T: TreeNode<'tu>> Index<&'tu [T]> for SyntaxStore<'tu> {
    type Output = [T];

    fn index(&self, index: &'tu [T]) -> &[T] {
        index
    }
}

impl<'tu> SyntaxStore<'tu> {
    /// Stores one node and returns its raw handle.
    pub(super) fn push<T: StoredNode<'tu>>(&mut self, node: T) -> u32 {
        self.arena.push(erase(node))
    }

    /// Moves `nodes` into one contiguous list.
    pub(super) fn append<'a, T: StoredNode<'tu>>(
        &mut self,
        nodes: &mut ArenaVec<'a, T>,
    ) -> SyntaxList<T> {
        // SAFETY: `StoredNode` guarantees that `T::Erased` is `T` with its
        // lifetimes replaced, so both vectors share one layout. `extend`
        // moves every value out, so no erased value stays in `nodes`.
        let erased = unsafe { &mut *std::ptr::from_mut(nodes).cast::<ArenaVec<'a, T::Erased>>() };
        SyntaxList::new(self.arena.extend(erased))
    }

    /// Counts a node allocated in the translation-unit arena.
    pub(super) fn record<T: TreeNode<'tu>>(&mut self, node: &'tu T) {
        self.tree_nodes += 1;
        #[cfg(test)]
        T::log(&mut self.log, node);
        #[cfg(not(test))]
        let _ = node;
    }

    /// Counts a list allocated in the translation-unit arena.
    pub(super) fn record_list<T: TreeNode<'tu>>(&mut self, nodes: &'tu [T]) {
        self.tree_nodes += nodes.len();
        #[cfg(test)]
        for node in nodes {
            T::log(&mut self.log, node);
        }
    }

    /// Puts `new` where the test log holds `old`, which it replaces in the
    /// tree. The node count does not change.
    #[cfg(test)]
    pub(super) fn replace_expression(
        &mut self,
        old: &'tu Expression<'tu>,
        new: &'tu Expression<'tu>,
    ) {
        if let Some(slot) = self
            .log
            .expressions
            .iter_mut()
            .rev()
            .find(|logged| std::ptr::eq(**logged, old))
        {
            *slot = new;
        }
    }

    /// Marks the store so [`Self::restore`] can discard later nodes.
    pub(super) fn checkpoint(&mut self) -> SyntaxStoreCheckpoint {
        SyntaxStoreCheckpoint(self.arena.checkpoint())
    }

    /// Discards every handle-arena node added since the most recent
    /// checkpoint. Nodes in the translation-unit arena stay, and still
    /// count.
    pub(super) fn restore(&mut self, checkpoint: SyntaxStoreCheckpoint) {
        self.arena.restore(checkpoint.0);
    }

    /// Nodes of every kind.
    pub(super) fn node_count(&self) -> usize {
        self.arena.len() + self.tree_nodes
    }

    /// Nodes of kind `T`.
    #[cfg(test)]
    pub(super) fn count<T: CountedNode<'tu>>(&self) -> usize {
        T::nodes(self).len()
    }

    /// Every node of kind `T`, in the order they were stored.
    #[cfg(test)]
    pub(super) fn iter<T: CountedNode<'tu>>(&self) -> std::vec::IntoIter<&T> {
        T::nodes(self).into_iter()
    }

    /// The `n`th stored node of kind `T`.
    #[cfg(test)]
    pub(super) fn nth<T: CountedNode<'tu>>(&self, n: usize) -> &T {
        T::nodes(self)
            .into_iter()
            .nth(n)
            .expect("the syntax store holds that many nodes of this kind")
    }

    /// Every node of kind `T` in the handle arena, in the order they were
    /// stored.
    fn stored<T: StoredNode<'tu> + 'tu>(&self) -> impl Iterator<Item = &T> {
        self.arena.iter::<T::Erased>().map(|node| {
            // SAFETY: as in `Index<SyntaxList<T>>`.
            unsafe { &*std::ptr::from_ref(node).cast::<T>() }
        })
    }
}

/// Lists every node of one kind for [`SyntaxStore`]'s debug view.
struct DebugNodes<'a, 'tu, T>(&'a SyntaxStore<'tu>, PhantomData<T>);

impl<'tu, T: Debug + StoredNode<'tu> + 'tu> Debug for DebugNodes<'_, 'tu, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_list().entries(self.0.stored::<T>()).finish()
    }
}

impl<'tu> Debug for SyntaxStore<'tu> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        fn nodes<'a, 'tu, T>(store: &'a SyntaxStore<'tu>) -> DebugNodes<'a, 'tu, T> {
            DebugNodes(store, PhantomData)
        }
        f.debug_struct("SyntaxStore")
            .field("type_names", &nodes::<TypeName<'tu>>(self))
            .field("declarations", &nodes::<Declaration<'tu>>(self))
            .field("init_declarators", &nodes::<InitDeclarator<'tu>>(self))
            .field("initializers", &nodes::<Initializer<'tu>>(self))
            .field("initializer_elements", &nodes::<InitializerElement>(self))
            .field("designations", &nodes::<Designation<'tu>>(self))
            .field("designators", &nodes::<Designator<'tu>>(self))
            .field("statements", &nodes::<Statement<'tu>>(self))
            .field("block_items", &nodes::<BlockItem>(self))
            .field(
                "function_definitions",
                &nodes::<FunctionDefinition<'tu>>(self),
            )
            .field("declaration_indices", &nodes::<DeclarationIndex>(self))
            .field("type_qualifiers", &nodes::<TypeQualifiers>(self))
            .field("direct_declarators", &nodes::<DirectDeclarator<'tu>>(self))
            .field(
                "parenthesized_declarators",
                &nodes::<ParenthesizedDeclarator<'tu>>(self),
            )
            .field("identifiers", &nodes::<Identifier>(self))
            .field(
                "parameter_declarations",
                &nodes::<ParameterDeclaration<'tu>>(self),
            )
            .field(
                "struct_or_union_specifiers",
                &nodes::<StructOrUnionSpecifier<'tu>>(self),
            )
            .field(
                "struct_declarations",
                &nodes::<StructDeclaration<'tu>>(self),
            )
            .field("struct_declarators", &nodes::<StructDeclarator<'tu>>(self))
            .field("enum_specifiers", &nodes::<EnumSpecifier<'tu>>(self))
            .field("enumerators", &nodes::<Enumerator<'tu>>(self))
            .finish()
    }
}

/// Validated, read-only access to arena-backed C syntax.
///
/// Typed handles cannot be constructed outside this module. List-bearing
/// views resolve their ranges here so callers never coordinate raw arena
/// positions themselves. The arena checks every handle as it resolves it.
#[derive(Debug)]
pub(crate) struct SyntaxTree<'tu> {
    store: SyntaxStore<'tu>,
}

impl<'tu> SyntaxTree<'tu> {
    pub(super) fn new(store: SyntaxStore<'tu>, roots: &[ExternalDeclaration]) -> Self {
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
        for declaration in self.store.stored::<Declaration<'tu>>() {
            let _ = self.init_declarators(declaration.init_declarators);
            self.validate_specifiers(declaration.declaration_specifiers);
        }
        for definition in self.store.stored::<FunctionDefinition<'tu>>() {
            self.validate_declarator(definition.declarator);
            for declaration in self.declaration_indices(definition.declaration_list) {
                let _ = self.declaration(*declaration);
            }
            let _ = self.statement(definition.body);
            self.validate_specifiers(definition.declaration_specifiers);
        }
        for init in self.store.stored::<InitDeclarator<'tu>>() {
            self.validate_declarator(init.declarator);
            if let Some(initializer) = init.initializer {
                let _ = self.initializer(initializer);
            }
        }
        for initializer in self.store.stored::<Initializer<'tu>>() {
            match initializer.kind {
                | InitializerType::AssignmentExpression(_) => {},
                | InitializerType::InitializerList(elements) => {
                    let _ = self.initializer_elements(elements);
                },
            }
        }
        for element in self.store.stored::<InitializerElement>() {
            if let Some(designation) = element.designation {
                let _ = self.designation(designation);
            }
            let _ = self.initializer(element.initializer);
        }
        for designation in self.store.stored::<Designation<'tu>>() {
            let _ = self.designators(designation.designators);
        }
        for statement in self.store.stored::<Statement<'tu>>() {
            match statement.kind {
                | StatementType::Compound { items } => {
                    let _ = self.block_items(items);
                },
                | StatementType::If {
                    then_statement,
                    else_statement,
                    ..
                } => {
                    let _ = self.statement(then_statement);
                    if let Some(statement) = else_statement {
                        let _ = self.statement(statement);
                    }
                },
                | StatementType::Switch { body_statement, .. }
                | StatementType::While { body_statement, .. }
                | StatementType::DoWhile { body_statement, .. } => {
                    let _ = self.statement(body_statement);
                },
                | StatementType::For {
                    initializer,
                    body_statement,
                    ..
                } => {
                    if let Some(ForInitializer::Declaration(declaration)) = initializer {
                        let _ = self.declaration(declaration);
                    }
                    let _ = self.statement(body_statement);
                },
                | StatementType::Label(_, child)
                | StatementType::Default(child)
                | StatementType::Case(_, child) => {
                    let _ = self.statement(child);
                },
                | StatementType::Expression(_)
                | StatementType::Return(_)
                | StatementType::Break
                | StatementType::Continue
                | StatementType::Goto(_)
                | StatementType::Null => {},
            }
        }
        for item in self.store.stored::<BlockItem>() {
            match *item {
                | BlockItem::Declaration(declaration) => {
                    let _ = self.declaration(declaration);
                },
                | BlockItem::Statement(statement) => {
                    let _ = self.statement(statement);
                },
            }
        }
        for declaration in self.store.stored::<DeclarationIndex>() {
            let _ = self.declaration(*declaration);
        }
        for type_name in self.store.stored::<TypeName<'tu>>() {
            self.validate_specifiers(type_name.declaration_specifiers);
            if let Some(declarator) = type_name.declarator {
                self.validate_declarator(declarator);
            }
        }
        for direct in self.store.stored::<DirectDeclarator<'tu>>() {
            match *direct {
                | DirectDeclarator::Parenthesized(index) => {
                    self.validate_declarator(self.store[index].declarator);
                },
                | DirectDeclarator::KAndRStyleFunction { parameters } => {
                    let _ = self.identifiers(parameters);
                },
                | DirectDeclarator::Function { parameter_list, .. } => {
                    let _ = self.parameter_declarations(parameter_list);
                },
                | DirectDeclarator::Identifier(_) | DirectDeclarator::Array { .. } => {},
            }
        }
        for parameter in self.store.stored::<ParameterDeclaration<'tu>>() {
            self.validate_specifiers(parameter.declaration_specifiers);
            if let Some(declarator) = parameter.declarator {
                self.validate_declarator(declarator);
            }
        }
        for specifier in self.store.stored::<StructOrUnionSpecifier<'tu>>() {
            if let Some(declarations) = specifier.struct_declaration_list {
                let _ = self.struct_declarations(declarations);
            }
        }
        for declaration in self.store.stored::<StructDeclaration<'tu>>() {
            self.validate_type_specifiers(declaration.type_specifiers);
            let _ = self.struct_declarators(declaration.struct_declarator_list);
        }
        for declarator in self.store.stored::<StructDeclarator<'tu>>() {
            if let Some(syntax) = declarator.declarator {
                self.validate_declarator(syntax);
            }
        }
        for specifier in self.store.stored::<EnumSpecifier<'tu>>() {
            if let Some(enumerators) = specifier.enumeration_list {
                let _ = self.enumerators(enumerators);
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

    fn validate_declarator(&self, declarator: Declarator<'tu>) {
        let _ = self.pointer_qualifiers(declarator.pointer.type_qualifiers_list);
        let _ = self.direct_declarators(declarator.kind);
    }

    pub(crate) fn declaration(&self, index: DeclarationIndex) -> DeclarationView<'_, 'tu> {
        DeclarationView {
            tree:        self,
            declaration: &self.store[index],
        }
    }

    pub(crate) fn function_definition(
        &self,
        index: FunctionDefinitionIndex,
    ) -> &FunctionDefinition<'tu> {
        &self.store[index]
    }

    pub(crate) fn statement(&self, index: StatementIndex) -> &Statement<'tu> {
        &self.store[index]
    }

    pub(crate) fn type_name(&self, index: TypeNameIndex) -> &TypeName<'tu> {
        &self.store[index]
    }

    pub(crate) fn initializer(&self, index: InitializerIndex) -> InitializerView<'_, 'tu> {
        InitializerView {
            tree:        self,
            initializer: &self.store[index],
        }
    }

    pub(crate) fn designation(&self, index: DesignationIndex) -> &Designation<'tu> {
        &self.store[index]
    }

    pub(crate) fn struct_or_union_specifier(
        &self,
        index: StructOrUnionSpecifierIndex,
    ) -> &StructOrUnionSpecifier<'tu> {
        &self.store[index]
    }

    pub(crate) fn enum_specifier(&self, index: EnumSpecifierIndex) -> &EnumSpecifier<'tu> {
        &self.store[index]
    }

    pub(crate) fn init_declarators(
        &self,
        range: SyntaxList<InitDeclarator<'tu>>,
    ) -> &[InitDeclarator<'tu>] {
        &self.store[range]
    }

    pub(crate) fn initializer_elements(
        &self,
        range: SyntaxList<InitializerElement>,
    ) -> &[InitializerElement] {
        &self.store[range]
    }

    pub(crate) fn designators(&self, range: SyntaxList<Designator<'tu>>) -> &[Designator<'tu>] {
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
    ) -> &ParenthesizedDeclarator<'tu> {
        &self.store[index]
    }

    pub(crate) fn direct_declarators(
        &self,
        range: SyntaxList<DirectDeclarator<'tu>>,
    ) -> &[DirectDeclarator<'tu>] {
        &self.store[range]
    }

    pub(crate) fn identifiers(&self, range: SyntaxList<Identifier>) -> &[Identifier] {
        &self.store[range]
    }

    pub(crate) fn parameter_declarations(
        &self,
        range: SyntaxList<ParameterDeclaration<'tu>>,
    ) -> &[ParameterDeclaration<'tu>] {
        &self.store[range]
    }

    pub(crate) fn struct_declarations(
        &self,
        range: SyntaxList<StructDeclaration<'tu>>,
    ) -> &[StructDeclaration<'tu>] {
        &self.store[range]
    }

    pub(crate) fn struct_declarators(
        &self,
        range: SyntaxList<StructDeclarator<'tu>>,
    ) -> &[StructDeclarator<'tu>] {
        &self.store[range]
    }

    pub(crate) fn enumerators(&self, range: SyntaxList<Enumerator<'tu>>) -> &[Enumerator<'tu>] {
        &self.store[range]
    }

    pub(crate) fn raw_debug(&self) -> impl Debug + '_ {
        &self.store
    }
}

pub(crate) struct DeclarationView<'a, 'tu> {
    tree:        &'a SyntaxTree<'tu>,
    declaration: &'a Declaration<'tu>,
}

impl<'a, 'tu> DeclarationView<'a, 'tu> {
    pub(crate) fn syntax(&self) -> &'a Declaration<'tu> {
        self.declaration
    }

    pub(crate) fn init_declarators(&self) -> &'a [InitDeclarator<'tu>] {
        self.tree
            .init_declarators(self.declaration.init_declarators)
    }
}

pub(crate) struct InitializerView<'a, 'tu> {
    tree:        &'a SyntaxTree<'tu>,
    initializer: &'a Initializer<'tu>,
}

impl<'a, 'tu> InitializerView<'a, 'tu> {
    pub(crate) fn syntax(&self) -> &'a Initializer<'tu> {
        self.initializer
    }

    pub(crate) fn elements(&self) -> Option<&'a [InitializerElement]> {
        let InitializerType::InitializerList(elements) = self.initializer.kind else {
            return None;
        };
        Some(self.tree.initializer_elements(elements))
    }
}
