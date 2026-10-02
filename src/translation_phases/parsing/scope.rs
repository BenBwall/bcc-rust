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

#[derive(Default)]
pub(super) struct Scope {
    kind:     Option<ScopeKind>,
    bindings: HashMap<StringCacheId, NameClass>,
}

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
    /// Emptied binding maps from closed scopes, reused so entering a scope
    /// does not allocate again.
    spare_bindings:           Vec<HashMap<StringCacheId, NameClass>>,
    #[cfg(test)]
    pub(super) trace:         Vec<ScopeTraceEvent>,
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
        self.nested_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.bindings.get(&name))
            .or_else(|| self.file_scope.get(&name))
            == Some(&NameClass::Typedef)
    }

    /// Publishes a binding in the innermost active scope.
    pub(super) fn publish(&mut self, name: StringCacheId, class: NameClass) {
        if let Some(scope) = self.nested_scopes.last_mut() {
            _ = scope.bindings.insert(name, class);
        } else {
            _ = self.file_scope.insert(name, class);
        }
    }

    pub(super) fn depth(&self) -> usize {
        self.nested_scopes.len()
    }

    /// Opens a nested scope with an explicit grammar lifetime.
    pub(super) fn enter_scope(&mut self, kind: ScopeKind) {
        #[cfg(test)]
        self.trace.push(ScopeTraceEvent { kind, enter: true });
        self.nested_scopes.push(Scope {
            kind:     Some(kind),
            bindings: self.spare_bindings.pop().unwrap_or_default(),
        });
    }

    /// Restores exactly the depth recorded by the owning frame.
    pub(super) fn restore_depth(&mut self, depth: usize) {
        debug_assert!(
            depth <= self.nested_scopes.len(),
            "a frame cannot restore below the scope depth at which it started"
        );
        while self.nested_scopes.len() > depth {
            let Scope { kind, mut bindings } =
                self.nested_scopes.pop().expect("scope depth was checked");
            let kind = kind.expect("nested scopes have a kind");
            bindings.clear();
            self.spare_bindings.push(bindings);
            #[cfg(test)]
            self.trace.push(ScopeTraceEvent { kind, enter: false });
            #[cfg(not(test))]
            let _ = kind;
        }
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
