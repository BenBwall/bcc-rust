//! Final entity and symbol policy. Completion joins retained occurrences,
//! definitions and ABI identities once; consumers keep point-of-use types.
//! GNU assembler labels are decoded here, with capability refusals retained
//! for lowering at the original provenance and emission owner.
//! C99: §6.2.2p2, p. 30; PDF p. 42; §6.9, pp. 140-143; PDF pp. 152-155.

use std::num::NonZeroUsize;

use super::{
    super::{
        Analyzer,
        Binding,
        BindingKind,
        Definition,
        DefinitionKind,
        Inlining,
        Linkage,
        SemanticTranslationUnit,
        TypeId,
        TypeKind,
    },
    EntityId,
    OccurrenceId,
};
use crate::{
    translation_phases::{
        Context,
        SourceVectors,
        parsing::declaration_syntax::{
            Declarator,
            DirectDeclarator,
            Initializer,
        },
        preprocessing::{
            StringTokenType,
            TokenType,
        },
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// Finalized entities, independent of emitted-symbol collisions. Distinct
/// entities can request the same assembler name; lowering checks their IR
/// definitions and still keys private inline bodies by semantic identity.
#[derive(Debug)]
pub(crate) struct EntityFacts<'tu> {
    records:             &'tu mut [EntityRecord<'tu>],
    functions:           &'tu mut [(usize, FunctionPolicy)],
    pub(crate) refusals: &'tu [SymbolRefusal],
}

