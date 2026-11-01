//! GCC's `scalar_storage_order`, under the GCC identity only: Clang and
//! clang-cl do not know the attribute, and TCC ignores it, so the other
//! identities keep the unknown-attribute warning. GCC stores the scalar
//! members of a structure or union in the order its definition names, by
//! the attribute or by `#pragma scalar_storage_order` in effect where the
//! definition ends; an attribute overrides the pragma. Every bcc target is
//! little-endian, so `little-endian` and `default` are the native order
//! and `big-endian` is refused rather than silently stored the other way.
//!
//! As GCC does, the attribute is an error with a missing or wrong argument,
//! ignored silently on a tag that is only named or declared, ignored with a
//! warning on an enumeration definition and on objects, functions,
//! parameters and members, and applied by a typedef whose type is a
//! structure or union. Two cases are conservative: a structure or union
//! naming both orders is refused if one of them is big-endian, where GCC
//! applies the last; and `[[gnu::scalar_storage_order]]` after a body is
//! refused like the GNU spelling there, where GCC ignores it. In a type
//! name, such as a cast's, it is ignored without GCC's warning.
//! GNU extension: GCC manual, "Common Type Attributes" and "Scalar Storage
//! Order Pragmas".
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Scalar-Storage-Order-Pragmas.html>

