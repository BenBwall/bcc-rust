//! Ordinary-identifier scopes used for typedef-sensitive parsing.

use std::fmt::Debug;

use rustc_hash::FxBuildHasher;

use crate::util::{
    bump::{
        ArenaMap,
        ArenaSet,
        ArenaVec,
        Bump,
    },
    region_bit_set::RegionBitSet,
    string_cache::StringCacheId,
};

/// Parser-visible classification in C's ordinary-identifier namespace.
///
/// C99: scopes and namespaces are §6.2.1-§6.2.3, pp. 29-31; PDF pp. 41-43.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum NameClass {
    /// Identifier currently denotes a typedef name.
    Typedef,
    /// Identifier denotes an object, function, parameter, or enumerator.
    Ordinary,
}

/// Kind of parser-visible scope whose lifetime is owned by one frame.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum ScopeKind {
    Function,
    FunctionPrototype,
    Block,
    ImplicitSelection,
    ImplicitIteration,
}

/// One open nested scope. Its bindings are the suffix of
/// [`ScopeStack::bindings`] that starts at `start`, so closing it pops them.
#[derive(Debug)]
pub(super) struct Scope {
    kind:  ScopeKind,
    start: usize,
}

/// Marks a binding that shadows no outer nested binding of its name.
const NO_OUTER_BINDING: usize = usize::MAX;

/// One nested binding of a name.
#[derive(Debug, Clone, Copy)]
struct Binding {
    name:  StringCacheId,
    /// The 1-based nested depth that bound the name.
    depth: u32,
    class: NameClass,
    /// The index of the binding this one shadows, or [`NO_OUTER_BINDING`].
    outer: usize,
}

/// Ordinary bindings retained from a closed file-level prototype scope, keyed
/// by the parameter list they belong to.
#[derive(Debug)]
struct RetainedPrototype {
    key:   (u32, u32),
    start: usize,
    end:   usize,
}

/// Upper bound on retained prototype records before the oldest are dropped.
/// A function definition consumes its record immediately after its declarator,
/// so only the few most recent records can still be needed.
const RETAINED_PROTOTYPE_LIMIT: usize = 64;

/// File, function, prototype, block, and implicit statement scopes used for
/// typedef-sensitive grammar choices.
///
/// Nested bindings form one stack in the parse arena. Closing a scope pops
/// its bindings and forgets the names that no open scope still binds, so
/// nested-scope storage is bounded by the bindings open at once, not by every
/// name the translation unit ever binds in a block.
///
/// File scope keeps only what the grammar asks of it: whether a name is a
/// typedef. That is one bit per interned name, in its own region, so the
/// parse arena does not grow with the number of file-scope declarations.
///
/// C99: identifier scopes are §6.2.1, pp. 29-30; PDF pp. 41-42; distinct
/// namespaces are §6.2.3, p. 31; PDF p. 43. Function-prototype scope ends at
/// the function declarator under §6.2.1 paragraph 4, p. 30; PDF p. 42.
pub(super) struct ScopeStack<'p> {
    /// The file-scope names whose latest declaration is a typedef, indexed
    /// by string-cache ID. A file-scope ordinary declaration of the name
    /// removes it.
    file_typedefs:            RegionBitSet,
    /// Nested scopes, with the innermost scope last.
    pub(super) nested_scopes: ArenaVec<'p, Scope>,
    /// Bindings of every open nested scope in the order they were made, so a
    /// scope's bindings follow those of the scopes enclosing it.
    bindings:                 ArenaVec<'p, Binding>,
    /// The innermost nested binding of each name that has one, as an index
    /// into `bindings`, so a lookup costs one probe however many scopes are
    /// open.
    innermost:                ArenaMap<'p, StringCacheId, usize>,
    /// Records of prototype scopes whose bindings outlive the scope for a
    /// possible function definition (C99 §6.2.1p4).
    retained_prototypes:      ArenaVec<'p, RetainedPrototype>,
    /// Bindings referenced by `retained_prototypes`.
    retained_names:           ArenaVec<'p, (StringCacheId, NameClass)>,
    #[cfg(test)]
    pub(super) trace:         Vec<ScopeTraceEvent>,
    /// Hash-map probes made by name lookups, for complexity regression tests.
    #[cfg(test)]
    pub(super) lookup_probes: std::cell::Cell<usize>,
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) struct ScopeTraceEvent {
    pub(super) kind:  ScopeKind,
    pub(super) enter: bool,
}