impl<'tu> EntityFacts<'tu> {
    pub(crate) fn entity(&self, id: EntityId) -> Option<Entity<'tu>> {
        let record = &self.records[record_index(self.records, id)?];
        let cold = record.cold.as_deref();
        Some(Entity {
            identity:      record.identity?,
            abi_name:      record.abi_name,
            ty:            record.ty,
            bindings:      record.bindings,
            occurrences:   record.occurrences,
            definition:    cold.and_then(|c| c.definition).copied(),
            initializer:   cold.and_then(|c| c.initializer),
            symbol:        cold.and_then(|c| c.symbol).copied(),
            function:      cold.and_then(|c| c.function).map(|i| i.get() - 1),
            body:          cold.map_or(BodyEmission::Declaration, |c| c.body),
            end:           cold.map_or(FunctionEnd::Indeterminate, |c| c.end),
            returns_twice: cold.is_some_and(|c| c.returns_twice),
        })
    }

    pub(crate) fn function(&self, binding: usize) -> Option<&FunctionPolicy> {
        let index = self
            .functions
            .binary_search_by_key(&binding, |p| p.0)
            .ok()?;
        Some(&self.functions[index].1)
    }

    #[cfg(test)]
    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        if let Some(index) = record_index(self.records, id) {
            self.records[index].identity = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn remove_function(&mut self, binding: usize) {
        if let Ok(index) = self.functions.binary_search_by_key(&binding, |p| p.0) {
            self.functions[index..].rotate_left(1);
            let length = self.functions.len() - 1;
            self.functions = &mut std::mem::take(&mut self.functions)[..length];
        }
    }

    #[cfg(test)]
    pub(crate) fn clear_initializer(&mut self, id: EntityId) {
        self.records[record_index(self.records, id).unwrap()]
            .cold
            .as_deref_mut()
            .unwrap()
            .initializer = None;
    }

    pub(in crate::translation_phases::semantic_analysis) fn empty(_tu: &'tu Bump) -> Self {
        Self {
            records:   &mut [],
            functions: &mut [],
            refusals:  &[],
        }
    }

    /// Completes policy after inline/DLL attributes, ABI names and definitions.
    /// No IR capabilities or reachability decisions are made here.
    pub(in crate::translation_phases::semantic_analysis) fn retain(
        context: &Context<'tu>,
        sema: &SemanticTranslationUnit<'tu>,
        scratch: &Bump,
        tu: &'tu Bump,
        requests: &[PendingSymbolRequest<'tu>],
    ) -> Self {
        // Sort only policy-bearing IDs: automatic locals must not leave holes
        // in the retained table. Policy tables have their final capacity.
        let count = sema
            .bindings
            .iter()
            .filter(|b| b.needs_entity_policy())
            .count();
        let mut ids = ArenaVec::with_capacity_in(count, scratch);
        ids.extend(
            sema.bindings
                .iter()
                .enumerate()
                .filter(|(_, b)| b.needs_entity_policy())
                .map(|(i, &b)| sema.correspondence.entity_of(i, b)),
        );
        ids.sort_unstable_by_key(|id| id.0);
        ids.dedup();
        let records = tu.alloc_slice_fill_iter(ids.iter().map(|&id| {
            let binding = sema.bindings[id.0];
            EntityRecord {
                id,
                identity: Some(binding.name.name),
                abi_name: binding.name.name,
                ty: binding.ty,
                bindings: &[],
                occurrences: &[],
                cold: None,
            }
        }));
        drop(ids);
        // Count into two words per entity, then reuse them as exclusive ends
        // and finally starts. Reverse filling preserves declaration order.
        let mut offsets = ArenaVec::with_capacity_in(records.len(), scratch);
        offsets.resize(records.len(), [0_usize; 2]);
        for (index, &binding) in sema.bindings.iter().enumerate() {
            if !binding.needs_entity_policy() {
                continue;
            }
            let i = record_index(records, sema.correspondence.entity_of(index, binding)).unwrap();
            if offsets[i][0] == 0 {
                let extension = sema.function_extensions.function(index);
                records[i].identity = Some(extension.map_or(binding.name.name, |e| e.identity));
                records[i].abi_name = extension.map_or(binding.name.name, |e| e.abi_name);
            }
            offsets[i][0] += 1;
            records[i].ty = binding.ty;
        }
        for fact in sema.correspondence.declarations {
            if let Some(i) = fact.entity.and_then(|id| record_index(records, id)) {
                offsets[i][1] += 1;
            }
        }
        let mut totals = [0; 2];
        for offset in &mut *offsets {
            for (total, count) in totals.iter_mut().zip(offset.iter_mut()) {
                *total += *count;
                *count = *total;
            }
        }
        let bindings = tu.alloc_slice_fill_iter(0..totals[0]);
        let occurrences = tu.alloc_slice_fill_iter((0..totals[1]).map(OccurrenceId));
        for (index, &binding) in sema.bindings.iter().enumerate().rev() {
            if !binding.needs_entity_policy() {
                continue;
            }
            let i = record_index(records, sema.correspondence.entity_of(index, binding)).unwrap();
            offsets[i][0] -= 1;
            bindings[offsets[i][0]] = index;
        }
        for fact in sema.correspondence.declarations.iter().rev() {
            if let Some(i) = fact.entity.and_then(|id| record_index(records, id)) {
                offsets[i][1] -= 1;
                occurrences[offsets[i][1]] = fact.id;
            }
        }
        for (i, record) in records.iter_mut().enumerate() {
            let end = offsets.get(i + 1).copied().unwrap_or(totals);
            record.bindings = &bindings[offsets[i][0]..end[0]];
            record.occurrences = &occurrences[offsets[i][1]..end[1]];
        }
        drop(offsets);
        let mut definitions = ArenaVec::with_capacity_in(sema.definitions.len(), scratch);
        definitions.extend(
            sema.definitions
                .iter()
                .enumerate()
                .map(|(i, d)| (d.binding, i, d.kind)),
        );
        definitions.sort_unstable_by_key(|d| (d.0, std::cmp::Reverse(d.1)));
        definitions.dedup_by_key(|d| d.0);
        for definition in sema.definitions {
            let id = sema
                .correspondence
                .entity_of(definition.binding, sema.bindings[definition.binding]);
            let Some(i) = record_index(records, id) else {
                continue;
            };
            records[i].cold(tu).definition = Some(definition);
            records[i].ty = sema.bindings[definition.binding].ty;
        }
        for fact in sema.correspondence.declarations {
            if let Some(i) = fact.entity.and_then(|id| record_index(records, id))
                && let Some(cold) = records[i].cold.as_deref_mut()
                && cold
                    .definition
                    .is_some_and(|d| Some(d.binding) == fact.binding)
            {
                cold.initializer = fact.initializer;
            }
        }
        // No other scratch allocations interleave this loop, so a refusal
        // vector grows in place. Retain only the actual refusals, not a slot
        // for every valid assembler request.
        let mut refusals = ArenaVec::new_in(scratch);
        for request in requests {
            let binding = sema
                .correspondence
                .declaration(request.declarator)
                .and_then(|f| f.binding);
            let owner = request.owner;
            let Some(binding) = binding else {
                refusals.push(SymbolRefusal {
                    owner,
                    source: request.declarator.source_vectors,
                    missing_occurrence: true,
                });
                continue;
            };
            let data = sema.bindings[binding];
            let name = request.name.filter(|_| {
                matches!(data.kind, BindingKind::Object | BindingKind::Function)
                    && (data.linkage != Linkage::None || data.duration.is_static_or_thread())
            });
            let index = record_index(records, sema.correspondence.entity_of(binding, data));
            if let Some(name) = name
                && let Some(index) = index
                && records[index]
                    .cold
                    .as_deref()
                    .and_then(|c| c.symbol)
                    .is_none_or(|previous| previous.name == name)
            {
                let cold = records[index].cold(tu);
                if cold.symbol.is_none() {
                    cold.symbol = Some(tu.alloc(SymbolRequest {
                        name,
                        source: request.source,
                    }));
                }
            } else {
                refusals.push(SymbolRefusal {
                    owner,
                    source: request.source,
                    missing_occurrence: false,
                });
            }
        }
        let mut functions = ArenaVec::with_capacity_in(sema.functions.len(), scratch);
        for (ordinal, function) in sema.functions.iter().enumerate() {
            let Some(binding) = function.binding else {
                continue;
            };
            // Recovered definitions can bind a typedef or another non-entity.
            // Their existing semantic diagnostics already prevent lowering.
            if !sema.bindings[binding].needs_entity_policy() {
                continue;
            }
            let i = record_index(
                records,
                sema.correspondence
                    .entity_of(binding, sema.bindings[binding]),
            )
            .expect("functions have entities");
            let cold = records[i].cold(tu);
            cold.function = NonZeroUsize::new(binding + 1);
            cold.body = if definitions
                .binary_search_by_key(&binding, |d| d.0)
                .is_ok_and(|i| definitions[i].2 == DefinitionKind::Inline)
            {
                let attributes = sema.entity_attributes(binding);
                if attributes.is_some_and(|a| a.weak) {
                    // A weak inline definition is a weak definition, emitted
                    // where it is referenced, as Clang gives it weak linkage
                    // before considering inline-only emission; a strong
                    // definition elsewhere replaces it at link time. GNU
                    // extension: GCC manual, "Common Function Attributes",
                    // weak; C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
                    BodyEmission::Definition
                } else if !function.assembly
                    && attributes.is_some_and(|a| a.inlining == Inlining::Always)
                {
                    BodyEmission::PrivateInline
                } else {
                    BodyEmission::InlineOnly
                }
            } else {
                BodyEmission::Definition
            };
            // Preserve the C-identifier-only rule, including asm renaming.
            // C99 §5.1.2.2.3p1, p. 13; PDF p. 25.
            cold.end = if context.string_cache.at(sema.bindings[binding].name.name) == "main" {
                FunctionEnd::MainZero
            } else {
                FunctionEnd::Indeterminate
            };
            functions.push((
                binding,
                ordinal,
                FunctionPolicy {
                    body: cold.body,
                    end:  cold.end,
                },
            ));
        }
        // As with the old map, the last policy for a binding wins, including
        // recovered duplicate bodies. The ordinal makes unstable sorting safe.
        functions.sort_unstable_by_key(|p| (p.0, std::cmp::Reverse(p.1)));
        functions.dedup_by_key(|p| p.0);
        for record in &mut *records {
            if let Some(cold) = record.cold.as_deref_mut()
                && let Some(definition) = cold.definition
                && let Ok(i) = functions.binary_search_by_key(&definition.binding, |p| p.0)
            {
                cold.function = NonZeroUsize::new(definition.binding + 1);
                cold.body = functions[i].2.body;
                cold.end = functions[i].2.end;
            }
            let returns_twice = record.bindings.iter().any(|&binding| {
                sema.entity_attributes(binding)
                    .is_some_and(|a| a.returns_twice)
            }) || record
                .cold
                .as_deref()
                .and_then(|c| c.symbol)
                .is_some_and(|r| crate::builtins::returns_twice(r.name))
                && record.bindings.iter().any(|&binding| {
                    sema.bindings[binding].kind == BindingKind::Function
                        && sema.bindings[binding].linkage != Linkage::None
                });
            if returns_twice {
                record.cold(tu).returns_twice = true;
            }
        }
        Self {
            records,
            functions: tu.alloc_slice_fill_iter(functions.iter().map(|p| (p.0, p.2))),
            refusals: tu.alloc_slice_copy(&refusals),
        }
    }
}

/// Dense, sorted entity storage. Most declarations have no definition, body,
/// assembler request or exceptional return policy, so those fields live cold.
#[derive(Debug)]
struct EntityRecord<'tu> {
    id:          EntityId,
    identity:    Option<StringCacheId>,
    abi_name:    StringCacheId,
    ty:          TypeId,
    bindings:    &'tu [usize],
    occurrences: &'tu [OccurrenceId],
    cold:        Option<&'tu mut EntityCold<'tu>>,
}