use super::{
    super::{
        Analyzer,
        Declarator,
        SemanticErrorKind,
        SpecifierExtension,
        SpecifierExtensionKind,
        types::{
            TagKind,
            TypeId,
            TypeKind,
        },
    },
    written::WrittenAttribute,
};
use crate::{
    configuration::CompilerIdentity,
    translation_phases::{
        SourceVectors,
        parsing::{
            AttributeSpecifier,
            AttributeSyntax,
        },
        pragma_storage_order::StorageOrder,
        preprocessing::{
            StringTokenType,
            TokenType,
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Applies the storage order of a structure or union definition: the
    /// attribute in `attributes`, or else the pragma in effect at output
    /// token `end`, after the body and its attributes.
    pub(in crate::translation_phases::semantic_analysis) fn record_storage_order(
        &mut self,
        attributes: Option<&'tu SpecifierExtension<'tu>>,
        end: usize,
    ) {
        if !self.gcc_identity() {
            return;
        }
        let mut written = None;
        let mut chain = attributes;
        while let Some(item) = chain {
            if let SpecifierExtensionKind::Attributes(specifier) = item.kind {
                written = prefer_big_endian(written, self.written_storage_order(specifier));
            }
            chain = item.next;
        }
        let source = match written {
            | Some((big_endian, source)) => big_endian.then_some(source),
            | None => {
                let pragmas = &self.context.storage_order_pragmas;
                let effective = pragmas.partition_point(|pragma| pragma.position <= end);
                effective
                    .checked_sub(1)
                    .map(|index| pragmas[index])
                    .filter(|pragma| pragma.order == StorageOrder::BigEndian)
                    .map(|pragma| pragma.source)
            },
        };
        if let Some(source) = source {
            self.error(
                SemanticErrorKind::UnsupportedScalarStorageOrder,
                source,
                None,
                None,
            );
        }
    }

    /// Warns, as GCC does, for the attribute on an enumeration definition.
    pub(in crate::translation_phases::semantic_analysis) fn enum_storage_order(
        &mut self,
        mut chain: Option<&'tu SpecifierExtension<'tu>>,
    ) {
        if !self.gcc_identity() {
            return;
        }
        while let Some(item) = chain {
            if let SpecifierExtensionKind::Attributes(specifier) = item.kind {
                self.ignore_storage_order(specifier);
            }
            chain = item.next;
        }
    }

    /// The attribute among the specifiers or on the declarator of a typedef
    /// of type `ty`: a structure or union type takes the order, and any
    /// other type ignores it with GCC's warning.
    pub(in crate::translation_phases::semantic_analysis) fn typedef_storage_order(
        &mut self,
        specifiers: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Declarator<'tu>,
        ty: TypeId,
    ) {
        if !self.gcc_identity() || self.types.unanalyzed(ty) {
            return;
        }
        let record = matches!(
            self.types.nodes[ty.index],
            TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum
        );
        let lists = self.declaration_attribute_lists(specifiers, Some(declarator));
        for &specifier in &lists {
            if !record {
                self.ignore_storage_order(specifier);
            } else if let Some((true, source)) = self.written_storage_order(specifier) {
                self.error(
                    SemanticErrorKind::UnsupportedScalarStorageOrder,
                    source,
                    None,
                    None,
                );
            }
        }
    }

    /// Warns for each valid `scalar_storage_order` in `specifier`, where it
    /// applies to nothing.
    fn ignore_storage_order(&mut self, specifier: &'tu AttributeSpecifier<'tu>) {
        for attribute in self.written_attributes(specifier) {
            if self.is_storage_order(specifier.syntax, attribute)
                && self.storage_order_argument(attribute).is_ok()
            {
                self.ignored(attribute);
            }
        }
    }

    /// Whether `attribute` is GCC's `scalar_storage_order` under the GCC
    /// identity, in the GNU or `gnu::` spelling.
    pub(super) fn is_storage_order(
        &self,
        syntax: AttributeSyntax,
        attribute: WrittenAttribute<'_>,
    ) -> bool {
        self.gcc_identity()
            && attribute.name == "scalar_storage_order"
            && matches!(
                (syntax, attribute.namespace),
                (AttributeSyntax::Gnu, None) | (AttributeSyntax::Standard, Some("gnu"))
            )
    }

    /// Reports GCC's error for a missing or invalid argument.
    pub(super) fn check_storage_order(&mut self, attribute: WrittenAttribute<'_>) {
        if let Err(kind) = self.storage_order_argument(attribute) {
            self.error(kind, attribute.source, None, None);
        }
    }

    /// Whether the valid `scalar_storage_order` attributes of `specifier`
    /// include `big-endian`, and where: the big-endian one, or the last.
    fn written_storage_order(
        &self,
        specifier: &'tu AttributeSpecifier<'tu>,
    ) -> Option<(bool, SourceVectors)> {
        let mut found = None;
        for attribute in self.written_attributes(specifier) {
            if self.is_storage_order(specifier.syntax, attribute)
                && let Ok(order) = self.storage_order_argument(attribute)
            {
                found = prefer_big_endian(
                    found,
                    Some((order == StorageOrder::BigEndian, attribute.source)),
                );
            }
        }
        found
    }

    /// The order one string argument names, which GCC reads after string
    /// concatenation and whatever its encoding prefix.
    pub(super) fn storage_order_argument(
        &self,
        attribute: WrittenAttribute<'_>,
    ) -> Result<StorageOrder, SemanticErrorKind> {
        let arguments = attribute.arguments.unwrap_or_default();
        if arguments.is_empty() || attribute.argument_count(self.context) != 1 {
            return Err(SemanticErrorKind::StorageOrderArgumentCount);
        }
        let mut text = crate::util::bump::ArenaString::new_in(self.scratch);
        for token in arguments {
            let (id, wide) = match token.kind {
                | TokenType::String(StringTokenType::String(id)) => (id, false),
                | TokenType::String(StringTokenType::WideString(id)) => (id, true),
                | TokenType::String(StringTokenType::EncodedString(id, encoding)) => (
                    id,
                    encoding != crate::translation_phases::preprocessing::LiteralEncoding::Utf8,
                ),
                | _ => return Err(SemanticErrorKind::StorageOrderArgument),
            };
            let part = self
                .context
                .literal_text_in(self.scratch, id, wide)
                .ok_or(SemanticErrorKind::StorageOrderArgument)?;
            text.push_str(part);
        }
        match text.as_str() {
            | "big-endian" => Ok(StorageOrder::BigEndian),
            | "little-endian" => Ok(StorageOrder::LittleEndian),
            | _ => Err(SemanticErrorKind::StorageOrderArgument),
        }
    }

    /// Whether the GCC identity itself is in effect, for rules GCC alone
    /// follows (the TCC and MSVC identities share GCC's rule set).
    pub(in crate::translation_phases::semantic_analysis) fn gcc_identity(&self) -> bool {
        self.context.configuration.compiler_identity() == CompilerIdentity::Gcc
    }
}

/// The big-endian one of two written orders, or else the later.
fn prefer_big_endian(
    earlier: Option<(bool, SourceVectors)>,
    later: Option<(bool, SourceVectors)>,
) -> Option<(bool, SourceVectors)> {
    match earlier {
        | Some((true, _)) => earlier,
        | _ => later.or(earlier),
    }
}
