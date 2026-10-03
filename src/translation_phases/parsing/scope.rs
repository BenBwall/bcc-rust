//! Ordinary-identifier scopes used for typedef-sensitive parsing.

use std::fmt::Debug;

use crate::util::{
    HashMap,
    HashSet,
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

/// One open nested scope. Its bindings live in [`ScopeStack::shadows`]; the
/// scope records only which names it bound so closing it can pop them.
#[derive(Default)]
pub(super) struct Scope {
    kind:  Option<ScopeKind>,
    names: Vec<StringCacheId>,
}

/// One nested binding of a name: the 1-based nested depth that bound it and
/// its classification.
type Shadow = (u32, NameClass);

/// Ordinary bindings retained from a closed file-level prototype scope, keyed
/// by the parameter list they belong to.
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
/// C99: identifier scopes are §6.2.1, pp. 29-30; PDF pp. 41-42; distinct
/// namespaces are §6.2.3, p. 31; PDF p. 43. Function-prototype scope ends at
/// the function declarator under §6.2.1 paragraph 4, p. 30; PDF p. 42.
#[derive(Default)]
pub(super) struct ScopeStack {
    /// Bindings visible for the translation unit.
    pub(super) file_scope:    HashMap<StringCacheId, NameClass>,
    /// Nested scopes, with the innermost scope last.
    pub(super) nested_scopes: Vec<Scope>,
    /// Per-name stack of nested bindings, innermost last, so a lookup costs
    /// one probe however many scopes are open. A name's stack may be empty
    /// once the scopes that bound it have closed.
    shadows:                  HashMap<StringCacheId, Vec<Shadow>>,
    /// Emptied name lists from closed scopes, reused so entering a scope does
    /// not allocate again.
    spare_names:              Vec<Vec<StringCacheId>>,
    /// Records of prototype scopes whose bindings outlive the scope for a
    /// possible function definition (C99 §6.2.1p4).
    retained_prototypes:      Vec<RetainedPrototype>,
    /// Bindings referenced by `retained_prototypes`.
    retained_names:           Vec<(StringCacheId, NameClass)>,
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

impl ScopeStack {
    /// Tests the innermost visible ordinary-name binding for typedef status.
    pub(super) fn is_typedef(&self, name: StringCacheId) -> bool {
        if !self.nested_scopes.is_empty() {
            #[cfg(test)]
            self.lookup_probes.set(self.lookup_probes.get() + 1);
            if let Some(&(_, class)) = self.shadows.get(&name).and_then(|shadows| shadows.last()) {
                return class == NameClass::Typedef;
            }
        }
        #[cfg(test)]
        self.lookup_probes.set(self.lookup_probes.get() + 1);
        self.file_scope.get(&name) == Some(&NameClass::Typedef)
    }

    /// Publishes a binding in the innermost active scope.
    pub(super) fn publish(&mut self, name: StringCacheId, class: NameClass) {
        _ = self.publish_reporting_new(name, class);
    }

    /// Publishes a binding in the innermost active scope and reports whether
    /// the name was not yet bound in that scope.
    pub(super) fn publish_reporting_new(&mut self, name: StringCacheId, class: NameClass) -> bool {
        let depth = u32::try_from(self.nested_scopes.len()).unwrap_or(u32::MAX);
        let Some(scope) = self.nested_scopes.last_mut() else {
            return self.file_scope.insert(name, class).is_none();
        };
        let shadows = self.shadows.entry(name).or_default();
        if let Some(shadow) = shadows.last_mut()
            && shadow.0 == depth
        {
            shadow.1 = class;
            false
        } else {
            shadows.push((depth, class));
            scope.names.push(name);
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
            .map_or(0, |scope| scope.names.len())
    }

    /// Opens a nested scope with an explicit grammar lifetime.
    pub(super) fn enter_scope(&mut self, kind: ScopeKind) {
        #[cfg(test)]
        self.trace.push(ScopeTraceEvent { kind, enter: true });
        self.nested_scopes.push(Scope {
            kind:  Some(kind),
            names: self.spare_names.pop().unwrap_or_default(),
        });
    }

    /// Restores exactly the depth recorded by the owning frame.
    pub(super) fn restore_depth(&mut self, depth: usize) {
        debug_assert!(
            depth <= self.nested_scopes.len(),
            "a frame cannot restore below the scope depth at which it started"
        );
        while self.nested_scopes.len() > depth {
            let Scope { kind, mut names } =
                self.nested_scopes.pop().expect("scope depth was checked");
            let kind = kind.expect("nested scopes have a kind");
            for name in names.drain(..) {
                if let Some(shadows) = self.shadows.get_mut(&name) {
                    _ = shadows.pop();
                }
            }
            self.spare_names.push(names);
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
            let keep = self
                .retained_prototypes
                .split_off(RETAINED_PROTOTYPE_LIMIT / 2);
            let offset = keep
                .first()
                .map_or(self.retained_names.len(), |record| record.start);
            self.retained_names.drain(..offset).for_each(drop);
            self.retained_prototypes = keep
                .into_iter()
                .map(|record| RetainedPrototype {
                    key:   record.key,
                    start: record.start - offset,
                    end:   record.end - offset,
                })
                .collect();
        }
        let start = self.retained_names.len();
        for &name in &scope.names {
            let class = self
                .shadows
                .get(&name)
                .and_then(|shadows| shadows.last())
                .map_or(NameClass::Ordinary, |shadow| shadow.1);
            self.retained_names.push((name, class));
        }
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
        let names = std::mem::take(&mut self.retained_names);
        if let Some((start, end)) = found {
            for &(name, class) in &names[start..end] {
                self.publish(name, class);
            }
        }
        self.retained_names = names;
        self.retained_names.clear();
        self.retained_prototypes.clear();
        found.is_some()
    }
}

#[derive(Debug, Default)]
pub(super) struct LabelScope {
    pub(super) definitions: HashSet<StringCacheId>,
    pub(super) references:  HashSet<StringCacheId>,
}

#[derive(Debug, Default)]
pub(super) struct SwitchScope {
    pub(super) has_default: bool,
}
