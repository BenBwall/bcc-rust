//! The signature rules GCC and Clang each apply to a function named `main`,
//! and to an external object of that name. The standard fixes two portable
//! forms and leaves the rest to the implementation, so which forms a program
//! may use is the compiler's answer, not C's: GNU make's three-parameter
//! `main` and `int main(int, const char **)` are accepted silently by both.
//!
//! - GCC checks a definition's written signature, under `-Wmain`, which is off
//!   until `-Wall` or `-Wpedantic` enables it. It holds no form to be an error.
//! - Clang checks every declaration whatever the options, with warnings for
//!   some forms and errors for others (`Sema::CheckMain`).
//! - TCC checks nothing (`tccgen.c` has no rule for the name). `cl`'s rules are
//!   not modeled, so that identity stays silent too.
//!
//! A freestanding startup function's name and type are the implementation's,
//! so neither compiler checks one, except that GCC honors an explicit
//! `-Wmain`.
//! C99: §5.1.2.2.1p1, p. 12; PDF p. 24; §5.1.2.1p1, p. 11; PDF p. 23.

use super::{
    Analyzer,
    BindingKind,
    Linkage,
    SemanticErrorKind,
    scopes::Namespace,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::{
        CompilerIdentity,
        DiagnosticGroup,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                DeclarationSpecifiers,
                TypeQualifiers,
            },
            syntax::{
                FunctionDefinition,
                Identifier,
                StorageClass,
            },
        },
    },
};

/// One complaint about `main`, in GCC's or Clang's terms: a compiler's rules
/// differ in more than their words, so the two sets stay apart.
/// C99: §5.1.2.2.1p1, p. 12; PDF p. 24.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MainRule {
    GccReturnType,
    GccAtomicReturn,
    /// The parameter's spelled type is the diagnostic's name.
    GccAtomicParameter,
    GccFirstParameter,
    GccSecondParameter,
    GccThirdParameter,
    GccParameterCount,
    GccVariadic,
    GccStatic,
    GccObject,
    GccInline,
    GccNoreturn,
    /// In a GNU mode, an extension; otherwise [`Self::ClangMustReturnInt`].
    ClangReturnType,
    ClangMustReturnInt,
    ClangStatic,
    ClangInline,
    ClangNoreturn,
    ClangVariadic,
    ClangOneParameter,
    ClangSurplusParameters(u32),
    ClangFirstParameter,
    ClangSecondParameter,
    ClangThirdParameter,
    ClangObject,
}

