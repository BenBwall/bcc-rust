//! GNU `transparent_union`: a union type attribute that type construction
//! applies once the union it names is complete. The attribute catalog
//! identifies the name (`Rule::TransparentUnion`), so every spelling the
//! name pass accepts counts here too. Written anywhere else it is ignored
//! with Clang's warnings: on a union specifier that does not define the
//! union, on another tag, among the specifiers of a declaration that
//! declares only a tag, and on an object, function, parameter or member
//! (`entities.rs`).
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
        StructOrUnion,
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
    /// a union definition: among its specifiers, including after a union
    /// specifier that only names the union, or after its declarator. Clang
    /// applies it to the union the typedef names. Within a union specifier
    /// that does not define the union it applies to nothing
    /// ([`Self::inapplicable_transparent_union`]).
    pub(in crate::translation_phases::semantic_analysis) fn typedef_transparent_union(
        &self,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Declarator<'tu>,
    ) -> Option<SourceVectors> {
        self.transparent_union_request(spec.extensions).or_else(|| {
            declarator.kind.iter().find_map(|direct| match direct {
                | DirectDeclarator::Attributes(a) => self.transparent_union_name(a),
                | _ => None,
            })
        })
    }

    /// Warns as Clang does for a `transparent_union` attribute within the
    /// specifier of a tag of `kind` that is not a union definition: a union
    /// specifier that only names or declares the union, or a structure or
    /// enumeration.
    pub(in crate::translation_phases::semantic_analysis) fn inapplicable_transparent_union(
        &mut self,
        kind: TagKind,
        attributes: Option<&'tu SpecifierExtension<'tu>>,
    ) {
        if let Some(source) = self.transparent_union_request(attributes) {
            let kind = if kind == TagKind::Union {
                SemanticErrorKind::TransparentUnionNotDefinition
            } else {
                SemanticErrorKind::TransparentUnionNotUnion
            };
            self.error(kind, source, None, None);
        }
    }

    /// Warns as Clang does for a `transparent_union` attribute among the
    /// specifiers of a declaration that declares only a tag, where it
    /// applies to no declarator and to no type.
    /// GNU extension: GCC manual, "Attribute Syntax".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
    pub(in crate::translation_phases::semantic_analysis) fn misplaced_transparent_union(
        &mut self,
        spec: DeclarationSpecifiers<'tu>,
    ) {
        let kind = match spec.type_specifiers {
            | TypeSpecifiers::StructOrUnion(s) if s.struct_or_union == StructOrUnion::Struct =>
                TagKind::Struct,
            | TypeSpecifiers::StructOrUnion(_) => TagKind::Union,
            | TypeSpecifiers::Enum(_) => TagKind::Enum,
            | _ => return,
        };
        if let Some(source) = self.transparent_union_request(spec.extensions) {
            self.error(
                SemanticErrorKind::TransparentUnionPlacement(kind),
                source,
                None,
                None,
            );
        }
    }

    /// Where `a` names `transparent_union` by the catalog's rule, the
    /// position Clang reports.
    fn transparent_union_name(&self, a: &AttributeSpecifier<'tu>) -> Option<SourceVectors> {
        if a.recovered {
            return None;
        }
        written::written(a, self.context)
            .find(|attribute| {
                catalog::rule(
                    a.syntax,
                    attribute.namespace,
                    attribute.name,
                    self.context.configuration.target(),
                ) == Some(catalog::Rule::TransparentUnion)
            })
            .map(|attribute| attribute.source)
    }

    /// Applies `transparent_union`, written at `source`, to the type `ty`.
    /// As in Clang, the attribute is ignored with a warning unless `ty` is a
    /// complete union with at least one member, a first member of neither
    /// floating nor vector type, and every member of the first member's
    /// size and at most its alignment; a member of incomplete type ignores
    /// it silently. A type other than a union, or an incomplete union, has
    /// its own warning.
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
            | TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Union => {
                self.error(
                    SemanticErrorKind::TransparentUnionNotDefinition,
                    source,
                    None,
                    None,
                );
                return;
            },
            | _ => {
                self.error(
                    SemanticErrorKind::TransparentUnionNotUnion,
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
