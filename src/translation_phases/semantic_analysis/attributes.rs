//! Phase-7 attribute pass. Each attribute specifier is checked once by name
//! (`catalog.rs`): unknown names warn as Clang's `-Wunknown-attributes`,
//! conventions are checked against the target, dropped meanings warn and
//! unimplementable ones become retained capability refusals. Declarations
//! then apply the attributes that describe their entity (`entities.rs`):
//! weak, alias, section, used, visibility, code alignment, constructors,
//! destructors and cleanup functions, with Clang's applicability
//! diagnostics. Translation-unit completion resolves aliases and registers
//! constructor and destructor definitions; `cleanup` opens a control region
//! after its variable's initializer. Representation attributes (`packed`,
//! `aligned`, `mode`) stay with type construction in `layout.rs`.
//!
//! For `static int n __attribute__((cleanup(drop))) = 1;` in a block, the
//! name pass accepts `cleanup`; the declaration pass reports that `cleanup`
//! only applies to automatic variables and ignores it. For an automatic
//! `int n`, it resolves `drop`, checks that it takes one parameter that
//! `&n` converts to, and opens the cleanup region once `= 1` is analyzed.
//!
//! Read [`Analyzer::check_attribute_list`] and
//! [`Analyzer::declaration_attributes`] first, then `entities.rs`.
//!
//! - `written.rs` splits a specifier into attribute entries;
//! - `catalog.rs` gives every known name its rule;
//! - `entities.rs` applies entity attributes and finalizes them;
//! - `jumps.rs` forbids jumps into cleanup scopes and indirect jumps across
//!   them;
//! - `layout.rs` retains packing, alignment and scalar machine modes;
//! - `transparent.rs` makes qualifying unions transparent.
//!
//! C99: §6.7, p. 97; PDF p. 109 (declarations, extended by GNU attributes);
//! §6.2.2p2, p. 30; PDF p. 42 (linked declarations share one entity).
//! C23: §6.7.13.2p1-3, pp. 142-143; PDF pp. 155-156.
//! GNU extension: GCC manual, "Attribute Syntax".
//! <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>

mod catalog;
mod entities;
mod functions;
mod jumps;
mod layout;
mod mangling;
mod overloads;
mod predicates;
mod transparent;
mod written;

pub(crate) use catalog::clang_attribute_version;
pub(super) use entities::{
    AttributeState,
    Subject,
};
pub(super) use written::known_names;

