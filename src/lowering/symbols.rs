//! Entity attributes as IR symbol facts. Semantic analysis validated and
//! merged every attribute; this file only maps the retained records onto
//! the IR: weak and COMDAT linkage, section, retention, visibility, DLL
//! storage, code alignment and inline control of each declared symbol,
//! aliases of definitions, and the constructor and destructor lists. No
//! attribute token is read here.
//! GNU extension: GCC manual, "Common Function Attributes" and "Common
//! Variable Attributes" (alias, constructor, destructor, section, used,
//! visibility, weak, aligned).
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>
//! C99: §6.2.2p2, p. 30; PDF p. 42 (one entity per linked identifier).

use super::{
    Construct,
    LoweringError,
    module_items::{
        Unit,
        at,
    },
};
use crate::{
    configuration::DefaultVisibility,
    ir::{
        Align,
        DllStorage as IrDllStorage,
        InitFunction,
        InitKind,
        InlineControl,
        Linkage as IrLinkage,
        Symbol,
        SymbolAttributes,
        Visibility as IrVisibility,
    },
    translation_phases::semantic_analysis::{
        BindingKind,
        DllStorage,
        InitFunctionKind,
        InlineOdr,
        Inlining,
        Linkage,
        UnsupportedAttribute,
        Visibility,
    },
    util::bump::ArenaVec,
};

