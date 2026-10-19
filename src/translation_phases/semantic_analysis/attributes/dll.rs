//! Windows DLL storage: `dllimport` and `dllexport` on functions and
//! objects, merged over an entity's declarations in source order as Clang
//! merges them, and the code-generation state each entity ends with.
//!
//! Each declaration first applies its own attributes (`handleDLLAttr`):
//! MinGW ignores `dllimport` on an inline function, and `dllexport` on the
//! same declaration prevails. It then inherits the previous declaration's
//! storage (`mergeDeclAttributes`) and checks the redeclaration
//! (`checkDLLAttributeRedeclaration`): adding storage warns, or is an error
//! once the entity was used (unless a function is imported, which its
//! import thunk still serves); omitting `dllimport` drops it, except on an
//! inline function or a block-scope declaration, and a Microsoft definition
//! without it exports instead. Storage needs external linkage and no thread
//! storage; an imported function has only inline definitions, and imported
//! data none. An attribute first written after a definition is ignored
//! before any of this (`entities.rs`).
//!
//! For `__declspec(dllimport) int f(void); int g(void) { return f(); }
//! int f(void);`, the call in `g` reaches the import, and the last
//! declaration drops it with Clang's `-Winconsistent-dllimport` warning:
//! the undefined `f` stays imported, because the unit's last reference
//! named it so, as Clang's code generation fixes declarations at their
//! references.
//!
//! An object declared `dllimport` without a storage class is `extern`
//! ([`Analyzer::dll_import_storage`]), and its address is not an address
//! constant (`expressions.rs`): the loader writes it at run time.
//!
//! MSVC extension: Microsoft Learn, "dllexport, dllimport".
//! <https://learn.microsoft.com/en-us/cpp/cpp/dllexport-dllimport>
//! Clang extension: Clang `SemaDecl.cpp`, `checkDLLAttributeRedeclaration`;
//! `SemaDeclAttr.cpp`, `handleDLLAttr`.
//! C99: §6.2.2p2, p. 30; PDF p. 42 (linked declarations denote one entity);
//! §6.6p9, p. 96; PDF p. 108 (address constants).

use super::{
    super::{
        Analyzer,
        BindingKind,
        Declarator,
        Duration,
        Linkage,
        ScopeKind,
        SemanticErrorKind,
        facts::{
            DllStorage,
            EntityId,
            InlineOdr,
            Visibility,
        },
        types::{
            TypeId,
            TypeKind,
        },
    },
    Subject,
    catalog::{
        Rule,
        rule,
    },
    written::WrittenAttribute,
};
use crate::{
    target::Target,
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::DeclarationSpecifiers,
            syntax::StorageClass,
        },
    },
    util::bump::ArenaVec,
};

/// The DLL storage of an entity's latest declaration, and how this unit
/// has used the entity so far.
#[derive(Clone, Copy, Default)]
pub(in crate::translation_phases::semantic_analysis) struct DllState {
    /// Storage the latest declaration has, written or inherited, and where
    /// its attribute was written.
    current:  Option<(DllStorage, SourceVectors)>,
    /// The latest declaration, once there is one.
    declared: Option<SourceVectors>,
    /// Whether the unit's last evaluated reference in emitted code named
    /// an imported declaration.
    import:   Option<bool>,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Records a written `dllimport` or `dllexport` of the declaration being
    /// processed, ignoring it where Clang does: on anything but a function
    /// or object, and on MinGW for `dllimport` on an inline function. A
    /// parameter keeps it silently, as in Clang.
    pub(super) fn dll_attribute(
        &mut self,
        subject: Subject,
        attribute: WrittenAttribute<'_>,
        storage: DllStorage,
    ) {
        if !self.no_arguments(attribute) {
            return;
        }
        match subject {
            | Subject::Function { .. } | Subject::Object { .. } => self
                .attributes
                .pending_dll
                .push((storage, attribute.source)),
            | Subject::Parameter => {},
            | Subject::Typedef | Subject::Member => self.ignored(attribute),
        }
    }

