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
        self.control.scope_regions.push(self.control.current);
        self.scope = next;
        if kind == ScopeKind::Block {
            let source = self
                .control
                .point()
                .map(|point| self.control.regions[point.region.0].source);
            if let Some(source) = source {
                _ = self.control.open(
                    next,
                    super::RegionKind::Lexical,
                    super::CleanupKind::None,
                    source,
                );
            }
        }
    }

    /// Enters the block a selection or iteration statement forms, or one
    /// of its substatements, and says whether it did. C90 has no such
    /// blocks: a tag or enumeration constant declared in a controlling
    /// expression, or in a substatement that is no compound statement, and
    /// a `for` declaration, a C99 feature C90 mode accepts as an extension,
    /// belong to the enclosing block, as GCC and Clang scope them under
    /// `-std=c90`, `-std=iso9899:199409` (C95, which keeps C90's scopes)
    /// and `-std=gnu89` (GCC rejects the `for` declaration). A `compound`
    /// substatement is a block of its own, the one this enters.
    /// C99: §6.8.4 paragraph 3, p. 133; PDF p. 145; §6.8.5 paragraph 5,
    /// p. 135; PDF p. 147; Foreword paragraph 5, p. xii; PDF p. 10
    /// (the change from C90).
    pub(super) fn enter_statement_block(&mut self, compound: bool) -> bool {
        let entered = compound
            || self.context.configuration.standard() >= crate::configuration::CStandard::C99;
        if entered {
            self.enter(ScopeKind::Block);
        }
        entered
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
        self.control
            .close_to(self.control.scope_regions[self.scope]);
        self.scope = self.scopes[self.scope].parent.unwrap_or(0);
    }

    /// C99: §6.2.1p2-4, pp. 29-30; PDF pp. 41-42; §6.2.3p1, p. 31; PDF p. 43.
    pub(super) fn lookup(&self, namespace: Namespace, name: StringCacheId) -> Option<Entry> {
        self.visible
            .get(&(namespace, name))
            .map(|&i| self.entries[i])
    }

    /// Looks up a name as visible in an enclosing declaration scope.
    /// C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42.
    pub(super) fn lookup_at(
        &self,
        namespace: Namespace,
        name: StringCacheId,
        scope: usize,
    ) -> Option<Entry> {
        let mut index = self.visible.get(&(namespace, name)).copied()?;
        loop {
            let entry = self.entries[index];
            let mut ancestor = Some(scope);
            while let Some(scope) = ancestor {
                if scope == entry.scope {
                    return Some(entry);
                }
                ancestor = self.scopes[scope].parent;
            }
            index = entry.previous?;
        }
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
