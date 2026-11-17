//! Clang's `-Wdeprecated-non-prototype`: a function declarator without a
//! prototype whose meaning C23 changes. C23 makes `()` declare a function
//! of no parameters and removes identifier lists, so Clang warns before
//! C23 about a definition with an identifier list, a declaration without
//! a prototype that another one with parameters contradicts, and a call
//! with arguments to a function it knows no prototype for.
//!
//! [`Analyzer::compare_prototypes`] runs as each function declarator is
//! bound, [`Analyzer::definition_declared`] once a definition's declarator
//! is, [`Analyzer::identifier_list_definition`] once its body is, and
//! [`Analyzer::call_without_prototype`] for each call.
//! Clang's `MergeFunctionDecl` and `ActOnFinishFunctionBody`
//! (`SemaDecl.cpp`) and `BuildResolvedCallExpr` (`SemaExpr.cpp`) report
//! them; `warn_non_prototype_changes_behavior` and
//! `warn_strict_uses_without_prototype` in `DiagnosticSemaKinds.td` word
//! them.
//!
//! C99: §6.11.6-§6.11.7, p. 163; PDF p. 175 (obsolescent features);
//! §6.5.2.2p6, p. 71; PDF p. 83 (calls without a prototype).

use super::{
    Analyzer,
    BindingKind,
    DeclarationSpecifiers,
    Expression,
    FunctionDefinition,
    Linkage,
    Namespace,
    ReportOrder,
    SemanticErrorKind,
    TypeSpecifiers,
    types::{
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::{
        CStandard,
        DiagnosticGroup,
        RuleSet,
    },
    translation_phases::{
        Context,
        SourceVectors,
    },
    util::{
        bump::{
            ArenaMap,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Compares the function declaration just bound as `index`, of the
    /// written type `written`, with the earlier declaration `previous` it
    /// redeclares, as Clang's `MergeFunctionDecl` does: when one of them
    /// has a prototype with parameters and the other has none, the one
    /// without is reported, and the prototype noted unless the one without
    /// is a definition. A declaration of a builtin is neither reported nor
    /// noted, and only the earlier one can be: Clang passes the builtin on
    /// to the later one after comparing them (`SemaDecl.cpp`,
    /// `MergeFunctionDecl`: `WithoutProto->getBuiltinID() == 0`). Clang
    /// reports a declaration it compares later at the place of the later
    /// one.
    pub(super) fn compare_prototypes(
        &mut self,
        index: usize,
        written: TypeId,
        specified: bool,
        previous: Option<usize>,
        bound: bool,
    ) {
        if !self.reports_non_prototypes() {
            return;
        }
        let mut new = self.declared_function(written);
        new.written |= specified;
        let file_scope = self.scope == 0;
        let name = self.bindings[index].name;
        let old = previous.filter(|_| bound).and_then(|old_index| {
            Some((old_index, *self.prototypes.declarations.get(&old_index)?))
        });
        if bound {
            // What Clang passes on from the declaration this one
            // redeclares once it has compared them, and compares the next
            // one with: whether it declares a builtin, and the prototype a
            // declaration without one takes.
            let mut declared = new;
            declared.builtin = self.declares_builtin(index, old.map(|(_, old)| old));
            if !new.written
                && !new.definition
                && let TypeKind::Function {
                    prototype: true,
                    parameters,
                    ..
                } = self.types.nodes[self.bindings[index].ty.index]
            {
                declared.parameters = parameters.len();
            }
            _ = self.prototypes.declarations.insert(index, declared);
        }
        let possible = self.prototypes.file_scope.get(&name.name).copied();
        if bound && file_scope {
            _ = self.prototypes.file_scope.insert(name.name, index);
        }
        if let Some(definition) = self.prototypes.definition.as_mut() {
            definition.possible = possible.filter(|_| bound);
            definition.written = new.written;
            definition.index = bound.then_some(index);
        }
        let Some((old_index, old)) = old else {
            return;
        };
        if old.written == new.written || self.overloadable(index) || self.overloadable(old_index) {
            return;
        }
        // A definition without a prototype whose parameters an earlier
        // prototype does not count is Clang's `conflicting types` error,
        // which it reports instead.
        if let TypeKind::Function {
            parameters,
            prototype: true,
            ..
        } = self.types.nodes[self.bindings[old_index].ty.index]
            && new.definition
            && !new.written
            && parameters.len() != new.parameters
        {
            return;
        }
        let ((with, with_index), (without, without_index)) = if new.written {
            ((new, index), (old, old_index))
        } else {
            ((old, old_index), (new, index))
        };
        if with.parameters == 0 || without.builtin {
            return;
        }
        let words = if without.parameters != 0 {
            not_supported(without.definition)
        } else {
            conflicting(
                without.definition,
                without_index == old_index,
                with.definition,
            )
        };
        // Clang notes a prototype only when it is the cause, and never one
        // that declares a builtin.
        let note = (!without.definition && !with.builtin)
            .then(|| self.bindings[with_index].name.source_vectors);
        let site = self.bindings[without_index].name.source_vectors;
        self.with_report_order(ReportOrder::After(name.source_vectors), |analyzer| {
            analyzer.non_prototype_warning(site, words, note);
        });
    }

    /// Ends the declarator of the definition `f`, of the function `name`:
    /// one without a prototype is the one later calls are checked against,
    /// and one with an identifier list of names is held for
    /// [`Self::identifier_list_definition`].
    pub(super) fn definition_declared(
        &mut self,
        f: &'tu FunctionDefinition<'tu>,
        name: SourceVectors,
    ) {
        let Some(definition) = self.prototypes.definition.take() else {
            return;
        };
        if !self.reports_non_prototypes() || definition.written {
            return;
        }
        if let Some(index) = definition.index {
            let name = self.bindings[index].name.name;
            _ = self.prototypes.defined.insert(name, definition.identifiers);
        }
        if definition.identifiers != 0 {
            self.prototypes.finishing = Some(Finishing {
                definition: std::ptr::from_ref(f).addr(),
                name,
                end: self
                    .context
                    .file_location(Context::last_source_vector(f.body.source_vectors)),
                possible: definition.possible,
            });
        }
    }

    /// The definition `f`, whose parameters' types its earlier declaration
    /// does not match, redeclares nothing: Clang finishes it as a
    /// definition without an earlier declaration.
    pub(super) fn incompatible_definition(&mut self, f: &'tu FunctionDefinition<'tu>) {
        if let Some(finishing) = self.prototypes.finishing.as_mut()
            && finishing.definition == std::ptr::from_ref(f).addr()
        {
            finishing.possible = None;
        }
    }

    /// Clang's `ActOnFinishFunctionBody` for the definition being finished,
    /// when it has an identifier list of names: it is reported unless the
    /// latest earlier file-scope declaration has a prototype, and that
    /// declaration, when it has none, too. Clang reports both at the
    /// definition's closing brace, after what it reports in the body and
    /// before its analyses of the finished body.
    pub(super) fn identifier_list_definition(&mut self, f: &'tu FunctionDefinition<'tu>) {
        let Some(finishing) = self
            .prototypes
            .finishing
            .take_if(|finishing| finishing.definition == std::ptr::from_ref(f).addr())
        else {
            return;
        };
        let possible = finishing
            .possible
            .map(|index| (index, self.prototypes.declarations[&index]));
        if possible.is_some_and(|(_, declared)| declared.written) {
            return;
        }
        self.with_report_order(ReportOrder::After(finishing.end), |analyzer| {
            analyzer.non_prototype_warning(finishing.name, not_supported(true), None);
            if let Some((index, _)) = possible {
                let site = analyzer.bindings[index].name.source_vectors;
                analyzer.non_prototype_warning(site, conflicting(false, true, true), None);
            }
        });
    }

    /// Clang's warnings for the call `e` with `arguments` to a function of
    /// type `callee` without a prototype, which `named` names when the
    /// call names a function, as `BuildResolvedCallExpr` reports them.
    /// When the function's definition is known and has another number of
    /// parameters, the call has too many or too few arguments, at its `)`.
    /// A call with arguments is deprecated, at its `(`, unless the function
    /// is an implicit declaration, one where the warning is off, or a
    /// library builtin's, which Clang gives the builtin's prototype, or a
    /// definition with an identifier list gave the function parameters,
    /// which Clang takes as a prototype. Clang words that one when it
    /// reports what follows the `)`.
    pub(super) fn call_without_prototype(
        &mut self,
        e: &'tu Expression<'tu>,
        callee: TypeId,
        named: Option<(usize, SourceVectors)>,
        arguments: usize,
    ) {
        let TypeKind::Function {
            parameters,
            prototype: false,
            ..
        } = self.types.nodes[callee.index]
        else {
            return;
        };
        if !self.reports_non_prototypes() {
            return;
        }
        let delimiters = e.operator_source_vectors.unwrap_or(e.source_vectors);
        let close = Context::last_source_vector(delimiters);
        let function =
            named.filter(|&(index, _)| self.bindings[index].kind == BindingKind::Function);
        if let Some((index, designator)) = function
            && let Some(&defined) = self.prototypes.defined.get(&self.bindings[index].name.name)
            && defined != arguments
        {
            let name = self.context.string_cache.at(self.bindings[index].name.name);
            let few = if arguments < defined { "few" } else { "many" };
            let words = crate::diagnostics::format_in!(
                self.scratch,
                "too {few} arguments in call to '{name}'"
            );
            let name = self.context.string_cache.intern(words);
            self.with_highlights([designator, SourceVectors::default()], |analyzer| {
                analyzer.group_diagnostic(
                    DiagnosticGroup::CallArgumentCount,
                    SemanticErrorKind::CallArgumentCount,
                    close,
                    Some(name),
                    None,
                );
            });
        }
        if arguments == 0 || !parameters.is_empty() {
            return;
        }
        if let Some((index, _)) = function {
            let binding = self.bindings[index];
            let declared = binding.name.source_vectors;
            if binding.origin != super::BindingOrigin::User
                || self.overloadable(index)
                || self
                    .context
                    .group_severity_read_at(
                        DiagnosticGroup::DeprecatedNonPrototype,
                        declared,
                        declared,
                    )
                    .is_none()
                || self
                    .implicit_library_type(binding.name)
                    .is_some_and(|(builtin, _)| self.types.composite(builtin, binding.ty).is_some())
            {
                return;
            }
        }
        let callee = match function {
            | Some((index, _)) => {
                let name = self.context.string_cache.at(self.bindings[index].name.name);
                crate::diagnostics::format_in!(self.scratch, "'{name}'")
            },
            | None => "a function",
        };
        let words = crate::diagnostics::format_in!(
            self.scratch,
            "passing arguments to {callee} without a prototype is deprecated in all versions of C \
             and is not supported in C23"
        );
        let open = Context::first_token_vector(delimiters);
        self.with_report_order(ReportOrder::After(close), |analyzer| {
            analyzer.non_prototype_warning(open, words, None);
        });
    }

    /// Whether the declaration specifiers `spec` name a typedef of a
    /// function type with a prototype, which Clang takes as the prototype
    /// of the function declared, whatever its declarator derives from that
    /// type (`hasWrittenPrototype` in `Sema::ActOnFunctionDeclarator`), so
    /// `F *f(a) int a; {}` declares one when `F` is `int (int)`.
    pub(super) fn specifies_prototype(&self, spec: DeclarationSpecifiers<'tu>) -> bool {
        let TypeSpecifiers::TypedefName(name) = spec.type_specifiers else {
            return false;
        };
        self.lookup(Namespace::Ordinary, name.name)
            .is_some_and(|entry| {
                matches!(
                    self.types.nodes[self.bindings[entry.binding].ty.index],
                    TypeKind::Function {
                        prototype: true,
                        ..
                    }
                )
            })
    }

    /// Whether the function declaration `index`, which redeclares `old`,
    /// declares a builtin, as Clang's `getBuiltinID` tells: the first
    /// declaration of a builtin's name does when its type is compatible
    /// with the builtin's, and a later one when the one it redeclares
    /// does. A declaration with internal linkage never does: Clang's
    /// `ActOnFunctionDeclarator` gives the builtin only to a declaration
    /// with C language linkage, and `getBuiltinID` none to a static one.
    /// C99: §6.2.7p2, p. 40; PDF p. 52 (compatible types).
    fn declares_builtin(&mut self, index: usize, old: Option<Declared>) -> bool {
        let binding = self.bindings[index];
        if binding.linkage != Linkage::External {
            return false;
        }
        match old {
            | Some(old) => old.builtin,
            | None => self
                .declared_builtin_type(binding.name)
                .is_some_and(|builtin| self.types.composite(builtin, binding.ty).is_some()),
        }
    }

    /// Whether the function declaration `index` is one of Clang's
    /// overloads, which it never merges.
    fn overloadable(&self, index: usize) -> bool {
        self.function_extensions
            .function(index)
            .is_some_and(|extension| extension.overloadable)
    }

    fn non_prototype_warning(
        &mut self,
        site: SourceVectors,
        words: &str,
        note: Option<SourceVectors>,
    ) {
        let name = self.context.string_cache.intern(words);
        self.group_diagnostic(
            DiagnosticGroup::DeprecatedNonPrototype,
            SemanticErrorKind::DeprecatedNonPrototype,
            site,
            Some(name),
            note,
        );
    }

    /// Clang's rules before C23, which has no function declarator without
    /// a prototype.
    fn reports_non_prototypes(&self) -> bool {
        let configuration = self.context.configuration;
        configuration.compiler_identity().rules() == RuleSet::Clang
            && configuration.standard() < CStandard::C23
            && !self.tainted
    }

    /// What Clang compares of a declaration of the written type `ty`:
    /// whether it has a prototype, and its parameters: a prototype's, or
    /// the names of the identifier list of the definition being declared.
    fn declared_function(&self, ty: TypeId) -> Declared {
        let (written, parameters) = match self.types.nodes[ty.index] {
            | TypeKind::Function {
                prototype: true,
                parameters,
                ..
            } => (true, parameters.len()),
            | _ => (
                false,
                self.prototypes
                    .definition
                    .map_or(0, |definition| definition.identifiers),
            ),
        };
        Declared {
            written,
            parameters,
            definition: self.prototypes.definition.is_some(),
            builtin: false,
        }
    }
}

/// Clang's words for a declarator without a prototype that has
/// parameters, which C23 cannot take.
fn not_supported(definition: bool) -> &'static str {
    if definition {
        "a function definition without a prototype is deprecated in all versions of C and is not \
         supported in C23"
    } else {
        "a function declaration without a prototype is deprecated in all versions of C and is not \
         supported in C23"
    }
}

/// Clang's words for one without parameters, which C23 takes as a
/// prototype of none, conflicting with a `subsequent` or previous
/// declaration or `definition` that has parameters.
fn conflicting(definition: bool, subsequent: bool, other_definition: bool) -> &'static str {
    match (definition, subsequent, other_definition) {
        | (false, false, false) =>
            "a function declaration without a prototype is deprecated in all versions of C and is \
             treated as a zero-parameter prototype in C23, conflicting with a previous declaration",
        | (false, false, true) =>
            "a function declaration without a prototype is deprecated in all versions of C and is \
             treated as a zero-parameter prototype in C23, conflicting with a previous definition",
        | (false, true, false) =>
            "a function declaration without a prototype is deprecated in all versions of C and is \
             treated as a zero-parameter prototype in C23, conflicting with a subsequent \
             declaration",
        | (false, true, true) =>
            "a function declaration without a prototype is deprecated in all versions of C and is \
             treated as a zero-parameter prototype in C23, conflicting with a subsequent definition",
        | (true, false, false) =>
            "a function definition without a prototype is deprecated in all versions of C and is \
             treated as a zero-parameter prototype in C23, conflicting with a previous declaration",
        | (true, false, true) =>
            "a function definition without a prototype is deprecated in all versions of C and is \
             treated as a zero-parameter prototype in C23, conflicting with a previous definition",
        | (true, true, false) =>
            "a function definition without a prototype is deprecated in all versions of C and is \
             treated as a zero-parameter prototype in C23, conflicting with a subsequent \
             declaration",
        | (true, true, true) =>
            "a function definition without a prototype is deprecated in all versions of C and is \
             treated as a zero-parameter prototype in C23, conflicting with a subsequent definition",
    }
}

/// The function declarations Clang compares, by binding.
pub(super) struct State<'s> {
    declarations:          ArenaMap<'s, usize, Declared>,
    /// The latest file-scope declaration of each function name: Clang's
    /// possible prototype for a definition (`FindPossiblePrototype`).
    file_scope:            ArenaMap<'s, StringCacheId, usize>,
    /// The definition whose declarator is being bound.
    pub(super) definition: Option<Definition>,
    /// The number of parameters of each function defined without a
    /// prototype, which Clang checks calls against (`hasBody`).
    defined:               ArenaMap<'s, StringCacheId, usize>,
    /// The definition with an identifier list whose body is analyzed.
    finishing:             Option<Finishing>,
}