    /// Applies the pending DLL attributes of one declaration of `subject`
    /// to its entity, after its other attributes. `initialized` says that
    /// an object declaration has an initializer.
    pub(in crate::translation_phases::semantic_analysis) fn declaration_dll(
        &mut self,
        subject: Subject,
        spec: DeclarationSpecifiers<'tu>,
        initialized: bool,
    ) {
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.extend_from_slice(&self.attributes.pending_dll);
        self.attributes.pending_dll.clear();
        let (Subject::Function {
            binding,
            definition,
        }
        | Subject::Object {
            binding,
            definition,
            ..
        }) = subject
        else {
            return;
        };
        let data = self.bindings[binding];
        if data.linkage == Linkage::None && pending.is_empty() {
            return;
        }
        let microsoft = self.context.configuration.target() == Target::WindowsMsvc;
        let function = data.kind == BindingKind::Function;
        // handleDLLAttr: this declaration's own attributes in order.
        let mut import = None;
        let mut export = None;
        for &(storage, source) in &pending {
            match storage {
                | DllStorage::Import
                    if function && spec.function_specifiers.is_inline && !microsoft =>
                    self.error(SemanticErrorKind::DllImportOnInline, source, None, None),
                | DllStorage::Import if export.is_some() => self.ignored_dll_import(source),
                | DllStorage::Import => import = import.or(Some(source)),
                | DllStorage::Export => {
                    if let Some(import) = import.take() {
                        self.ignored_dll_import(import);
                    }
                    export = export.or(Some(source));
                },
            }
        }
        let entity = (data.linkage != Linkage::None).then(|| self.entity_of(binding));
        let state = entity
            .and_then(|entity| self.attributes.dll.get(&entity).copied())
            .unwrap_or_default();
        let previous = state.declared;
        // mergeDeclAttributes: inherit the previous declaration's storage
        // unless this one conflicts with it.
        let inherited = match state.current {
            | Some((DllStorage::Import, source)) if export.is_some() => {
                self.ignored_dll_import(source);
                None
            },
            | Some((DllStorage::Export, _)) if import.is_some() => {
                self.ignored_dll_import(import.take().expect("checked"));
                state.current
            },
            | current => current,
        };
        let written = import
            .map(|source| (DllStorage::Import, source))
            .or(export.map(|source| (DllStorage::Export, source)));
        let mut current = written.or(inherited);
        if let Some(previous) = previous {
            let inlined = function && self.inline_declared(binding);
            let local = self.scopes[data.scope].kind != ScopeKind::File;
            // checkDLLAttributeRedeclaration: a redeclaration may not add
            // storage, nor drop an import outside the exempt cases.
            if let Some((storage, source)) = written
                && state.current.is_none()
            {
                if !(function && storage == DllStorage::Import)
                    && self.evaluated_reference(entity.expect("declared with linkage"))
                {
                    self.error(
                        SemanticErrorKind::DllAttributeAfterUse,
                        source,
                        Some(data.name.name),
                        Some(previous),
                    );
                    return;
                }
                self.error(
                    SemanticErrorKind::DllAttributeOnRedeclaration,
                    source,
                    Some(data.name.name),
                    Some(previous),
                );
            }
            if let Some((DllStorage::Import, source)) = state.current
                && written.is_none()
            {
                if !inlined && !local {
                    if microsoft && definition {
                        self.error(
                            SemanticErrorKind::DllExportAdded,
                            data.name.source_vectors,
                            Some(data.name.name),
                            Some(previous),
                        );
                        current = Some((DllStorage::Export, source));
                    } else {
                        self.error(
                            SemanticErrorKind::DllImportDropped,
                            data.name.source_vectors,
                            Some(data.name.name),
                            Some(previous),
                        );
                        current = None;
                    }
                } else if inlined && !microsoft {
                    self.error(
                        SemanticErrorKind::DllImportRedeclaredInline,
                        data.name.source_vectors,
                        Some(data.name.name),
                        None,
                    );
                    current = None;
                }
            }
        }
        if let Some((storage, _)) = current {
            self.check_dll_declaration(binding, storage, definition, initialized);
        }
        let Some(entity) = entity else { return };
        if current.is_some_and(|(storage, _)| storage == DllStorage::Export) {
            self.note_dll_export(binding);
        }
        let state = self.attributes.dll.entry(entity).or_default();
        state.current = current;
        state.declared = Some(data.name.source_vectors);
    }

    /// Clang's checks of a declaration's resulting storage: external
    /// linkage, no thread storage, visibility, and no definition of an
    /// imported entity except an inline function's.
    fn check_dll_declaration(
        &mut self,
        binding: usize,
        storage: DllStorage,
        definition: bool,
        initialized: bool,
    ) {
        let data = self.bindings[binding];
        let source = data.name.source_vectors;
        let name = Some(data.name.name);
        if data.linkage != Linkage::External {
            self.error(SemanticErrorKind::DllInternal, source, name, None);
            return;
        }
        if data.duration == Duration::Thread {
            self.error(SemanticErrorKind::DllThreadLocal, source, name, None);
            return;
        }
        let entity = self.entity_of(binding);
        let visibility = self.attributes.entity(entity).visibility;
        match (storage, visibility) {
            | (DllStorage::Export, Some(Visibility::Hidden)) =>
                self.error(SemanticErrorKind::DllExportHidden, source, name, None),
            | (DllStorage::Import, Some(Visibility::Hidden | Visibility::Protected)) =>
                self.error(SemanticErrorKind::DllImportVisibility, source, name, None),
            | _ => {},
        }
        if storage != DllStorage::Import {
            return;
        }
        if data.kind == BindingKind::Function && definition && !self.inline_declared(binding) {
            self.error(
                SemanticErrorKind::DllImportFunctionDefinition,
                source,
                name,
                None,
            );
        } else if data.kind == BindingKind::Object && initialized {
            self.error(
                SemanticErrorKind::DllImportDataDefinition,
                source,
                name,
                None,
            );
        }
    }