impl<'tu> EntityRecord<'tu> {
    fn cold(&mut self, tu: &'tu Bump) -> &mut EntityCold<'tu> {
        self.cold.get_or_insert_with(|| {
            tu.alloc(EntityCold {
                definition:    None,
                initializer:   None,
                symbol:        None,
                function:      None,
                body:          BodyEmission::Declaration,
                end:           FunctionEnd::Indeterminate,
                returns_twice: false,
            })
        })
    }
}

#[derive(Debug)]
struct EntityCold<'tu> {
    definition:    Option<&'tu Definition>,
    initializer:   Option<&'tu Initializer<'tu>>,
    symbol:        Option<&'tu SymbolRequest<'tu>>,
    // Binding indices fit an allocated slice, so index + 1 cannot overflow.
    function:      Option<NonZeroUsize>,
    body:          BodyEmission,
    end:           FunctionEnd,
    returns_twice: bool,
}

fn record_index(records: &[EntityRecord<'_>], id: EntityId) -> Option<usize> {
    // Declaration-only units commonly have no binding gaps. Keep their lookup
    // direct without retaining an index slot for every automatic local.
    if records.get(id.0).is_some_and(|r| r.id == id) {
        Some(id.0)
    } else {
        records.binary_search_by_key(&id.0, |r| r.id.0).ok()
    }
}

impl Binding {
    /// Entities that require symbol completion or static object association.
    /// C99: §6.2.2p2, p. 30; PDF p. 42; §6.2.4p3, p. 32; PDF p. 44.
    pub(crate) fn needs_entity_policy(self) -> bool {
        self.kind == BindingKind::Function
            || self.kind == BindingKind::Object
                && (self.linkage != Linkage::None || self.duration.is_static_or_thread())
    }
}

/// Policy of a particular body. GNU inline replacement bodies share an
/// entity but retain different emission decisions.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionPolicy {
    pub(crate) body: BodyEmission,
    pub(crate) end:  FunctionEnd,
}

