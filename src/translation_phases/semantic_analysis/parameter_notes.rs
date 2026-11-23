//! Clang's `note: passing argument to parameter 'p' here`, which follows
//! each diagnostic of an argument's conversion to its parameter in a call
//! that names a function declared with a prototype
//! (`InitializationSequence::PrintInitLocationNote` in `SemaInit.cpp`;
//! `note_parameter_named_here` and `note_parameter_here` in
//! `DiagnosticSemaKinds.td`).
//!
//! [`Analyzer::record_parameter_sites`] keeps where each function
//! declaration declares its parameters, and
//! [`Analyzer::with_argument_parameter`] notes the parameter on what an
//! argument's conversion reports.
//!
//! C99: §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84 (arguments converted as if
//! by assignment to their parameters).

use super::{
    Analyzer,
    ParameterNote,
    SemanticErrorKind,
    functions::function_derivation,
    types::TypeKind,
};
use crate::{
    configuration::{
        DiagnosticGroup,
        RuleSet,
    },
    translation_phases::{
        Context,
        SourceVectors,
        parsing::declaration_syntax::{
            Declarator,
            DirectDeclarator,
        },
    },
    util::bump::{
        ArenaMap,
        Bump,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Keeps where the function declaration bound as `index` by `declarator`
    /// declares its parameters, when it has a prototype: each parameter's
    /// name, or for one without a name where Clang's parser places the name
    /// its abstract declarator leaves out ([`unnamed_site`]); a function
    /// type from a typedef declares them all at the declarator's name. A
    /// definition with an identifier list has none Clang notes.
    pub(super) fn record_parameter_sites(&mut self, index: usize, declarator: Declarator<'tu>) {
        if self.context.configuration.compiler_identity().rules() != RuleSet::Clang {
            return;
        }
        let TypeKind::Function {
            prototype: true, ..
        } = self.types.nodes[self.bindings[index].ty.index]
        else {
            return;
        };
        let sites = match function_derivation(declarator, self.scratch) {
            | Some(DirectDeclarator::Function { parameter_list, .. }) => Sites::Each(
                self.scratch
                    .alloc_slice_fill_iter(parameter_list.iter().map(|parameter| {
                        match parameter.declarator.and_then(Declarator::identifier) {
                            | Some(name) => ParameterNote {
                                site: name.source_vectors,
                                name: Some(name.name),
                            },
                            | None => {
                                let after = Context::token_after(
                                    declarator.source_vectors,
                                    parameter.source_vectors,
                                )
                                .unwrap_or_else(|| {
                                    Context::last_source_vector(parameter.source_vectors)
                                });
                                ParameterNote {
                                    site: parameter
                                        .declarator
                                        .map_or(after, |abstract_| unnamed_site(abstract_, after)),
                                    name: None,
                                }
                            },
                        }
                    })),
            ),
            | Some(DirectDeclarator::KAndRStyleFunction { .. }) => return,
            | _ => match declarator.identifier() {
                | Some(name) => Sites::Declarator(name.source_vectors),
                | None => return,
            },
        };
        _ = self.parameter_notes.sites.insert(index, sites);
    }

    /// Runs `convert`, the conversion of argument `position` (from 0) of a
    /// call to the function `function` names, with what it reports noting
    /// the parameter.
    pub(super) fn with_argument_parameter<R>(
        &mut self,
        function: Option<usize>,
        position: usize,
        convert: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let note = function
            .and_then(|index| self.parameter_notes.sites.get(&index))
            .and_then(|sites| match *sites {
                | Sites::Each(parameters) => parameters.get(position).copied(),
                | Sites::Declarator(site) => Some(ParameterNote { site, name: None }),
            });
        let outer = std::mem::replace(&mut self.parameter_notes.argument, note);
        let result = convert(self);
        self.parameter_notes.argument = outer;
        result
    }

    /// The note for `kind` reported now, at its place in a file: Clang
    /// notes what `DiagnoseAssignmentResult` reports of the conversion and
    /// an argument it cannot convert, not the value warnings of
    /// `CheckImplicitConversion`.
    pub(super) fn argument_parameter_note(
        &mut self,
        kind: SemanticErrorKind,
    ) -> Option<ParameterNote> {
        let note = self.parameter_notes.argument?;
        if !matches!(
            kind,
            SemanticErrorKind::InvalidArgumentType
                | SemanticErrorKind::PointerConstraint(
                    DiagnosticGroup::IntConversion
                        | DiagnosticGroup::IncompatiblePointerTypes
                        | DiagnosticGroup::IncompatibleFunctionPointerTypes
                        | DiagnosticGroup::PointerSign
                        | DiagnosticGroup::PointerQualifiers
                        | DiagnosticGroup::DiscardedArrayQualifiers
                )
        ) {
            return None;
        }
        Some(ParameterNote {
            site: self.context.file_location(note.site),
            name: note.name,
        })
    }
}

/// Where Clang's parser places the name that the abstract declarator
/// `declarator` of an unnamed parameter leaves out, which is the place of
/// the parameter (`Declarator::SetIdentifier(nullptr, Tok.getLocation())`
/// in `Parser::ParseDirectDeclarator` and `Parser::ParseParenDeclarator`,
/// `ParseDecl.cpp`): past the pointers, with their qualifiers and
/// attributes, at the `[` of an array, at the token after the `(` of a
/// function's parameters, within parentheses where the declarator inside
/// places it, and otherwise at `after`, the token after the declarator.
/// C99: §6.7.6p1-2, p. 122; PDF p. 134 (abstract declarators).
fn unnamed_site(mut declarator: Declarator<'_>, mut after: SourceVectors) -> SourceVectors {
    loop {
        // The tokens before the direct abstract declarator: each `*`, its
        // qualifiers as written, repeats included, and attributes, and the
        // attributes and Microsoft modifiers among them.
        let mut prefix = 0;
        for level in declarator.pointer.levels {
            prefix += 1 + u32::from(level.written_qualifiers);
            let mut attributes = level.attributes;
            while let Some(attribute) = attributes {
                prefix += attribute.source_vectors.length();
                attributes = attribute.next;
            }
        }
        let mut direct = None;
        for part in declarator.kind {
            match *part {
                | DirectDeclarator::Attributes(attributes) =>
                    prefix += attributes.source_vectors.length(),
                | DirectDeclarator::MsModifier(_, source_vectors) =>
                    prefix += source_vectors.length(),
                | _ => {
                    direct = Some(*part);
                    break;
                },
            }
        }
        let at = |index| {
            Some(Context::source_vector_at(declarator.source_vectors, index))
                .filter(|site| site.length() != 0)
                .unwrap_or(after)
        };
        match direct {
            | Some(DirectDeclarator::Parenthesized(inner)) => {
                after = Context::token_after(
                    declarator.source_vectors,
                    inner.declarator.source_vectors,
                )
                .unwrap_or(after);
                declarator = inner.declarator;
            },
            | Some(DirectDeclarator::Array { .. }) => return at(prefix),
            | Some(DirectDeclarator::Function { parameter_list, .. }) =>
                return parameter_list.first().map_or_else(
                    || at(prefix + 1),
                    |first| Context::first_token_vector(first.source_vectors),
                ),
            // `()` before C23, which has no parameters.
            | Some(DirectDeclarator::KAndRStyleFunction { .. }) => return at(prefix + 1),
            | _ => return after,
        }
    }
}

/// Where a function declaration declares its parameters.
#[derive(Clone, Copy)]
enum Sites<'s> {
    Each(&'s [ParameterNote]),
    /// All at the declarator's name.
    Declarator(SourceVectors),
}

pub(super) struct State<'s> {
    /// By the binding of each function declaration with a prototype.
    sites:    ArenaMap<'s, usize, Sites<'s>>,
    /// The parameter of the argument being converted.
    argument: Option<ParameterNote>,
}

impl<'s> State<'s> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            sites:    ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            argument: None,
        }
    }
}
