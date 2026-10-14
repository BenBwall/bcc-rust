//! Vendor declaration modifiers. Semantic analysis validates attributes and
//! conventions once and retains the meanings code generation lacks as
//! capability refusals, wherever they were written: declarations, typedefs,
//! abstract casts or function pointers. This file reports those refusals
//! and records GNU assembler labels before any declaration or definition
//! can reserve the ordinary C spelling.
//! C99: §6.7, p. 97; PDF p. 109; §6.7.5-6.7.6, pp. 114-122;
//! PDF pp. 126-134 (GNU/MSVC extensions to declarations and type names).

use super::{
    Construct,
    LoweringError,
    module_items::Unit,
};
use crate::{
    translation_phases::{
        parsing::declaration_syntax::DirectDeclarator,
        preprocessing::{
            StringTokenType,
            Token,
            TokenType,
        },
        semantic_analysis::{
            BindingKind,
            Duration,
            Linkage,
        },
    },
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

impl<'a, 'tu> Unit<'a, 'tu, '_> {
    /// Refuses attribute meanings code generation lacks and records
    /// assembler labels.
    /// C99: §6.2.2 paragraph 2, p. 30; PDF p. 42 (linked declarations);
    /// §6.7, p. 97; PDF p. 109 (GNU/MSVC declaration extensions).
    pub(super) fn prepare_extensions(&mut self, errors: &mut ArenaVec<'_, LoweringError>) {
        for refusal in self.sema.symbols.refusals {
            errors.push(LoweringError::unsupported(
                Construct::Attribute(refusal.capability),
                refusal.source,
            ));
        }
        for declarator in self.sema.declarators {
            for direct in declarator.kind {
                if let DirectDeclarator::AsmLabel(label) = *direct {
                    if self
                        .sema
                        .correspondence
                        .declaration(*declarator)
                        .and_then(|fact| fact.binding)
                        .is_none()
                    {
                        errors.push(LoweringError::missing(
                            "assembler-label declaration occurrence is not retained",
                            declarator.source_vectors,
                        ));
                        continue;
                    }
                    let result = self.record_label(*declarator, label.template, label.recovered);
                    if result.is_err() {
                        errors.push(LoweringError::unsupported(
                            Construct::AssemblerLabel,
                            label.source_vectors,
                        ));
                    }
                }
            }
        }
    }

    /// C identifiers are NUL-free; assembler names pass the explicit gate.
    /// C99: §6.4.3 paragraph 2, p. 53; PDF p. 65.
    pub(super) fn symbol_name(&self, binding: usize) -> &'a str {
        let data = self.sema.bindings[binding];
        let name = if data.linkage == Linkage::None {
            self.names.locals.get(&binding).copied()
        } else {
            self.names.linked.get(&data.name.name).copied()
        }
        .unwrap_or_else(|| self.context.string_cache.at(data.name.name));
        debug_assert!(!name.contains('\0'), "lowered symbol names are NUL-free");
        name
    }

    /// GNU asm labels extend the name of a linked or static-duration entity.
    /// Automatic register labels and non-narrow, empty or NUL-bearing names
    /// cannot be represented by this lowering.
    /// C99: §6.2.2, pp. 30-31; PDF pp. 42-43; §6.2.4, p. 32;
    /// PDF p. 44 (GNU assembler-label extension).
    fn record_label(
        &mut self,
        declarator: crate::translation_phases::parsing::declaration_syntax::Declarator<'tu>,
        template: Option<Token>,
        recovered: bool,
    ) -> Result<(), ()> {
        let binding = self
            .sema
            .correspondence
            .declaration(declarator)
            .and_then(|fact| fact.binding)
            .ok_or(())?;
        let data = self.sema.bindings[binding];
        if recovered
            || !matches!(data.kind, BindingKind::Object | BindingKind::Function)
            || (data.linkage == Linkage::None && data.duration != Duration::Static)
        {
            return Err(());
        }
        let Some(TokenType::String(StringTokenType::String(id))) = template.map(|token| token.kind)
        else {
            return Err(());
        };
        let mut bytes = self.string_bytes(id);
        _ = bytes.pop(); // The literal's implicit terminating NUL is not its name.
        let name = std::str::from_utf8(&bytes).map_err(|_| ())?;
        if name.is_empty() || name.contains('\0') || name.starts_with('\u{1}') {
            return Err(());
        }
        let map = if data.linkage == Linkage::None {
            &mut self.names.locals
        } else {
            if let Some(previous) = self.names.linked.get(&data.name.name) {
                return if *previous == name { Ok(()) } else { Err(()) };
            }
            _ = self
                .names
                .linked
                .insert(data.name.name, self.tables.alloc_str(name));
            return Ok(());
        };
        if let Some(previous) = map.get(&binding) {
            return if *previous == name { Ok(()) } else { Err(()) };
        }
        _ = map.insert(binding, self.tables.alloc_str(name));
        Ok(())
    }
}

/// Assembler names shared by linked declarations, or owned by a static local.
pub(super) struct SymbolNames<'a> {
    linked: ArenaMap<'a, StringCacheId, &'a str>,
    locals: ArenaMap<'a, usize, &'a str>,
}

impl<'a> SymbolNames<'a> {
    pub(super) fn new(arena: &'a Bump) -> Self {
        Self {
            linked: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            locals: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
        }
    }

    pub(super) fn local(&self, binding: usize) -> Option<&'a str> {
        self.locals.get(&binding).copied()
    }

    pub(super) fn linked(&self, name: StringCacheId) -> Option<&'a str> {
        self.linked.get(&name).copied()
    }
}