/// A value view of one entity's dense hot record and optional cold state.
/// Initializer placement is supplied by M8;
/// this record only retains the association with its defining occurrence.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "The entity view also exposes retained inspection facts."
    )
)]
pub(crate) struct Entity<'tu> {
    pub(crate) identity:      StringCacheId,
    pub(crate) abi_name:      StringCacheId,
    pub(crate) bindings:      &'tu [usize],
    pub(crate) occurrences:   &'tu [OccurrenceId],
    pub(crate) definition:    Option<Definition>,
    pub(crate) initializer:   Option<&'tu Initializer<'tu>>,
    pub(crate) ty:            TypeId,
    pub(crate) symbol:        Option<SymbolRequest<'tu>>,
    /// A known definition, including inline bodies that supply no external
    /// definition.
    pub(crate) function:      Option<usize>,
    pub(crate) body:          BodyEmission,
    pub(crate) end:           FunctionEnd,
    pub(crate) returns_twice: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SymbolRequest<'tu> {
    pub(crate) name:   &'tu str,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Valid requests retain provenance for downstream symbol diagnostics."
        )
    )]
    pub(crate) source: SourceVectors,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SymbolRefusal {
    pub(crate) owner:              Option<usize>,
    pub(crate) source:             SourceVectors,
    pub(crate) missing_occurrence: bool,
}

/// Language permission to supply a body; module reachability remains
/// downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyEmission {
    Declaration,
    Definition,
    InlineOnly,
    PrivateInline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionEnd {
    MainZero,
    Indeterminate,
}

impl<'tu> SemanticTranslationUnit<'tu> {
    pub(crate) fn entity(&self, binding: usize) -> Option<Entity<'tu>> {
        let data = *self.bindings.get(binding)?;
        self.entities
            .entity(self.correspondence.entity_of(binding, data))
    }

    pub(crate) fn function_policy(&self, binding: usize) -> Option<&FunctionPolicy> {
        self.entities.function(binding)
    }

    /// The assembler name a binding with an entity is emitted under: its
    /// assembler label, otherwise its ABI name (with any MSVC decoration).
    /// GNU extension: assembler labels extend C99 §6.2.2p2, p. 30; PDF p. 42.
    pub(crate) fn symbol_name<'a>(
        &self,
        binding: usize,
        context: &'a Context<'tu>,
    ) -> Option<&'a str> {
        let entity = self.entity(binding)?;
        Some(entity.symbol.map_or_else(
            || context.string_cache.at(entity.abi_name),
            |request| request.name,
        ))
    }

    /// Open calls use the occurrence's type, never the final composite type.
    /// C99: §6.5.2.2p6, pp. 71-72; PDF pp. 83-84.
    pub(crate) fn unprototyped_external(&self, binding: usize) -> Option<bool> {
        Some(
            matches!(
                self.types.kind(self.bindings.get(binding)?.ty),
                TypeKind::Function {
                    prototype: false,
                    ..
                }
            ) && self.entity(binding)?.function.is_none(),
        )
    }
}