impl<'p> ScopeStack<'p> {
    /// An empty stack whose storage comes from the parse arena.
    pub(super) fn new_in(arena: &'p Bump) -> Self {
        Self {
            file_typedefs:              RegionBitSet::new(),
            nested_scopes:              ArenaVec::new_in(arena),
            bindings:                   ArenaVec::new_in(arena),
            innermost:                  ArenaMap::with_hasher_in(FxBuildHasher, arena),
            retained_prototypes:        ArenaVec::new_in(arena),
            retained_names:             ArenaVec::new_in(arena),
            #[cfg(test)]
            trace:                      Vec::new(),
            #[cfg(test)]
            lookup_probes:              std::cell::Cell::new(0),
        }
    }

    /// Tests the innermost visible ordinary-name binding for typedef status.
    pub(super) fn is_typedef(&self, name: StringCacheId) -> bool {
        if !self.nested_scopes.is_empty() {
            #[cfg(test)]
            self.lookup_probes.set(self.lookup_probes.get() + 1);
            if let Some(&index) = self.innermost.get(&name) {
                return self.bindings[index].class == NameClass::Typedef;
            }
        }
        #[cfg(test)]
        self.lookup_probes.set(self.lookup_probes.get() + 1);
        self.file_typedefs.contains(file_scope_index(name))
    }

    /// Whether the latest file-scope declaration of `name` is a typedef,
    /// whatever nested scopes are open.
    #[cfg(test)]
    pub(super) fn is_file_scope_typedef(&self, name: StringCacheId) -> bool {
        self.file_typedefs.contains(file_scope_index(name))
    }

    /// Publishes a binding in the innermost active scope.
    pub(super) fn publish(&mut self, name: StringCacheId, class: NameClass) {
        if self.nested_scopes.is_empty() {
            self.publish_at_file_scope(name, class);
        } else {
            _ = self.publish_nested(name, class);
        }
    }

    fn publish_at_file_scope(&mut self, name: StringCacheId, class: NameClass) {
        let index = file_scope_index(name);
        match class {
            | NameClass::Typedef => self.file_typedefs.insert(index),
            | NameClass::Ordinary => self.file_typedefs.remove(index),
        }
    }

    /// Publishes a binding in the innermost nested scope and reports whether
    /// the name was not yet bound in that scope.
    ///
    /// Only prototype scopes ask this. File scope keeps one typedef bit per
    /// name and cannot tell, so outside every nested scope the name is
    /// published at file scope and reported as not new.
    pub(super) fn publish_reporting_new(&mut self, name: StringCacheId, class: NameClass) -> bool {
        debug_assert!(
            !self.nested_scopes.is_empty(),
            "only nested scopes report new bindings"
        );
        if self.nested_scopes.is_empty() {
            self.publish_at_file_scope(name, class);
            return false;
        }
        self.publish_nested(name, class)
    }

    fn publish_nested(&mut self, name: StringCacheId, class: NameClass) -> bool {
        let depth = u32::try_from(self.nested_scopes.len()).unwrap_or(u32::MAX);
        let index = self.bindings.len();
        let innermost = self.innermost.entry(name).or_insert(NO_OUTER_BINDING);
        let outer = *innermost;
        if let Some(binding) = self.bindings.get_mut(outer)
            && binding.depth == depth
        {
            binding.class = class;
            false
        } else {
            *innermost = index;
            self.bindings.push(Binding {
                name,
                depth,
                class,
                outer,
            });
            true
        }
    }

    pub(super) fn depth(&self) -> usize {
        self.nested_scopes.len()
    }

    /// Number of distinct names bound in the innermost nested scope.
    pub(super) fn innermost_binding_count(&self) -> usize {
        self.nested_scopes
            .last()
            .map_or(0, |scope| self.bindings.len() - scope.start)
    }

    /// Opens a nested scope with an explicit grammar lifetime.
    pub(super) fn enter_scope(&mut self, kind: ScopeKind) {
        #[cfg(test)]
        self.trace.push(ScopeTraceEvent { kind, enter: true });
        self.nested_scopes.push(Scope {
            kind,
            start: self.bindings.len(),
        });
    }

    /// Restores exactly the depth recorded by the owning frame.
    pub(super) fn restore_depth(&mut self, depth: usize) {
        debug_assert!(
            depth <= self.nested_scopes.len(),
            "a frame cannot restore below the scope depth at which it started"
        );
        while self.nested_scopes.len() > depth {
            let Scope { kind, start } = self.nested_scopes.pop().expect("scope depth was checked");
            while self.bindings.len() > start {
                let binding = self.bindings.pop().expect("binding count was checked");
                if binding.outer == NO_OUTER_BINDING {
                    _ = self.innermost.remove(&binding.name);
                } else {
                    _ = self.innermost.insert(binding.name, binding.outer);
                }
            }
            #[cfg(test)]
            self.trace.push(ScopeTraceEvent { kind, enter: false });
            #[cfg(not(test))]
            let _ = kind;
        }
    }