use super::{
    Analyzer,
    AttributeSpecifier,
    Declarator,
    DirectDeclarator,
    SemanticErrorKind,
    SpecifierExtension,
    SpecifierExtensionKind,
    facts::{
        AttributeRefusal,
        UnsupportedAttribute,
    },
};
use crate::{
    target::Target,
    translation_phases::{
        SourceVectors,
        preprocessing::KeywordTokenType,
    },
    util::bump::ArenaVec,
};

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Checks every attribute name of `specifier` once, wherever it appears:
    /// declarations, type names, pointers, members, tags and statements.
    /// GNU extension: GCC manual, "Attribute Syntax".
    pub(super) fn check_attribute_list(&mut self, specifier: &'tu AttributeSpecifier<'tu>) {
        if specifier.recovered
            || self
                .attributes
                .checked
                .insert(std::ptr::from_ref(specifier).addr(), ())
                .is_some()
        {
            return;
        }
        let windows = matches!(
            self.context.configuration.target(),
            Target::WindowsGnu | Target::WindowsMsvc
        );
        for attribute in self.written_attributes(specifier) {
            let Some(rule) = catalog::rule(
                specifier.syntax,
                attribute.namespace,
                attribute.name,
                self.context.configuration.target(),
            ) else {
                let spelling = match attribute.namespace {
                    | Some(namespace) => crate::diagnostics::format_in!(
                        self.scratch,
                        "{namespace}::{}",
                        attribute.name
                    ),
                    | None => attribute.name,
                };
                let name = self.context.string_cache.intern(spelling);
                self.error(
                    SemanticErrorKind::UnknownAttribute,
                    attribute.source,
                    Some(name),
                    None,
                );
                continue;
            };
            let refusal = match rule {
                | catalog::Rule::Convention(convention) => match convention {
                    | catalog::Convention::Default => None,
                    | catalog::Convention::X86 | catalog::Convention::Pascal => {
                        if !windows || convention == catalog::Convention::Pascal {
                            let name = self.context.string_cache.intern(attribute.name);
                            self.error(
                                SemanticErrorKind::IgnoredConvention,
                                attribute.source,
                                Some(name),
                                None,
                            );
                        }
                        None
                    },
                    | catalog::Convention::Microsoft =>
                        (!windows).then_some(UnsupportedAttribute::CallingConvention),
                    | catalog::Convention::SystemV =>
                        windows.then_some(UnsupportedAttribute::CallingConvention),
                    | catalog::Convention::Other => Some(UnsupportedAttribute::CallingConvention),
                },
                | catalog::Rule::Unsupported(capability) => Some(capability),
                | catalog::Rule::RecordLayout { microsoft } =>
                    (microsoft != windows).then_some(UnsupportedAttribute::RecordLayout),
                | catalog::Rule::Ignored => {
                    let name = self.context.string_cache.intern(attribute.name);
                    self.error(
                        SemanticErrorKind::IgnoredAttribute,
                        attribute.source,
                        Some(name),
                        None,
                    );
                    None
                },
                | _ => None,
            };
            if let Some(capability) = refusal {
                self.refuse(capability, specifier.source_vectors);
            }
        }
    }

    /// The entries of `specifier`, with names copied to semantic scratch so
    /// diagnostics can be reported while visiting them.
    fn written_attributes(
        &self,
        specifier: &AttributeSpecifier<'_>,
    ) -> ArenaVec<'s, written::WrittenAttribute<'s>> {
        let scratch = self.scratch;
        let mut list = ArenaVec::new_in(scratch);
        for attribute in written::written(specifier, self.context) {
            list.push(written::WrittenAttribute {
                namespace: attribute.namespace.map(|n| &*scratch.alloc_str(n)),
                name:      scratch.alloc_str(attribute.name),
                index:     attribute.index,
                arguments: attribute
                    .arguments
                    .map(|tokens| &*scratch.alloc_slice_copy(tokens)),
                source:    attribute.source,
            });
        }
        list
    }

    /// Checks the attribute lists and MSVC modifiers of a specifier chain.
    pub(super) fn check_extension_chain(
        &mut self,
        mut chain: Option<&'tu SpecifierExtension<'tu>>,
    ) {
        while let Some(item) = chain {
            match item.kind {
                | SpecifierExtensionKind::Attributes(specifier) =>
                    self.check_attribute_list(specifier),
                | SpecifierExtensionKind::MsModifier(keyword) =>
                    self.check_ms_modifier(keyword, item.source_vectors),
                | _ => {},
            }
            chain = item.next;
        }
    }

    /// Checks a declarator's own attribute lists, pointer-level lists and
    /// MSVC modifiers, excluding grouped children (visited on their own).
    pub(super) fn check_declarator_attributes(&mut self, declarator: Declarator<'tu>) {
        for level in declarator.pointer.levels {
            self.check_extension_chain(level.attributes);
        }
        for direct in declarator.kind {
            match *direct {
                | DirectDeclarator::Attributes(specifier) => self.check_attribute_list(specifier),
                | DirectDeclarator::MsModifier(keyword, source) =>
                    self.check_ms_modifier(keyword, source),
                | _ => {},
            }
        }
    }

    /// `__vectorcall` is not the x86-64 C convention; Clang leaves the other
    /// MSVC convention keywords at the default there.
    /// MSVC extension: Microsoft Learn, "Argument Passing and Naming
    /// Conventions". <https://learn.microsoft.com/en-us/cpp/cpp/argument-passing-and-naming-conventions>
    fn check_ms_modifier(&mut self, keyword: KeywordTokenType, source: SourceVectors) {
        if keyword == KeywordTokenType::Vectorcall {
            self.refuse(UnsupportedAttribute::CallingConvention, source);
        }
    }

    /// Retains an accepted attribute whose meaning code generation lacks.
    pub(super) fn refuse(&mut self, capability: UnsupportedAttribute, source: SourceVectors) {
        if !self.tainted {
            self.attributes.refusals.push(AttributeRefusal {
                capability,
                source,
                owner: self.functions.reference_function,
            });
        }
    }

    /// Applies the entity attributes of one declaration: those of its
    /// specifiers, its declarator, pointer levels and grouped declarators.
    /// GNU attributes in any of these positions describe the declaration,
    /// as in Clang. C99: §6.7, p. 97; PDF p. 109.
    pub(super) fn declaration_attributes(
        &mut self,
        subject: Subject,
        specifiers: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Option<Declarator<'tu>>,
    ) {
        let mut chain = specifiers;
        while let Some(item) = chain {
            // C11 §6.7.5p6: zero has no effect, including on target storage
            // minima; only nonzero `_Alignas` gives the object its alignment.
            if let SpecifierExtensionKind::Alignment(operand) = item.kind
                && let Subject::Object { binding, .. } = subject
            {
                let alignment = match operand {
                    | super::SyntaxOperand::Expression(expression) => self
                        .expression_info(expression)
                        .integer
                        .and_then(super::Integer::to_u64),
                    | super::SyntaxOperand::Type(_) =>
                        self.types.alignment(self.operand_type(operand)),
                };
                if alignment.is_some_and(|alignment| alignment != 0) {
                    self.declares_alignment(binding);
                }
            }
            chain = item.next;
        }
        let lists = self.declaration_attribute_lists(specifiers, declarator);
        for &specifier in &lists {
            if !specifier.recovered {
                self.entity_attribute_list(subject, specifier);
            }
        }
    }

    /// Source-ordered lists on this declaration, excluding nested function
    /// parameters, whose attributes have their own semantic subject.
    pub(super) fn declaration_attribute_lists(
        &self,
        specifiers: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Option<Declarator<'tu>>,
    ) -> ArenaVec<'s, &'tu AttributeSpecifier<'tu>> {
        let mut lists = ArenaVec::new_in(self.scratch);
        let mut chain = specifiers;
        while let Some(item) = chain {
            if let SpecifierExtensionKind::Attributes(specifier) = item.kind {
                lists.push(specifier);
            }
            chain = item.next;
        }
        // Chains are linked in reverse source order.
        lists.reverse();
        let mut declarators = ArenaVec::new_in(self.scratch);
        declarators.extend(declarator);
        while let Some(declarator) = declarators.pop() {
            for level in declarator.pointer.levels {
                let start = lists.len();
                let mut chain = level.attributes;
                while let Some(item) = chain {
                    if let SpecifierExtensionKind::Attributes(specifier) = item.kind {
                        lists.push(specifier);
                    }
                    chain = item.next;
                }
                lists[start..].reverse();
            }
            for direct in declarator.kind {
                match *direct {
                    | DirectDeclarator::Attributes(specifier) => lists.push(specifier),
                    | DirectDeclarator::Parenthesized(group) => declarators.push(group.declarator),
                    | _ => {},
                }
            }
        }
        lists
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// What a bound declaration declares, for its entity attributes. An
    /// object declaration counts as a definition with an initializer, and at
    /// file scope also without a storage class (an external tentative
    /// definition). GCC and Clang accept `alias` on a `static` tentative
    /// declaration, so it does not count.
    /// C99: §6.7p5, p. 97; PDF p. 109; §6.9.2p1-2, p. 143; PDF p. 155.
    pub(super) fn declaration_subject(
        &self,
        binding: usize,
        declarator: Declarator<'tu>,
        specifiers: super::DeclarationSpecifiers<'tu>,
        initialized: bool,
    ) -> Subject {
        let data = self.bindings[binding];
        match data.kind {
            | super::BindingKind::Function => Subject::Function {
                binding,
                definition: false,
            },
            | super::BindingKind::Object => Subject::Object {
                binding,
                definition: initialized
                    || (self.scopes[data.scope].kind == super::ScopeKind::File
                        && specifiers.storage_class.is_none()),
                declarator: declarator.kind.as_slice().as_ptr().addr(),
            },
            | super::BindingKind::Parameter => Subject::Parameter,
            | super::BindingKind::Typedef | super::BindingKind::Enumerator => Subject::Typedef,
        }
    }
}

/// Whether a declaration's specifiers or declarator carry any attribute
/// list, so that its end must be marked for a possible cleanup region.
pub(super) fn written_on(
    specifiers: Option<&SpecifierExtension<'_>>,
    declarator: Declarator<'_>,
) -> bool {
    specifiers.is_some()
        || declarator
            .pointer
            .levels
            .iter()
            .any(|level| level.attributes.is_some())
        || declarator.kind.iter().any(|direct| {
            matches!(
                direct,
                DirectDeclarator::Attributes(_) | DirectDeclarator::Parenthesized(_)
            )
        })
}
