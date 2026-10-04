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
        Designation,
        Designator,
        DirectDeclarator,
        EnumSpecifier,
        Enumerator,
        InitDeclarator,
        Initializer,
        InitializerElement,
        ParameterDeclaration,
        ParenthesizedDeclarator,
        StructDeclaration,
        StructDeclarator,
        StructOrUnionSpecifier,
        TypeName,
        TypeQualifiers,
    },
    syntax::{
        BlockItem,
        Expression,
        ExternalDeclaration,
        FunctionDefinition,
        FunctionDefinitionIndex,
        Identifier,
        Statement,
        StatementIndex,
        StatementType,
        SyntaxList,
    },
};
use crate::util::{
    arena::Arena,
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
    Statement,
    FunctionDefinition,
    BlockItem;
}

// SAFETY: `Erased` is the same reference type with `'tu` replaced by
// `'static`.
unsafe impl<'tu> StoredNode<'tu> for &'tu Declaration<'tu> {
    type Erased = &'static Declaration<'static>;
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
    expressions:          Vec<&'tu Expression<'tu>>,
    type_names:           Vec<&'tu TypeName<'tu>>,
    initializers:         Vec<&'tu Initializer<'tu>>,
    initializer_elements: Vec<&'tu InitializerElement<'tu>>,
    designations:         Vec<&'tu Designation<'tu>>,
    designators:          Vec<&'tu Designator<'tu>>,
    declarations:         Vec<&'tu Declaration<'tu>>,
    init_declarators:     Vec<&'tu InitDeclarator<'tu>>,
    direct_declarators:   Vec<&'tu DirectDeclarator<'tu>>,
    parenthesized:        Vec<&'tu ParenthesizedDeclarator<'tu>>,
    parameters:           Vec<&'tu ParameterDeclaration<'tu>>,
    struct_or_unions:     Vec<&'tu StructOrUnionSpecifier<'tu>>,
    struct_declarations:  Vec<&'tu StructDeclaration<'tu>>,
    struct_declarators:   Vec<&'tu StructDeclarator<'tu>>,
    enum_specifiers:      Vec<&'tu EnumSpecifier<'tu>>,
    enumerators:          Vec<&'tu Enumerator<'tu>>,
    type_qualifiers:      Vec<&'tu TypeQualifiers>,
    identifiers:          Vec<&'tu Identifier>,
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
    TypeName => type_names,
    Initializer => initializers,
    InitializerElement => initializer_elements,
    Designation => designations,
    Designator => designators,
    Declaration => declarations,
    InitDeclarator => init_declarators,
    DirectDeclarator => direct_declarators,
    ParenthesizedDeclarator => parenthesized,
    ParameterDeclaration => parameters,
    StructOrUnionSpecifier => struct_or_unions,
    StructDeclaration => struct_declarations,
    StructDeclarator => struct_declarators,
    EnumSpecifier => enum_specifiers,
    Enumerator => enumerators,
}

macro_rules! plain_tree_nodes {
    ($($node:ident => $field:ident),* $(,)?) => {$(
        #[cfg_attr(
            not(test),
            expect(single_use_lifetimes, reason = "Only the test log names the lifetime.")
        )]
        impl<'tu> TreeNode<'tu> for $node {
            #[cfg(test)]
            fn log(log: &mut SyntaxLog<'tu>, node: &'tu Self) {
                log.$field.push(node);
            }
        }

        #[cfg(test)]
        impl<'tu> CountedNode<'tu> for $node {
            fn nodes<'a>(store: &'a SyntaxStore<'tu>) -> Vec<&'a Self> {
                store.log.$field.iter().map(|node| -> &'a Self { node }).collect()
            }
        }
    )*};
}

plain_tree_nodes! {
    TypeQualifiers => type_qualifiers,
    Identifier => identifiers,
}

/// Call arguments are lists of expression references.
impl<'tu> TreeNode<'tu> for &'tu Expression<'tu> {
    #[cfg(test)]
    fn log(_: &mut SyntaxLog<'tu>, _: &'tu Self) {}
}

/// Old-style declaration lists are lists of declaration references.
impl<'tu> TreeNode<'tu> for &'tu Declaration<'tu> {
    #[cfg(test)]
    fn log(_: &mut SyntaxLog<'tu>, _: &'tu Self) {}
}

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
    StatementIndex => Statement,
    FunctionDefinitionIndex => FunctionDefinition,
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
            .field("statements", &nodes::<Statement<'tu>>(self))
            .field("block_items", &nodes::<BlockItem<'tu>>(self))
            .field(
                "function_definitions",
                &nodes::<FunctionDefinition<'tu>>(self),
            )
            .field("declaration_lists", &nodes::<&'tu Declaration<'tu>>(self))
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
    pub(super) fn new(store: SyntaxStore<'tu>, roots: &[ExternalDeclaration<'tu>]) -> Self {
        let tree = Self { store };
        tree.validate(roots);
        tree
    }

    fn validate(&self, roots: &[ExternalDeclaration<'tu>]) {
        for root in roots {
            if let ExternalDeclaration::FunctionDefinition(index)
            | ExternalDeclaration::RecoveredFunctionDefinition(index) = *root
            {
                let _ = self.function_definition(index);
            }
        }
        for definition in self.store.stored::<FunctionDefinition<'tu>>() {
            let _ = self.declaration_indices(definition.declaration_list);
            let _ = self.statement(definition.body);
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
                | StatementType::DoWhile { body_statement, .. }
                | StatementType::For { body_statement, .. }
                | StatementType::Label(_, body_statement)
                | StatementType::Default(body_statement)
                | StatementType::Case(_, body_statement) => {
                    let _ = self.statement(body_statement);
                },
                | StatementType::Expression(_)
                | StatementType::Return(_)
                | StatementType::Break
                | StatementType::Continue
                | StatementType::Goto(_)
                | StatementType::Null => {},
            }
        }
        for item in self.store.stored::<BlockItem<'tu>>() {
            if let BlockItem::Statement(statement) = *item {
                let _ = self.statement(statement);
            }
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

    pub(crate) fn block_items(&self, range: SyntaxList<BlockItem<'tu>>) -> &[BlockItem<'tu>] {
        &self.store[range]
    }

    pub(crate) fn declaration_indices(
        &self,
        range: SyntaxList<&'tu Declaration<'tu>>,
    ) -> &[&'tu Declaration<'tu>] {
        &self.store[range]
    }

    pub(crate) fn raw_debug(&self) -> impl Debug + '_ {
        &self.store
    }
}