/// Traversal-time request before its declarator has a bound occurrence.
/// The decoded name lives in the TU arena; this continuation lives in scratch.
#[derive(Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) struct PendingSymbolRequest<'tu> {
    pub(in crate::translation_phases::semantic_analysis) owner: Option<usize>,
    declarator: Declarator<'tu>,
    name: Option<&'tu str>,
    source: SourceVectors,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Decode at the existing type traversal, then associate the completed
    /// binding at TU finalization. Invalid requests stay capability refusals.
    /// GNU extension: assembler labels extend C99 §6.2.2p2, p. 30; PDF p. 42.
    pub(in crate::translation_phases::semantic_analysis) fn record_symbol_requests(
        &mut self,
        declarator: Declarator<'tu>,
    ) {
        for direct in declarator.kind {
            let DirectDeclarator::AsmLabel(label) = *direct else {
                continue;
            };
            let name = if !label.recovered
                && let Some(TokenType::String(StringTokenType::String(id))) =
                    label.template.map(|t| t.kind)
            {
                self.context
                    .literal_text_in(self.scratch, id, false)
                    .filter(|n| !n.is_empty() && !n.contains('\0') && !n.starts_with('\u{1}'))
                    .map(|n| &*self.types.tu.alloc_str(n))
            } else {
                None
            };
            self.symbol_requests.push(PendingSymbolRequest {
                owner: self.functions.reference_function,
                declarator,
                name,
                source: label.source_vectors,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Write,
        path::Path,
    };

    use super::*;

    // Host layouts; the arenas already require a 64-bit host.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn retained_entity_record_sizes() {
        assert_eq!(size_of::<EntityRecord<'_>>(), 72);
        assert_eq!(size_of::<EntityCold<'_>>(), 40);
        assert_eq!(size_of::<FunctionPolicy>(), 2);
        assert_eq!(size_of::<(usize, FunctionPolicy)>(), 16);
        assert_eq!(size_of::<SymbolRequest<'_>>(), 24);
    }

    /// Plain declarations stay in the dense hot table. Only definitions,
    /// assembler requests and returns-twice policy allocate cold records, so
    /// a path that makes every record cold fails here rather than only
    /// showing up as retained memory.
    #[test]
    #[expect(
        clippy::disallowed_types,
        reason = "The test builds its generated input and expected names with std types."
    )]
    fn plain_declarations_do_not_allocate_cold_records() {
        const PLAIN: usize = 500;
        let mut source = String::new();
        for i in 0..PLAIN {
            writeln!(source, "extern int x{i}; int p{i}(int); extern int x{i};").unwrap();
        }
        // Each of these declarations needs exactly one cold record.
        source.push_str(
            "int defined = 1;\nint tentative;\nstatic int internal;\nint labelled(void) \
             __asm__(\"renamed\");\n__attribute__((returns_twice)) int twice(void);\nint \
             body(void) { return x0 + p0(defined); }\n",
        );
        let tu = Bump::new();
        let mut context = Context::with_configuration(
            &tu,
            crate::configuration::CompilerConfiguration::default(),
        );
        let unit = crate::pipeline::parse_translation_unit(
            &mut context,
            Path::new("<cold>"),
            tu.alloc_str(&source),
            crate::headers::HeaderSearch::default(),
        );
        let sema = super::super::super::analyze(&mut context, &unit);
        assert_eq!(sema.errors, 0);
        let records = &*sema.entities.records;
        let name = |r: &EntityRecord<'_>| context.string_cache.at(sema.bindings[r.id.0].name.name);
        // Predefined bindings may add records, so count the fixture's own.
        let plain = records
            .iter()
            .filter(|r| {
                let n = name(r);
                (n.starts_with('x') || n.starts_with('p')) && n[1..].parse::<usize>().is_ok()
            })
            .collect::<Vec<_>>();
        assert_eq!(plain.len(), 2 * PLAIN);
        assert!(plain.iter().all(|r| r.cold.is_none()));
        let cold: Vec<_> = records
            .iter()
            .filter(|r| r.cold.is_some())
            .map(name)
            .collect();
        assert_eq!(
            cold,
            [
                "defined",
                "tentative",
                "internal",
                "labelled",
                "twice",
                "body",
                // The defined static that `body` implicitly declares.
                "__func__"
            ]
        );
    }
}
