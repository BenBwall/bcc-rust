//! The C89 defaults C99 removed, which GCC 14 and Clang 16 report as
//! errors in C99 and later and TCC and `cl` accept: declaration specifiers
//! without a type specifier (implicit int) and an identifier list in a
//! function declarator that is no definition. Implicit function
//! declarations are bound where a call names an undeclared identifier
//! (`Analyzer::implicit_function`), and an undeclared identifier-list
//! parameter where its definition's signature is built.
//! C99: Foreword paragraph 5, pp. xii-xiii; PDF pp. 10-11; §6.7.2p2, p. 99;
//! PDF p. 111; §6.7.5.3p3, p. 118; PDF p. 130.

use super::{
    Analyzer,
    SemanticErrorKind,
};
use crate::{
    configuration::{
        CStandard,
        DiagnosticFormat,
        DiagnosticGroup,
        RuleSet,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Declaration,
                DeclarationSpecifiers,
                Declarator,
                DirectDeclarator,
            },
            syntax::{
                FunctionDefinition,
                Identifier,
            },
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Reports a declaration whose specifiers name no type, and each
    /// declarator in it with a nonempty identifier list.
    /// C99: §6.7.2p2, p. 99; PDF p. 111; §6.7.5.3p3, p. 118; PDF p. 130.
    pub(super) fn implicit_declaration_defaults(&mut self, d: &'tu Declaration<'tu>) {
        if d.recovered {
            return;
        }
        if d.declaration_specifiers.implicit_int {
            // GCC reports each declarator, Clang the declaration once.
            let each = self.context.configuration.diagnostic_format() == DiagnosticFormat::Gcc;
            for init in d.init_declarators.as_slice() {
                if let Some(name) = init.declarator.identifier() {
                    self.implicit_int(SemanticErrorKind::ImplicitInt, name);
                    if !each {
                        break;
                    }
                }
            }
        }
        for init in d.init_declarators.as_slice() {
            self.identifier_list_declaration(init.declarator, d.source_vectors);
        }
    }

    /// Reports a function definition whose specifiers name no type.
    /// C99: §6.7.2p2, p. 99; PDF p. 111.
    pub(super) fn implicit_return_type(&mut self, f: &'tu FunctionDefinition<'tu>) {
        if f.declaration_specifiers.implicit_int
            && let Some(name) = f.declarator.identifier()
        {
            self.implicit_int(SemanticErrorKind::ImplicitIntReturn, name);
        }
    }

    /// Reports a prototype parameter whose specifiers name no type
    /// (`int f(register x)`).
    /// C99: §6.7.2p2, p. 99; PDF p. 111.
    pub(super) fn implicit_parameter_type(
        &mut self,
        specifiers: DeclarationSpecifiers<'tu>,
        declarator: Option<Declarator<'tu>>,
    ) {
        if specifiers.implicit_int
            && let Some(name) = declarator.and_then(Declarator::identifier)
        {
            self.implicit_int(SemanticErrorKind::ImplicitInt, name);
        }
    }

    /// Reports implicit int under `-Wimplicit-int`; Clang refuses it in
    /// C23, where no option controls it.
    /// C99: §6.7.2p2, p. 99; PDF p. 111.
    pub(super) fn implicit_int(&mut self, kind: SemanticErrorKind, name: Identifier) {
        if self.context.configuration.standard() >= CStandard::C23
            && self.context.configuration.compiler_identity().rules() == RuleSet::Clang
        {
            self.error(
                SemanticErrorKind::TypeSpecifierRequired,
                name.source_vectors,
                Some(name.name),
                None,
            );
        } else {
            self.group_diagnostic(
                DiagnosticGroup::ImplicitInt,
                kind,
                name.source_vectors,
                Some(name.name),
                None,
            );
        }
    }

    /// Reports a nonempty identifier list in a declarator that declares a
    /// function without defining it: GCC's
    /// `-Wdeclaration-missing-parameter-type`, which it reports at the
    /// declaration; Clang and `cl`, as clang-cl models it, refuse it at
    /// the first parameter. C23 has no identifier lists; the parser
    /// reports them.
    /// C99: §6.7.5.3p3, p. 118; PDF p. 130.
    fn identifier_list_declaration(&mut self, declarator: Declarator<'tu>, site: SourceVectors) {
        let mut declarator = Some(declarator);
        while let Some(current) = declarator.take() {
            for direct in current.kind {
                match *direct {
                    | DirectDeclarator::KAndRStyleFunction { parameters } => {
                        let Some(first) = parameters.as_slice().first() else {
                            continue;
                        };
                        if self.context.configuration.compiler_identity().ms_compatible_rules()
                            == RuleSet::Clang
                        {
                            self.error(
                                SemanticErrorKind::IdentifierListDeclaration,
                                first.source_vectors,
                                None,
                                None,
                            );
                        } else {
                            let site = if self.context.configuration.diagnostic_format()
                                == DiagnosticFormat::Gcc
                            {
                                site
                            } else {
                                first.source_vectors
                            };
                            self.group_diagnostic(
                                DiagnosticGroup::DeclarationMissingParameterType,
                                SemanticErrorKind::DeclarationMissingParameterType,
                                site,
                                None,
                                None,
                            );
                        }
                        return;
                    },
                    | DirectDeclarator::Parenthesized(parenthesized) =>
                        declarator = Some(parenthesized.declarator),
                    | _ => {},
                }
            }
        }
    }
}
