//! GNU `transparent_union`: a union type attribute that type construction
//! applies once the union it names is complete. The attribute catalog
//! identifies the name (`Rule::TransparentUnion`), so every spelling the
//! name pass accepts counts here too.
//! GNU extension: GCC manual, "Common Type Attributes".
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html>

use super::{
    super::{
        Analyzer,
        AttributeSpecifier,
        Declarator,
        DirectDeclarator,
        SemanticErrorKind,
        SpecifierExtension,
        SpecifierExtensionKind,
        types::{
            TagKind,
            TypeId,
            TypeKind,
        },
    },
    catalog,
    written,
};
use crate::translation_phases::{
    SourceVectors,
    parsing::declaration_syntax::{
        DeclarationSpecifiers,
        TypeSpecifiers,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// The first GNU `transparent_union` attribute in a specifier chain.
    pub(in crate::translation_phases::semantic_analysis) fn transparent_union_request(
        &self,
        mut chain: Option<&'tu SpecifierExtension<'tu>>,
    ) -> Option<SourceVectors> {
        while let Some(item) = chain {
            if let SpecifierExtensionKind::Attributes(a) = item.kind
                && let Some(source) = self.transparent_union_name(a)
            {
                return Some(source);
            }
            chain = item.next;
        }
        None
    }

    /// A `transparent_union` attribute a typedef declaration writes outside
    /// a union definition: among its specifiers, after a union specifier
    /// that only names the union, or after its declarator. Clang applies it
    /// to the union the typedef names.
    pub(in crate::translation_phases::semantic_analysis) fn typedef_transparent_union(
        &self,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Declarator<'tu>,
    ) -> Option<SourceVectors> {
        let named = match spec.type_specifiers {
            | TypeSpecifiers::StructOrUnion(s) if s.struct_declaration_list.is_none() =>
                self.transparent_union_request(s.attributes),
            | _ => None,
        };
        self.transparent_union_request(spec.extensions)
            .or(named)
            .or_else(|| {
                declarator.kind.iter().find_map(|direct| match direct {
                    | DirectDeclarator::Attributes(a) => self.transparent_union_name(a),
                    | _ => None,
                })
            })
    }

    /// Where `a` names `transparent_union` by the catalog's rule, the
    /// position Clang reports.
    fn transparent_union_name(&self, a: &AttributeSpecifier<'tu>) -> Option<SourceVectors> {
        if a.recovered {
            return None;
        }
        written::written(a, self.context)
            .find(|attribute| {
                catalog::rule(a.syntax, attribute.namespace, attribute.name)
                    == Some(catalog::Rule::TransparentUnion)
            })
            .map(|attribute| attribute.source)
    }

    /// Applies `transparent_union`, written at `source`, to the type `ty`.
    /// As in Clang, the attribute is ignored with a warning unless `ty` is a
    /// complete union with at least one member, a first member of neither
    /// floating nor vector type, and every member of the first member's
    /// size and at most its alignment; a member of incomplete type ignores
    /// it silently.
    pub(in crate::translation_phases::semantic_analysis) fn make_transparent_union(
        &mut self,
        ty: TypeId,
        source: SourceVectors,
    ) {
        let id = match self.types.nodes[ty.index] {
            | TypeKind::Tag(id)
                if self.types.tags[id].kind == TagKind::Union
                    && self.types.tags[id].complete.get() =>
                id,
            | _ if self.types.unanalyzed(ty) => return,
            | _ => {
                self.error(
                    SemanticErrorKind::IgnoredTransparentUnion,
                    source,
                    None,
                    None,
                );
                return;
            },
        };
        let tag = self.types.tags[id];
        if tag.tainted.get() {
            return;
        }
        let members = tag.members.get();
        let at = |member: &super::super::types::Member| {
            member
                .name
                .map_or(member.source_vectors, |name| name.source_vectors)
        };
        let violation = if let Some(first) = members.first() {
            let floating = self.arithmetic(first.ty) && self.integer_type(first.ty).is_none();
            if floating || self.vector(first.ty).is_some() {
                Some(at(first))
            } else {
                let Some(expected) = self.types.layout(first.ty) else {
                    return;
                };
                let mut violation = None;
                for member in members {
                    let Some(layout) = self.types.layout(member.ty) else {
                        return;
                    };
                    if layout.size != expected.size || layout.align > expected.align {
                        violation = Some(at(member));
                        break;
                    }
                }
                violation
            }
        } else {
            Some(source)
        };
        if let Some(at) = violation {
            self.error(SemanticErrorKind::IgnoredTransparentUnion, at, None, None);
        } else {
            tag.transparent.set(true);
        }
    }
}
