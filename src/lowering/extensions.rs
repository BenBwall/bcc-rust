//! Vendor declaration modifiers retained by semantic analysis's iterative
//! type traversal. Resolve symbol aliases before planning reachability, then
//! check modifiers belonging to emitted definitions and file declarations.
//! C99: §6.7, p. 97; PDF p. 109; §6.7.5-6.7.6, pp. 114-122;
//! PDF pp. 126-134 (GNU/MSVC extensions to declarations and type names).

use super::{
    Construct,
    LoweringError,
    module_items::Unit,
};
use crate::{
    target::Target,
    translation_phases::{
        SourceVectors,
        parsing::{
            AttributeSpecifier,
            SpecifierExtension,
            SpecifierExtensionKind,
            declaration_syntax::DirectDeclarator,
        },
        preprocessing::{
            KeywordTokenType,
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
    /// Records assembler labels before reachability merges linked declarations.
    /// C99: §6.2.2 paragraph 2, p. 30; PDF p. 42 (linked declarations);
    /// §6.7, p. 97; PDF p. 109 (GNU/MSVC declaration extensions).
    pub(super) fn prepare_symbol_names(&mut self) -> ArenaVec<'a, (usize, LoweringError)> {
        let mut errors = ArenaVec::new_in(self.tables);
        for (index, declarator) in self.sema.declarators.iter().enumerate() {
            for direct in declarator.kind {
                if let DirectDeclarator::AsmLabel(label) = *direct
                    && self
                        .record_label(*declarator, label.template, label.recovered)
                        .is_err()
                {
                    errors.push((
                        index,
                        LoweringError::unsupported(Construct::AssemblerLabel, label.source_vectors),
                    ));
                }
            }
        }
        errors
    }

    /// Retained ownership keeps unsupported modifiers in unreachable bodies
    /// from refusing the module. File declarations still constrain emitted
    /// uses. C99: §6.9 paragraphs 3-5, p. 140; PDF p. 152.
    pub(super) fn check_extensions(&self, errors: &mut ArenaVec<'_, LoweringError>) {
        for (index, &chain) in self.sema.spec_extensions.iter().enumerate() {
            if self.sema.specifier_functions[index].is_some_and(|owner| !self.emits_function(owner))
            {
                continue;
            }
            self.check_chain(Some(chain), errors);
        }
        for (index, declarator) in self.sema.declarators.iter().enumerate() {
            if self.sema.declarator_functions[index]
                .is_some_and(|owner| !self.emits_function(owner))
            {
                continue;
            }
            for level in declarator.pointer.levels {
                self.check_chain(level.attributes, errors);
            }
            for direct in declarator.kind {
                match *direct {
                    | DirectDeclarator::Attributes(attribute) =>
                        self.check_attribute(attribute, errors),
                    | DirectDeclarator::MsModifier(keyword, source) =>
                        Self::check_keyword(keyword, source, errors),
                    | _ => {},
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

    fn check_chain(
        &self,
        mut chain: Option<&SpecifierExtension<'_>>,
        errors: &mut ArenaVec<'_, LoweringError>,
    ) {
        while let Some(item) = chain {
            match item.kind {
                | SpecifierExtensionKind::Attributes(a) => self.check_attribute(a, errors),
                | SpecifierExtensionKind::MsModifier(k) =>
                    Self::check_keyword(k, item.source_vectors, errors),
                | _ => {},
            }
            chain = item.next;
        }
    }

    fn check_keyword(
        keyword: KeywordTokenType,
        source: SourceVectors,
        errors: &mut ArenaVec<'_, LoweringError>,
    ) {
        if keyword == KeywordTokenType::Vectorcall {
            errors.push(LoweringError::unsupported(
                Construct::CallingConvention,
                source,
            ));
        }
    }

    /// Attribute names occur at list depth, never inside argument tokens.
    /// C99: §6.7, p. 97; PDF p. 109 (GNU/MSVC attribute extension).
    fn check_attribute(
        &self,
        attribute: &AttributeSpecifier<'_>,
        errors: &mut ArenaVec<'_, LoweringError>,
    ) {
        let list_depth = match attribute.tokens.first().map(|token| {
            self.context
                .string_cache
                .at(token.contents)
                .trim_matches('_')
        }) {
            | Some("attribute") => 2,
            | Some("declspec") => 1,
            | _ => 0,
        };
        let mut depth = 0_usize;
        for token in attribute.tokens {
            let name = self
                .context
                .string_cache
                .at(token.contents)
                .trim_end_matches('\0');
            match name {
                | "(" => depth += 1,
                | ")" => depth = depth.saturating_sub(1),
                | _ if depth == list_depth && !self.default_convention(name.trim_matches('_')) => {
                    errors.push(LoweringError::unsupported(
                        Construct::CallingConvention,
                        attribute.source_vectors,
                    ));
                },
                | _ => {},
            }
        }
    }

    /// Bundled Clang 23 leaves stdcall/cdecl/fastcall/thiscall/regparm at
    /// the x86-64 default. Explicit ABI selection must match the target.
    /// C99: §6.5.2.2, pp. 71-72; PDF pp. 83-84 (vendor call extensions).
    fn default_convention(&self, name: &str) -> bool {
        let windows = matches!(self.target, Target::WindowsGnu | Target::WindowsMsvc);
        match name {
            | "ms_abi" => windows,
            | "sysv_abi" => !windows,
            | "vectorcall"
            | "regcall"
            | "preserve_most"
            | "preserve_all"
            | "preserve_none"
            | "swiftcall"
            | "swiftasynccall"
            | "intel_ocl_bicc"
            | "pcs"
            | "aarch64_vector_pcs"
            | "aarch64_sve_pcs"
            | "riscv_vector_cc"
            | "no_caller_saved_registers"
            | "no_callee_saved_registers"
            | "interrupt" => false,
            | _ => true,
        }
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
        let binding = self.declarator_binding(declarator).ok_or(())?;
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
