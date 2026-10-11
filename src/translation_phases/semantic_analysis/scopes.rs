//! Phase-7 name installation, lookup and scope restoration. Each installed
//! entry remembers the binding it shadows; leaving a scope restores those
//! entries. Ordinary names, tags and GNU local labels have separate keys.
//! Redeclaration compatibility and linkage checks remain in the declaration
//! analyzer. C99: §6.2.1 paragraph 4, pp. 29-30; PDF pp. 41-42;
//! §6.2.3 paragraph 1, p. 31; PDF p. 43.

use super::{
    Analyzer,
    Scope,
    ScopeKind,
    collection::Collection,
};
use crate::{
    translation_phases::parsing::syntax::Identifier,
    util::string_cache::StringCacheId,
};

impl Analyzer<'_, '_, '_> {
    /// C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42.
    pub(super) fn enter(&mut self, kind: ScopeKind) {
        let next = self.scopes.len();
        self.scopes.push(Scope {
            parent: Some(self.scope),
            kind,
        });
        self.scope_entries
            .push(self.scratch.alloc(Collection::new()));
        self.statements.scope_vm.push(self.statements.vm);
        self.scope = next;
    }

    /// C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42.
    pub(super) fn leave(&mut self) {
        let mut next = self.scope_entries[self.scope].head.get();
        while let Some(link) = next {
            next = link.next;
            let entry = self.entries[link.value];
            if let Some(previous) = entry.previous {
                _ = self
                    .visible
                    .insert((entry.namespace, entry.name.name), previous);
            } else {
                _ = self.visible.remove(&(entry.namespace, entry.name.name));
            }
        }
        self.statements.vm = self.statements.scope_vm[self.scope];
        self.scope = self.scopes[self.scope].parent.unwrap_or(0);
    }

    /// C99: §6.2.1p2-4, pp. 29-30; PDF pp. 41-42; §6.2.3p1, p. 31; PDF p. 43.
    pub(super) fn lookup(&self, namespace: Namespace, name: StringCacheId) -> Option<Entry> {
        self.visible
            .get(&(namespace, name))
            .map(|&i| self.entries[i])
    }

    /// C99: §6.2.1p7, p. 30; PDF p. 42; §6.2.3p1, p. 31; PDF p. 43.
    pub(super) fn install(&mut self, name: Identifier, namespace: Namespace, binding: usize) {
        let previous = self.visible.get(&(namespace, name.name)).copied();
        let index = self.entries.len();
        self.entries.push(Entry {
            name,
            namespace,
            binding,
            scope: self.scope,
            previous,
        });
        _ = self.visible.insert((namespace, name.name), index);
        self.scope_entries[self.scope].push(self.scratch, index);
    }
}

#[derive(Clone, Copy)]
pub(super) struct Entry {
    pub(super) name:      Identifier,
    pub(super) namespace: Namespace,
    pub(super) scope:     usize,
    pub(super) binding:   usize,
    previous:             Option<usize>,
}

/// Separate identifier namespaces for ordinary names, tags and GNU local
/// labels.
/// GNU extension: GCC manual, "Local Labels".
/// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
/// C99: §6.2.3 paragraph 1, p. 31; PDF p. 43.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Namespace {
    Ordinary,
    Tag,
    Label,
}
