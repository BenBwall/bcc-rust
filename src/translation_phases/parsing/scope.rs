//! Ordinary-identifier scopes used for typedef-sensitive parsing.
//!
//! Translation phase 7 (§5.1.1.2, p. 10; PDF p. 22) needs to know, at each
//! identifier, whether it names a typedef: a `typedef-name` is an identifier
//! (C99: §6.7.7 paragraph 1, p. 123; PDF p. 135) that shares the ordinary
//! name space (paragraph 3, p. 123; PDF p. 135). This module tracks that one
//! fact over the scopes of §6.2.1, pp. 29-30; PDF pp. 41-42 (file, block,
//! and function prototype; labels' function scope is separate) and keeps
//! function-local label and `switch` state for the statement frames.
//!
//! Tags and members live in their own name spaces (§6.2.3 paragraph 1,
//! p. 31; PDF p. 43) and never change typedef classification, so they are
//! not tracked. Linkage (§6.2.2, pp. 30-31; PDF pp. 42-43), redeclaration
//! constraints (§6.7 paragraph 3, p. 97; PDF p. 109), and what each
//! identifier denotes are left to semantic analysis. Scopes nest through the
//! arena stack, so the minimums of 127 nested blocks and 511 block-scope
//! identifiers (§5.2.4.1, pp. 20-21; PDF pp. 32-33) impose no fixed ceiling.

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
/// Ordinary identifiers are those declared in ordinary declarators or as
/// enumeration constants (§6.2.3 paragraph 1, p. 31; PDF p. 43); a typedef
/// name is one of them (§6.7.7 paragraph 3, p. 123; PDF p. 135).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum NameClass {
    /// Identifier currently denotes a typedef name.
    Typedef,
    /// Identifier denotes an object, function, parameter, or enumerator.
    Ordinary,
}

/// Kind of parser-visible scope whose lifetime is owned by one frame.
///
/// C99: the scope kinds of §6.2.1 paragraphs 2 and 4, pp. 29-30;
/// PDF pp. 41-42.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum ScopeKind {
    /// The block scope of a function definition's parameters, which lasts
    /// until the end of the body (§6.2.1 paragraph 4, p. 29; PDF p. 41;
    /// §6.9.1 paragraph 9, p. 142; PDF p. 154). Labels' function scope
    /// (§6.2.1 paragraph 3, p. 29; PDF p. 41) is tracked by [`LabelScopes`].
    Function,
    /// Function prototype scope, ending with the function declarator
    /// (§6.2.1 paragraph 4, p. 30; PDF p. 42).
    FunctionPrototype,
    /// The block scope of a compound statement (§6.2.1 paragraph 4, p. 29;
    /// PDF p. 41; §6.8.2 paragraph 2, p. 132; PDF p. 144).
    Block,
    /// The block a selection statement forms (§6.8.4 paragraph 3, p. 133;
    /// PDF p. 145).
    ImplicitSelection,
    /// The block an iteration statement forms (§6.8.5 paragraph 5, p. 135;
    /// PDF p. 147).
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
    key:   (usize, usize),
    start: usize,
    end:   usize,
}

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
    pub(super) trace:         ScopeTrace,
    /// Hash-map probes made by name lookups, for complexity regression tests.
    #[cfg(test)]
    pub(super) lookup_probes: std::cell::Cell<usize>,
}

/// Scope entries and exits, recorded only by test builds.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "A test-only trace, compiled only under `cfg(test)`."
)]
pub(super) type ScopeTrace = Vec<ScopeTraceEvent>;

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
            trace:                      ScopeTrace::new(),
            #[cfg(test)]
            lookup_probes:              std::cell::Cell::new(0),
        }
    }

    /// Tests the innermost visible ordinary-name binding for typedef status.
    ///
    /// C99: an inner declaration hides an outer one of the same name space
    /// (§6.2.1 paragraph 4, p. 30; PDF p. 42).
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
        let depth =
            u32::try_from(self.nested_scopes.len()).expect("scope nesting depth exceeds u32::MAX");
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
    /// declarations have block scope ending with the body, §6.2.1p4, p. 29;
    /// PDF p. 41.
    pub(super) fn retain_innermost_bindings(&mut self, key: (usize, usize)) {
        let Some(scope) = self.nested_scopes.last() else {
            return;
        };
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
    pub(super) fn publish_retained_bindings(&mut self, key: (usize, usize)) -> bool {
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
        self.clear_retained_bindings();
        found.is_some()
    }

    /// No parameter list from a completed external declaration can belong to
    /// a later function definition.
    pub(super) fn clear_retained_bindings(&mut self) {
        self.retained_names.clear();
        self.retained_prototypes.clear();
    }
}

/// The bit that holds `name`'s file-scope typedef status.
fn file_scope_index(name: StringCacheId) -> usize {
    // Interned IDs count up from one, so the set stays dense.
    name.to_u32() as usize
}

/// Labels defined and referenced in one function body.
///
/// C99: label names have function scope (§6.2.1 paragraph 3, p. 29;
/// PDF p. 41) and their own name space (§6.2.3 paragraph 1, p. 31;
/// PDF p. 43). The sets only record names; uniqueness (§6.8.1 paragraph 3,
/// p. 132; PDF p. 144) and `goto` targets (§6.8.6.1 paragraph 1, p. 137;
/// PDF p. 149) are not diagnosed from them.
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

/// State of one enclosing `switch` body.
///
/// C99: a `switch` has at most one `default` label (§6.8.4.2 paragraph 3,
/// p. 134; PDF p. 146), diagnosed by the statement frame.
#[derive(Debug, Default)]
pub(super) struct SwitchScope {
    pub(super) has_default: bool,
}

/// Identifies a parameter list by where its slice lives in the
/// translation-unit arena and its length, so a definition can find the
/// bindings its prototype scope retained. Every empty list shares one key.
pub(super) fn list_key<T>(list: &[T]) -> (usize, usize) {
    (list.as_ptr().addr(), list.len())
}
