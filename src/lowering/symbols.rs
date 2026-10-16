//! Entity attributes as IR symbol facts. Semantic analysis validated and
//! merged every attribute; this file only maps the retained records onto
//! the IR: weak linkage, section, retention, visibility and code alignment
//! of each declared symbol, aliases of definitions, and the constructor and
//! destructor lists. No attribute token is read here.
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
    ir::{
        Align,
        InitFunction,
        InitKind,
        Linkage as IrLinkage,
        Symbol,
        SymbolAttributes,
        Visibility as IrVisibility,
    },
    translation_phases::semantic_analysis::{
        BindingKind,
        InitFunctionKind,
        Linkage,
        UnsupportedAttribute,
        Visibility,
    },
    util::bump::ArenaVec,
};

impl Unit<'_, '_, '_> {
    /// The IR linkage of the entity `binding` declares: internal for C
    /// internal linkage, weak for a `weak` external entity, else external.
    /// C99: §6.2.2p3-5, pp. 30-31; PDF pp. 42-43.
    pub(super) fn ir_linkage(&self, binding: usize) -> IrLinkage {
        match self.sema.bindings[binding].linkage {
            | Linkage::Internal => IrLinkage::Internal,
            | Linkage::External
                if self
                    .sema
                    .entity_attributes(binding)
                    .is_some_and(|attributes| attributes.weak) =>
                IrLinkage::Weak,
            | Linkage::External | Linkage::None => IrLinkage::External,
        }
    }

    /// Gives a newly declared symbol its entity's section, retention,
    /// visibility and, for a function, code alignment and returns-twice
    /// fact.
    pub(super) fn symbol_attributes(&mut self, binding: usize, symbol: Symbol) {
        let Some(&attributes) = self.sema.entity_attributes(binding) else {
            return;
        };
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
            },
        );
        if let Symbol::Function(func) = symbol
            && attributes.code_alignment > 1
        {
            self.module.function_mut(func).align =
                Align::from_bytes(u64::from(attributes.code_alignment));
        }
        if let Symbol::Function(func) = symbol
            && attributes.returns_twice
        {
            self.module.function_mut(func).returns_twice = true;
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
            let names = [alias.binding, alias.target].map(|binding| self.binding_identity(binding));
            if names.iter().any(|&name| self.names.linked(name).is_some()) {
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