/// A definition with an identifier list, reported once its body is
/// finished.
#[derive(Clone, Copy)]
struct Finishing {
    definition: usize,
    name:       SourceVectors,
    /// Its closing brace.
    end:        SourceVectors,
    possible:   Option<usize>,
}

impl<'s> State<'s> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            declarations: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            file_scope:   ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            definition:   None,
            defined:      ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            finishing:    None,
        }
    }
}

/// A definition being declared: the names of its identifier list, and,
/// once its declarator is bound, the latest earlier file-scope declaration.
#[derive(Clone, Copy)]
pub(super) struct Definition {
    identifiers: usize,
    possible:    Option<usize>,
    /// Its declarator has a prototype.
    written:     bool,
    /// Its binding, once bound.
    index:       Option<usize>,
}

impl Definition {
    pub(super) const fn new(identifiers: usize) -> Self {
        Self {
            identifiers,
            possible: None,
            written: false,
            index: None,
        }
    }
}

/// One function declaration as Clang compares it.
#[derive(Clone, Copy)]
struct Declared {
    /// It has a prototype: `hasWrittenPrototype`.
    written:    bool,
    /// Its parameters: `getNumParams`.
    parameters: usize,
    definition: bool,
    /// It declares a builtin: `getBuiltinID`.
    builtin:    bool,
}