impl Unit<'_, '_, '_> {
    /// The IR linkage of the entity `binding` declares: internal for C
    /// internal linkage, weak for a `weak` external entity, selectany for a
    /// Microsoft `selectany` object this unit defines and for an inline
    /// function whose units each keep a definition, linkonce for one that
    /// only units reaching it emit, else external.
    /// MSVC extension: Microsoft Learn, "selectany" and "inline"; Microsoft
    /// ABI extension under C99 §4p6, p. 7; PDF p. 19.
    /// C99: §6.2.2p3-5, pp. 30-31; PDF pp. 42-43.
    pub(super) fn ir_linkage(&self, binding: usize) -> IrLinkage {
        let attributes = self.sema.entity_attributes(binding);
        match self.sema.bindings[binding].linkage {
            | Linkage::Internal => IrLinkage::Internal,
            | Linkage::External if attributes.is_some_and(|attributes| attributes.weak) =>
                IrLinkage::Weak,
            // A `selectany` object this unit defines; another unit's
            // definition is an ordinary external reference here.
            | Linkage::External
                if attributes.is_some_and(|attributes| attributes.select_any)
                    && self.defines_object(binding) =>
                IrLinkage::SelectAny,
            // Semantic analysis records this only for an inline body the
            // unit emits once reached, so the symbol is always defined.
            | Linkage::External if self.sema.bindings[binding].kind == BindingKind::Function =>
                match attributes.and_then(|attributes| attributes.inline_odr) {
                    | Some(InlineOdr::Strong) => IrLinkage::SelectAny,
                    | Some(InlineOdr::Discardable) => IrLinkage::LinkOnce,
                    | None => IrLinkage::External,
                },
            | Linkage::External if self.common_symbol(binding) => IrLinkage::Common,
            | Linkage::External | Linkage::None => IrLinkage::External,
        }
    }

    /// Whether `-fcommon` makes the object `binding` a common symbol: an
    /// external one this unit defines only tentatively, unless an
    /// attribute asks for a section or for Windows DLL storage, or it is
    /// thread-local, as Clang's `isVarDeclStrongDefinition` decides. On the
    /// MSVC target an explicitly aligned object is no common symbol
    /// either, as there; a WebAssembly module has none.
    /// Extension: C99 §J.5.11, p. 514; PDF p. 526.
    fn common_symbol(&self, binding: usize) -> bool {
        let data = self.sema.bindings[binding];
        if !self.context.code_assumptions.common_symbols
            || self.target.is_wasm()
            || data.kind != BindingKind::Object
            || data.duration == crate::translation_phases::semantic_analysis::Duration::Thread
            || !self.defines_tentatively(binding)
        {
            return false;
        }
        self.sema
            .entity_attributes(binding)
            .is_none_or(|attributes| {
                attributes.section.is_none()
                    && attributes.dll.is_none()
                    && !(self.target == crate::target::Target::WindowsMsvc
                        && attributes.declared_alignment)
            })
    }

    /// A definition this unit emits is not imported: a later definition
    /// drops an earlier `dllimport` (`attributes/dll.rs`), which the
    /// declaration lowered first may still carry.
    pub(super) fn defines_symbol(&mut self, symbol: Symbol) {
        let mut attributes = self.module.symbol_attributes(symbol);
        if attributes.dll == IrDllStorage::Import {
            attributes.dll = IrDllStorage::Default;
            self.module.set_symbol_attributes(symbol, attributes);
        }
    }

    /// Windows DLL storage applies to what this unit emits: an export to a
    /// definition (a body, an initializer or an alias), an import to a
    /// declaration. Otherwise it has no effect, as in Clang, which also
    /// keeps an imported `selectany` definition but emits it as a local
    /// definition here. MSVC extension: Microsoft Learn, "dllexport,
    /// dllimport".
    pub(super) fn settle_dll_storage(&mut self) {
        let mut symbols = ArenaVec::new_in(self.tables);
        symbols.extend(self.module.functions().map(|(id, _)| Symbol::Function(id)));
        symbols.extend(self.module.globals().map(|(id, _)| Symbol::Global(id)));
        for symbol in symbols {
            let mut attributes = self.module.symbol_attributes(symbol);
            let defined = attributes.alias.is_some()
                || match symbol {
                    | Symbol::Function(func) => self.module.function(func).body.is_some(),
                    | Symbol::Global(global) => self.module.global(global).init.is_some(),
                };
            let applies = match attributes.dll {
                | IrDllStorage::Default => true,
                | IrDllStorage::Export => defined,
                | IrDllStorage::Import => !defined,
            };
            if !applies {
                attributes.dll = IrDllStorage::Default;
                self.module.set_symbol_attributes(symbol, attributes);
            }
            // `-fvisibility=` gives a definition with linkage the
            // visibility its entity does not name; declarations keep the
            // default, as in GCC.
            let linkage = match symbol {
                | Symbol::Function(func) => self.module.function(func).linkage,
                | Symbol::Global(global) => self.module.global(global).linkage,
            };
            if defined
                && linkage != IrLinkage::Internal
                && attributes.visibility == IrVisibility::Default
                && !self.explicit_visibility.contains(&symbol)
            {
                let visibility = match self.context.configuration.default_visibility() {
                    | DefaultVisibility::Default => continue,
                    | DefaultVisibility::Hidden => IrVisibility::Hidden,
                    | DefaultVisibility::Protected => IrVisibility::Protected,
                };
                attributes.visibility = visibility;
                self.module.set_symbol_attributes(symbol, attributes);
            }
        }
    }

    /// Gives a newly declared symbol its entity's section, retention,
    /// visibility and, for a function, code alignment and returns-twice
    /// fact.
    pub(super) fn symbol_attributes(&mut self, binding: usize, symbol: Symbol) {
        if let Symbol::Function(func) = symbol {
            self.module.function_mut(func).returns_twice = self
                .sema
                .entity(binding)
                .expect(super::ENTITY_FACTS)
                .returns_twice;
        }
        let Some(&attributes) = self.sema.entity_attributes(binding) else {
            return;
        };
        if attributes.visibility.is_some() {
            _ = self.explicit_visibility.insert(symbol);
        }
        let visibility = match attributes.visibility {
            | None | Some(Visibility::Default) => IrVisibility::Default,
            | Some(Visibility::Hidden) => IrVisibility::Hidden,
            | Some(Visibility::Protected) => IrVisibility::Protected,
        };
        self.module.set_symbol_attributes(
            symbol,
            SymbolAttributes {
                alias: None,
                section: attributes.section,
                used: attributes.used,
                visibility,
                dll: match attributes.dll {
                    | None => IrDllStorage::Default,
                    | Some(DllStorage::Import) => IrDllStorage::Import,
                    | Some(DllStorage::Export) => IrDllStorage::Export,
                },
            },
        );
        if let Symbol::Function(func) = symbol {
            let function = self.module.function_mut(func);
            if attributes.code_alignment > 1 {
                function.align = Align::from_bytes(u64::from(attributes.code_alignment));
            }
            function.inlining = match attributes.inlining {
                | Inlining::Default => InlineControl::Default,
                | Inlining::Never => InlineControl::Never,
                | Inlining::Always => InlineControl::Always,
            };
        }
    }

    /// Declares every alias as another name for its target's symbol, and
    /// registers constructor and destructor definitions in definition order.
    /// An alias is emitted even when this translation unit never uses it.
    pub(super) fn declare_aliases_and_initializers(
        &mut self,
        errors: &mut ArenaVec<'_, LoweringError>,
    ) {
        for alias in self.sema.symbols.aliases {
            if [alias.binding, alias.target]
                .iter()
                .any(|&binding| self.symbol_request(binding).is_some())
            {
                errors.push(LoweringError::unsupported(
                    Construct::Attribute(UnsupportedAttribute::AliasAssemblerLabel),
                    alias.source,
                ));
                continue;
            }
            let symbol = |unit: &mut Self, binding: usize| {
                if unit.sema.bindings[binding].kind == BindingKind::Function {
                    unit.function(binding).map(Symbol::Function)
                } else {
                    unit.object(binding).map(Symbol::Global)
                }
            };
            let result = symbol(self, alias.binding)
                .and_then(|from| symbol(self, alias.target).map(|to| (from, to)));
            match result {
                | Ok((from, to)) => {
                    let mut attributes = self.module.symbol_attributes(from);
                    attributes.alias = Some(to);
                    self.module.set_symbol_attributes(from, attributes);
                },
                | Err(kind) => errors.push(at(kind, alias.source)),
            }
        }
        for init in self.sema.symbols.init_functions {
            let source = self.sema.bindings[init.binding].name.source_vectors;
            match self.function(init.binding) {
                | Ok(func) => self.module.add_init_function(InitFunction {
                    kind: match init.kind {
                        | InitFunctionKind::Constructor => InitKind::Constructor,
                        | InitFunctionKind::Destructor => InitKind::Destructor,
                    },
                    priority: init.priority,
                    func,
                }),
                | Err(kind) => errors.push(at(kind, source)),
            }
        }
    }
}