    /// Keeps the innermost scope's bindings under `key` after the scope
    /// closes, so a function definition whose parameter list is `key` can
    /// republish them in its own scope.
    ///
    /// C99: identifiers declared in a function definition's parameter
    /// declarations have block scope ending with the body, §6.2.1p4, p. 30;
    /// PDF p. 42.
    pub(super) fn retain_innermost_bindings(&mut self, key: (u32, u32)) {
        let Some(scope) = self.nested_scopes.last() else {
            return;
        };
        if self.retained_prototypes.len() >= RETAINED_PROTOTYPE_LIMIT {
            // Forget the older half in place, so the records keep using the
            // same storage.
            let offset = self
                .retained_prototypes
                .get(RETAINED_PROTOTYPE_LIMIT / 2)
                .map_or(self.retained_names.len(), |record| record.start);
            self.retained_prototypes
                .drain(..RETAINED_PROTOTYPE_LIMIT / 2)
                .for_each(drop);
            self.retained_names.drain(..offset).for_each(drop);
            for record in &mut self.retained_prototypes {
                record.start -= offset;
                record.end -= offset;
            }
        }
        let start = self.retained_names.len();
        self.retained_names.extend(
            self.bindings[scope.start..]
                .iter()
                .map(|binding| (binding.name, binding.class)),
        );
        self.retained_prototypes.push(RetainedPrototype {
            key,
            start,
            end: self.retained_names.len(),
        });
    }

    /// Publishes the bindings retained under `key` in the innermost scope and
    /// discards every retained record. Returns whether `key` was retained.
    pub(super) fn publish_retained_bindings(&mut self, key: (u32, u32)) -> bool {
        let found = self
            .retained_prototypes
            .iter()
            .rev()
            .find(|record| record.key == key)
            .map(|record| (record.start, record.end));
        if let Some((start, end)) = found {
            for index in start..end {
                let (name, class) = self.retained_names[index];
                self.publish(name, class);
            }
        }
        self.retained_names.clear();
        self.retained_prototypes.clear();
        found.is_some()
    }
}

/// The bit that holds `name`'s file-scope typedef status.
fn file_scope_index(name: StringCacheId) -> usize {
    // Interned IDs count up from one, so the set stays dense.
    name.to_u32() as usize
}

/// Labels defined and referenced in one function body.
#[derive(Debug)]
pub(super) struct LabelScope<'p> {
    pub(super) definitions: ArenaSet<'p, StringCacheId>,
    pub(super) references:  ArenaSet<'p, StringCacheId>,
}

/// Function-local label namespaces, innermost last. A closed namespace keeps
/// its emptied sets for the next function body, so label bookkeeping reuses
/// the same parse-arena storage for the whole translation unit.
pub(super) struct LabelScopes<'p> {
    open:  ArenaVec<'p, LabelScope<'p>>,
    spare: ArenaVec<'p, LabelScope<'p>>,
    arena: &'p Bump,
}

impl<'p> LabelScopes<'p> {
    pub(super) fn new_in(arena: &'p Bump) -> Self {
        Self {
            open: ArenaVec::new_in(arena),
            spare: ArenaVec::new_in(arena),
            arena,
        }
    }

    /// Opens an empty label namespace.
    pub(super) fn enter(&mut self) {
        let scope = self.spare.pop().unwrap_or_else(|| LabelScope {
            definitions: ArenaSet::with_hasher_in(FxBuildHasher, self.arena),
            references:  ArenaSet::with_hasher_in(FxBuildHasher, self.arena),
        });
        self.open.push(scope);
    }

    /// Closes the innermost label namespace and reports whether one was open.
    pub(super) fn exit(&mut self) -> bool {
        let Some(mut scope) = self.open.pop() else {
            return false;
        };
        scope.definitions.clear();
        scope.references.clear();
        self.spare.push(scope);
        true
    }

    /// Closes every open label namespace.
    pub(super) fn exit_all(&mut self) {
        while self.exit() {}
    }

    pub(super) fn innermost_mut(&mut self) -> Option<&mut LabelScope<'p>> {
        self.open.last_mut()
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.open.is_empty()
    }
}

#[derive(Debug, Default)]
pub(super) struct SwitchScope {
    pub(super) has_default: bool,
}