    /// Clang's `-Wignored-attributes` for a `dllimport` that `dllexport`
    /// overrides.
    fn ignored_dll_import(&mut self, source: SourceVectors) {
        let name = self.context.string_cache.intern("dllimport");
        self.error(
            SemanticErrorKind::IgnoredAttribute,
            source,
            Some(name),
            None,
        );
    }

    /// Whether a reference to `binding` names an imported declaration, for
    /// an entity with linkage. Only evaluated references in emitted code
    /// fix the entity's storage ([`Self::dll_referenced`]).
    pub(in crate::translation_phases::semantic_analysis) fn dll_reference(
        &mut self,
        binding: usize,
    ) -> Option<bool> {
        if self.bindings[binding].linkage == Linkage::None {
            return None;
        }
        Some(matches!(
            self.dll_storage(binding),
            Some(DllStorage::Import)
        ))
    }

    /// Notes an evaluated reference to `binding` in emitted code, in source
    /// order, and whether it named an imported declaration.
    pub(in crate::translation_phases::semantic_analysis) fn dll_referenced(
        &mut self,
        binding: usize,
        import: bool,
    ) {
        let entity = self.entity_of(binding);
        if let Some(state) = self.attributes.dll.get_mut(&entity) {
            state.import = Some(import);
        }
    }

    /// Whether GNU `used` retains the internal entity `entity`'s definition.
    pub(in crate::translation_phases::semantic_analysis) fn retained_by_used(
        &mut self,
        entity: EntityId,
    ) -> bool {
        self.attributes.entity(entity).used
    }

    /// The storage of the latest declaration of `binding`'s entity.
    pub(in crate::translation_phases::semantic_analysis) fn dll_storage(
        &mut self,
        binding: usize,
    ) -> Option<DllStorage> {
        if self.bindings[binding].linkage == Linkage::None {
            return None;
        }
        let entity = self.entity_of(binding);
        self.attributes
            .dll
            .get(&entity)
            .and_then(|state| state.current)
            .map(|(storage, _)| storage)
    }

    /// An object declared `dllimport`, without `dllexport`, and without a
    /// storage class is `extern`, as in Clang's `ActOnVariableDeclarator`;
    /// other declarations keep their specifiers.
    pub(in crate::translation_phases::semantic_analysis) fn dll_import_storage(
        &self,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Declarator<'tu>,
        ty: TypeId,
    ) -> DeclarationSpecifiers<'tu> {
        if spec.storage_class.is_some()
            || !matches!(
                self.context.configuration.target(),
                Target::WindowsGnu | Target::WindowsMsvc
            )
            || matches!(self.types.nodes[ty.index], TypeKind::Function { .. })
        {
            return spec;
        }
        let target = self.context.configuration.target();
        let (mut import, mut export) = (false, false);
        for specifier in self.declaration_attribute_lists(spec.extensions, Some(declarator)) {
            if specifier.recovered {
                continue;
            }
            for attribute in super::written::written(specifier, self.context) {
                match rule(
                    specifier.syntax,
                    attribute.namespace,
                    attribute.name,
                    target,
                ) {
                    | Some(Rule::DllImport) => import = true,
                    | Some(Rule::DllExport) => export = true,
                    | _ => {},
                }
            }
        }
        if import && !export {
            DeclarationSpecifiers {
                storage_class: Some(StorageClass::Extern),
                ..spec
            }
        } else {
            spec
        }
    }

    /// Gives each entity its code-generation storage: `Export` when its
    /// declarations end exported, else `Import` when the unit's last
    /// reference named an import (or, unreferenced, the declarations end
    /// imported).
    pub(super) fn finish_dll(&mut self) {
        let mut states = ArenaVec::new_in(self.scratch);
        states.extend(
            self.attributes
                .dll
                .iter()
                .map(|(&entity, &state)| (entity, state)),
        );
        for (entity, state) in states {
            let current = state.current.map(|(storage, _)| storage);
            let storage = match current {
                | Some(DllStorage::Export) => Some(DllStorage::Export),
                | _ if state.import.unwrap_or(current == Some(DllStorage::Import)) =>
                    Some(DllStorage::Import),
                | _ => None,
            };
            if storage.is_some() {
                self.attributes.entity(entity).dll = storage;
            }
        }
    }

    /// Records the COMDAT linkage of `binding`'s inline definition.
    pub(in crate::translation_phases::semantic_analysis) fn set_inline_odr(
        &mut self,
        binding: usize,
        odr: InlineOdr,
    ) {
        let entity = self.entity_of(binding);
        self.attributes.entity(entity).inline_odr = Some(odr);
    }

    /// Whether the entity `id` ends imported, for the definitions code
    /// generation treats as declarations.
    pub(in crate::translation_phases::semantic_analysis) fn ends_imported(
        &self,
        entity: EntityId,
    ) -> bool {
        self.attributes
            .dll
            .get(&entity)
            .is_some_and(|state| matches!(state.current, Some((DllStorage::Import, _))))
    }
}