impl MainRule {
    /// The compiler's words, except that [`Self::GccAtomicParameter`] and
    /// [`Self::ClangSurplusParameters`] have their operand added.
    pub(super) const fn words(self) -> &'static str {
        match self {
            | Self::GccReturnType | Self::ClangReturnType => "return type of 'main' is not 'int'",
            | Self::GccAtomicReturn => "'_Atomic'-qualified return type of 'main'",
            | Self::GccAtomicParameter => "'_Atomic'-qualified parameter type of 'main'",
            | Self::GccFirstParameter => "first argument of 'main' should be 'int'",
            | Self::GccSecondParameter => "second argument of 'main' should be 'char **'",
            | Self::GccThirdParameter => "third argument of 'main' should probably be 'char **'",
            | Self::GccParameterCount => "'main' takes only zero or two arguments",
            | Self::GccVariadic => "'main' declared as variadic function",
            | Self::GccStatic => "'main' is normally a non-static function",
            | Self::GccObject => "'main' is usually a function",
            | Self::GccInline => "cannot inline function 'main'",
            | Self::GccNoreturn => "'main' declared '_Noreturn'",
            | Self::ClangMustReturnInt => "'main' must return 'int'",
            | Self::ClangStatic => "'main' should not be declared static",
            | Self::ClangInline => "'main' is not allowed to be declared inline",
            | Self::ClangNoreturn => "'main' is not allowed to be declared _Noreturn",
            | Self::ClangVariadic => "'main' is not allowed to be declared variadic",
            | Self::ClangOneParameter => "only one parameter on 'main' declaration",
            | Self::ClangSurplusParameters(_) =>
                "too many parameters for 'main': must be 0, 2, or 3",
            | Self::ClangFirstParameter =>
                "first parameter of 'main' (argument count) must be of type 'int'",
            | Self::ClangSecondParameter =>
                "second parameter of 'main' (argument array) must be of type 'char **'",
            | Self::ClangThirdParameter =>
                "third parameter of 'main' (environment) must be of type 'char **'",
            | Self::ClangObject =>
                "variable named 'main' with external linkage has undefined behavior",
        }
    }

    /// The warning group that controls the rule; `None` for an error no
    /// option controls.
    pub(super) const fn group(self) -> Option<DiagnosticGroup> {
        match self {
            | Self::GccReturnType
            | Self::GccAtomicReturn
            | Self::GccAtomicParameter
            | Self::GccFirstParameter
            | Self::GccSecondParameter
            | Self::GccThirdParameter
            | Self::GccParameterCount
            | Self::GccVariadic
            | Self::GccStatic
            | Self::GccObject
            | Self::ClangStatic
            | Self::ClangOneParameter
            | Self::ClangObject => Some(DiagnosticGroup::Main),
            | Self::GccInline | Self::GccNoreturn => Some(DiagnosticGroup::MainSpecifier),
            | Self::ClangNoreturn | Self::ClangVariadic => Some(DiagnosticGroup::MainExtension),
            | Self::ClangReturnType => Some(DiagnosticGroup::MainReturnType),
            | Self::ClangMustReturnInt
            | Self::ClangInline
            | Self::ClangSurplusParameters(_)
            | Self::ClangFirstParameter
            | Self::ClangSecondParameter
            | Self::ClangThirdParameter => None,
        }
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks a declaration that binds the ordinary identifier `name`, which
    /// Clang's rules read on each declaration of `main` and GCC's only for
    /// its specifiers and for an object.
    /// C99: §5.1.2.2.1p1, p. 12; PDF p. 24.
    pub(super) fn check_main_declaration(
        &mut self,
        name: Identifier,
        spec: DeclarationSpecifiers<'tu>,
        ty: TypeId,
        kind: BindingKind,
        linkage: Linkage,
    ) {
        if !self.is_main(name) {
            return;
        }
        let identity = self.context.configuration.compiler_identity();
        let hosted = self.context.configuration.hosted();
        let specifiers = spec.source_vectors;
        match (identity, kind) {
            | (CompilerIdentity::Gcc, BindingKind::Object) if linkage == Linkage::External =>
                if self.gcc_checks_main() {
                    self.main_rule(MainRule::GccObject, name.source_vectors);
                },
            | (CompilerIdentity::Gcc, BindingKind::Function) if hosted => {
                if spec.function_specifiers.is_inline {
                    self.main_rule(MainRule::GccInline, name.source_vectors);
                }
                if spec.function_specifiers.is_noreturn {
                    self.main_rule(MainRule::GccNoreturn, name.source_vectors);
                }
            },
            | (CompilerIdentity::Clang | CompilerIdentity::ClangCl, BindingKind::Object)
                if hosted && linkage == Linkage::External =>
                self.main_rule(MainRule::ClangObject, specifiers),
            | (CompilerIdentity::Clang | CompilerIdentity::ClangCl, BindingKind::Function)
                if hosted =>
                self.clang_main_declaration(name, spec, ty),
            | _ => {},
        }
    }

    /// Clang's `Sema::CheckMain`, for one declaration.
    fn clang_main_declaration(
        &mut self,
        name: Identifier,
        spec: DeclarationSpecifiers<'tu>,
        ty: TypeId,
    ) {
        let specifiers = spec.source_vectors;
        if spec.storage_class == Some(StorageClass::Static) {
            self.main_rule(MainRule::ClangStatic, specifiers);
        }
        if spec.function_specifiers.is_inline {
            self.main_rule(MainRule::ClangInline, specifiers);
        }
        if spec.function_specifiers.is_noreturn {
            self.main_rule(MainRule::ClangNoreturn, specifiers);
        }
        let TypeKind::Function { result, .. } = self.types.nodes[ty.index] else {
            return;
        };
        if self.types.unanalyzed(ty) {
            return;
        }
        let int = self.types.scalar(Scalar::Int);
        let mut invalid = false;
        if self.context.configuration.gnu_extensions() {
            if result.unqualified() != int {
                self.main_rule(MainRule::ClangReturnType, specifiers);
            }
        } else if result != int {
            self.main_rule(MainRule::ClangMustReturnInt, specifiers);
            invalid = true;
        }
        self.clang_main_parameters(name.source_vectors, ty, invalid);
    }

    /// The parameter half of Clang's rules, which an old-style definition
    /// also runs once its parameters have types. A declaration with no
    /// prototype counts as having no parameters.
    fn clang_main_parameters(&mut self, site: SourceVectors, ty: TypeId, mut invalid: bool) {
        let TypeKind::Function {
            parameters,
            prototype,
            variadic,
            ..
        } = self.types.nodes[ty.index]
        else {
            return;
        };
        if !prototype && parameters.is_empty() {
            return;
        }
        if variadic {
            self.main_rule(MainRule::ClangVariadic, site);
        }
        let mut count = parameters.len();
        if count > 3 {
            let surplus = u32::try_from(count).unwrap_or(u32::MAX);
            self.main_rule(MainRule::ClangSurplusParameters(surplus), site);
            invalid = true;
            count = 3;
        }
        let int = self.types.scalar(Scalar::Int);
        for (position, &parameter) in parameters[..count].iter().enumerate() {
            let fault = match position {
                | 0 if parameter.unqualified() == int => None,
                | 0 => Some(MainRule::ClangFirstParameter),
                | 1 if self.clang_argument_array(parameter) => None,
                | 1 => Some(MainRule::ClangSecondParameter),
                | _ if self.clang_argument_array(parameter) => None,
                | _ => Some(MainRule::ClangThirdParameter),
            };
            if let Some(fault) = fault {
                self.main_rule(fault, site);
                invalid = true;
            }
        }
        if count == 1 && !invalid {
            self.main_rule(MainRule::ClangOneParameter, site);
        }
    }

    /// Whether Clang takes `ty` for `char **`: `const` may qualify any level,
    /// while another qualifier may only be on the parameter itself, and only
    /// when the rest is exactly `char **`.
    fn clang_argument_array(&mut self, ty: TypeId) -> bool {
        let char = self.types.scalar(Scalar::Char);
        let TypeKind::Pointer(pointer) = self.types.nodes[ty.index] else {
            return false;
        };
        let TypeKind::Pointer(target) = self.types.nodes[pointer.index] else {
            return false;
        };
        if target.unqualified() != char {
            return false;
        }
        let inner = pointer.qualifiers | target.qualifiers;
        (inner.is_empty()) || ((inner | ty.qualifiers) & !TypeQualifiers::CONST).is_empty()
    }

    /// Clang's parameter rules for the old-style definition `ty`, whose
    /// parameters' types come after its declarator.
    pub(super) fn check_main_old_parameters(
        &mut self,
        function: &'tu FunctionDefinition<'tu>,
        ty: TypeId,
    ) {
        if matches!(
            self.context.configuration.compiler_identity(),
            CompilerIdentity::Clang | CompilerIdentity::ClangCl
        ) && self.context.configuration.hosted()
            && let Some(name) = function.declarator.identifier()
            && self.is_file_scope_main(name)
            && !self.types.unanalyzed(ty)
        {
            self.clang_main_parameters(name.source_vectors, ty, false);
        }
    }

    /// GCC's rules for a definition: its return type, its written
    /// parameters (an old-style definition writes none) and its linkage.
    /// C99: §5.1.2.2.1p1, p. 12; PDF p. 24.
    pub(super) fn check_main_definition(&mut self, f: &'tu FunctionDefinition<'tu>, ty: TypeId) {
        if self.context.configuration.compiler_identity() != CompilerIdentity::Gcc
            || !self.gcc_checks_main()
        {
            return;
        }
        let Some(name) = f.declarator.identifier() else {
            return;
        };
        if !self.is_file_scope_main(name) || self.types.unanalyzed(ty) {
            return;
        }
        let TypeKind::Function {
            result,
            parameters,
            prototype,
            variadic,
        } = self.types.nodes[ty.index]
        else {
            return;
        };
        let parameters = if prototype { parameters } else { &[] };
        let site = name.source_vectors;
        let int = self.types.scalar(Scalar::Int);
        if self.without_atomic(result).unqualified() != int {
            self.main_rule(MainRule::GccReturnType, site);
        } else if matches!(self.types.nodes[result.index], TypeKind::Atomic(_)) {
            self.main_rule(MainRule::GccAtomicReturn, site);
        }
        for (position, &parameter) in parameters.iter().enumerate() {
            self.gcc_main_atomic_parameter(parameter, site);
            let fault = match position {
                | 0 if self.without_atomic(parameter).unqualified() == int => None,
                | 0 => Some(MainRule::GccFirstParameter),
                | 1 if self.gcc_argument_array(parameter) => None,
                | 1 => Some(MainRule::GccSecondParameter),
                | 2 if self.gcc_argument_array(parameter) => None,
                | 2 => Some(MainRule::GccThirdParameter),
                | _ => None,
            };
            if let Some(fault) = fault {
                self.main_rule(fault, site);
            }
        }
        if !parameters.is_empty() && !matches!(parameters.len(), 2 | 3) {
            self.main_rule(MainRule::GccParameterCount, site);
        }
        if variadic {
            self.main_rule(MainRule::GccVariadic, site);
        }
        if self
            .lookup(Namespace::Ordinary, name.name)
            .is_some_and(|entry| self.bindings[entry.binding].linkage != Linkage::External)
        {
            self.main_rule(MainRule::GccStatic, site);
        }
    }

    /// GCC's warning about each `_Atomic` level of a parameter's type.
    fn gcc_main_atomic_parameter(&mut self, parameter: TypeId, site: SourceVectors) {
        let mut ty = parameter;
        loop {
            match self.types.nodes[ty.index] {
                | TypeKind::Atomic(_) => {
                    let name = self
                        .type_spelling(parameter, true)
                        .map(|text| self.context.string_cache.intern(text));
                    self.group_diagnostic(
                        DiagnosticGroup::Main,
                        SemanticErrorKind::Main(MainRule::GccAtomicParameter),
                        site,
                        name,
                        None,
                    );
                    ty = self.without_atomic(ty);
                },
                | TypeKind::Pointer(target) => ty = target,
                | _ => return,
            }
        }
    }

    /// Whether GCC takes `ty` for `char **`, any level qualified.
    fn gcc_argument_array(&mut self, ty: TypeId) -> bool {
        let char = self.types.scalar(Scalar::Char);
        let ty = self.without_atomic(ty);
        let TypeKind::Pointer(pointer) = self.types.nodes[ty.index] else {
            return false;
        };
        let pointer = self.without_atomic(pointer);
        let TypeKind::Pointer(target) = self.types.nodes[pointer.index] else {
            return false;
        };
        self.without_atomic(target).unqualified() == char
    }

    fn without_atomic(&self, ty: TypeId) -> TypeId {
        match self.types.nodes[ty.index] {
            | TypeKind::Atomic(inner) => inner,
            | _ => ty,
        }
    }

    /// `-Wall` and `-Wpedantic` hold `-Wmain` only for a hosted program; an
    /// explicit `-Wmain` holds it for any.
    fn gcc_checks_main(&self) -> bool {
        let configuration = self.context.configuration;
        configuration.hosted() || configuration.diagnostic_requested(DiagnosticGroup::Main)
    }

    fn is_main(&self, name: Identifier) -> bool {
        self.context.string_cache.at(name.name) == "main"
    }

    /// Whether `name` is `main` in a definition at file scope: `self.scope`
    /// is the definition's own scope. A GNU nested function named `main` is
    /// no program's entry point.
    fn is_file_scope_main(&self, name: Identifier) -> bool {
        self.is_main(name)
            && !self.scopes[self.scope]
                .parent
                .is_some_and(|scope| self.scopes[scope].kind != super::ScopeKind::File)
    }

    fn main_rule(&mut self, rule: MainRule, site: SourceVectors) {
        let kind = SemanticErrorKind::Main(rule);
        match rule.group() {
            | Some(group) => self.group_diagnostic(group, kind, site, None, None),
            | None => self.error(kind, site, None, None),
        }
    }
}
