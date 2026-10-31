//! Phase-7 type inference: a C23 `auto` or GNU `__auto_type` object takes
//! the type of its initializer.
//!
//! The initializer is analyzed before the object is bound, so a use of the
//! name inside it finds an outer declaration, as GCC's `__auto_type` scope
//! rule wants, or none. The work then reads the analyzed expression's type
//! after lvalue conversion and decay, applies the written qualifiers, and
//! binds and initializes the object as an ordinary declaration would,
//! without analyzing the initializer a second time.
//!
//! For `__auto_type x = (s);`, [`Analyzer::infer_declarator`] checks the
//! declaration's form and queues the initializer; [`Analyzer::infer_type`]
//! then takes the type of `(s)` and queues the declarator and binding; and
//! [`Analyzer::infer_initialization`] queues the initialization of the
//! bound object.
//!
//! The forms C23 leaves implementation-defined follow each identity: GCC
//! accepts one declarator that is a plain identifier, and rejects a tag
//! declared inside a C23 `auto` initializer; Clang deduces each declarator
//! separately, through pointer and function declarators, and keeps an
//! atomic operand's `_Atomic`.
//!
//! C23 (N3220): §6.7.10 paragraphs 1-2, pp. 134-135; PDF pp. 147-148;
//! §6.7.1 paragraph 12, p. 98; PDF p. 111; §6.2.1 paragraph 7, p. 36;
//! PDF p. 49. C99: lvalue conversion and decay §6.3.2.1 paragraphs 2-4,
//! p. 46; PDF p. 58. GNU extension: GCC manual, "Referring to a Type with
//! `typeof`" (`__auto_type`).

use super::{
    Analyzer,
    Duration,
    ExtendedType,
    ScopeKind,
    SemanticErrorKind,
    Work,
    functions::{
        FunctionWork,
        ReferenceMark,
    },
    scopes::Namespace,
    types::{
        TypeId,
        TypeKind,
        UnmodeledType,
    },
};
use crate::{
    configuration::RuleSet,
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Declaration,
                DeclarationSpecifiers,
                Declarator,
                DirectDeclarator,
                InitializerType,
                TypeSpecifiers,
            },
            syntax::{
                Identifier,
                StorageClass,
            },
        },
    },
    util::{
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the form of one inferred init-declarator and queues the
    /// analysis of its initializer, its typing and its binding. A rejected
    /// form binds its name with an unanalyzed type, so later uses do not
    /// cascade.
    /// C23 (N3220): §6.7.10 paragraph 2, pp. 134-135; PDF pp. 147-148.
    pub(super) fn infer_declarator(&mut self, d: &'tu Declaration<'tu>, index: usize) {
        let init = d.init_declarators[index];
        let spec = inferred_specifiers(d.declaration_specifiers);
        let keyword = inference_keyword(spec.type_specifiers).expect("an inferred declaration");
        let name = init.declarator.identifier();
        if let Some((fault, previous)) = self.declaration_fault(d, index, keyword) {
            self.error(
                SemanticErrorKind::InvalidInference(fault),
                init.declarator.source_vectors,
                name.map(|name| name.name),
                previous,
            );
            self.work.push(Work::DeclarationDeclarator(
                init.declarator,
                spec,
                self.types.unknown(),
                init.initializer.is_some(),
            ));
            return;
        }
        let initializer = init.initializer.expect("checked by declaration_fault");
        // As `Work::InitializeDeclaration` does for an ordinary object,
        // whose duration the binding tells; here the specifiers do.
        let automatic = self.scopes[self.scope].kind != ScopeKind::File
            && matches!(
                spec.storage_class,
                None | Some(StorageClass::Auto | StorageClass::Register)
            )
            && !self.thread_storage(spec.extensions);
        let labeled =
            automatic && self.functions.dead && self.initializer_contains_label(initializer);
        let omitted = automatic && self.functions.dead && !labeled;
        if labeled {
            self.functions.dead = false;
        }
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreElided(
                self.functions.elided,
            )));
        self.functions.elided |= omitted;
        self.work
            .push(Work::RestoreReferenceOwner(self.functions.initializer));
        self.work.push(Work::RestoreInitializerValue(
            self.functions.initializer_value,
        ));
        self.functions.initializer_value = None;
        self.functions.initializer = None;
        self.work.push(Work::InferInitialization(
            d,
            index,
            omitted,
            self.reference_mark(),
        ));
        self.work.push(Work::InferType(d, index));
        self.work.push(Work::EndInference);
        let file = self.scopes[self.scope].kind == ScopeKind::File;
        let duration = if self.thread_storage(spec.extensions) {
            Duration::Thread
        } else if automatic {
            Duration::Automatic
        } else {
            Duration::Static
        };
        self.inferences.push(Inference {
            name: name.map(|name| name.name),
            bindings: self.bindings.len(),
            keyword,
            duration,
            local: !file && spec.storage_class != Some(StorageClass::Extern),
        });
        self.work.push(Work::Initializer(initializer));
    }

    /// The fault in an inferred declaration that is visible before its
    /// initializer is analyzed, with the earlier declaration it conflicts
    /// with.
    fn declaration_fault(
        &mut self,
        d: &'tu Declaration<'tu>,
        index: usize,
        keyword: InferenceKeyword,
    ) -> Option<(InferenceFault, Option<SourceVectors>)> {
        let rules = self.context.configuration.compiler_identity().rules();
        let init = d.init_declarators[index];
        let spec = d.declaration_specifiers;
        if spec.storage_class == Some(StorageClass::Typedef) || self.old_parameter_mode {
            return Some((InferenceFault::NotAllowedHere, None));
        }
        // GCC stops reading the declaration at its first fault, or at the
        // comma after its first declarator (`c_parser_declaration_or_fndef`),
        // so it reports the second declarator only after a valid first one.
        if rules == RuleSet::Gcc && index > 0 {
            let first = d.init_declarators[0];
            let mut derivations = ArenaVec::new_in(self.scratch);
            derivations_of(first.declarator, &mut derivations);
            let first_valid = derivations.is_empty()
                && first.initializer.is_some_and(|initializer| {
                    matches!(initializer.kind, InitializerType::AssignmentExpression(_))
                });
            return (index == 1 && first_valid)
                .then_some((InferenceFault::MultipleDeclarators, None));
        }
        let mut derivations = ArenaVec::new_in(self.scratch);
        derivations_of(init.declarator, &mut derivations);
        match (rules, derivations.last()) {
            | (_, None) => {},
            | (RuleSet::Gcc, Some(_)) => return Some((InferenceFault::NotPlainIdentifier, None)),
            | (RuleSet::Clang, Some(Derivation::Array)) =>
                return Some((InferenceFault::ArrayDeclarator, None)),
            | (RuleSet::Clang, Some(Derivation::Function)) =>
                return Some((InferenceFault::FunctionDeclarator, None)),
            | (RuleSet::Clang, Some(Derivation::Pointer)) => {},
        }
        let Some(initializer) = init.initializer else {
            return Some((InferenceFault::MissingInitializer, None));
        };
        if !matches!(initializer.kind, InitializerType::AssignmentExpression(_)) {
            return Some((InferenceFault::InitializerList, None));
        }
        // C23 §6.7.1p12 leaves a redeclaration in the same scope
        // implementation-defined; GCC and Clang reject one by C23 `auto`
        // and judge `__auto_type` as any redeclaration.
        if keyword == InferenceKeyword::Auto
            && let Some(name) = init.declarator.identifier()
            && let Some(entry) = self.lookup(Namespace::Ordinary, name.name)
            && entry.scope == self.scope
        {
            return Some((
                InferenceFault::Redeclared,
                Some(entry.name.source_vectors),
            ));
        }
        None
    }

    /// Types an inferred declarator from its analyzed initializer and
    /// queues its qualification and binding.
    /// C23 (N3220): §6.7.10 paragraph 2, pp. 134-135; PDF pp. 147-148: the
    /// type after lvalue, array to pointer or function to pointer
    /// conversion, additionally qualified as the specifiers are.
    pub(super) fn infer_type(&mut self, d: &'tu Declaration<'tu>, index: usize) {
        let init = d.init_declarators[index];
        let spec = inferred_specifiers(d.declaration_specifiers);
        let keyword = inference_keyword(spec.type_specifiers).expect("an inferred declaration");
        let rules = self.context.configuration.compiler_identity().rules();
        let Some(InitializerType::AssignmentExpression(e)) =
            init.initializer.map(|initializer| initializer.kind)
        else {
            unreachable!("checked by declaration_fault");
        };
        let info = self.expression_info(e);
        let bit_field = info.bit_field.is_some() || info.promotion_width.is_some();
        let mut fault = None;
        let ty = if self.types.unanalyzed(info.ty) {
            self.types.unknown()
        } else if bit_field {
            // Clang rejects every operand that refers to a bit-field
            // (`Expr::refersToBitField`), GCC only a member access for
            // `__auto_type`. Otherwise GCC gives the object the operand's
            // bit-field type, which no declaration can name.
            if rules == RuleSet::Clang
                || (keyword == InferenceKeyword::AutoType && info.bit_field.is_some())
            {
                fault = Some(InferenceFault::BitField);
                self.types.unknown()
            } else {
                self.types.unmodeled(UnmodeledType::InferredBitField)
            }
        } else {
            let (mut ty, _) = self.value_conversion(info);
            // Clang's deduction removes qualifiers but keeps `_Atomic`,
            // which it models as a distinct type rather than a qualifier.
            if rules == RuleSet::Clang
                && matches!(self.types.nodes[info.ty.index], TypeKind::Atomic(_))
            {
                ty = info.ty.unqualified();
            }
            match self.deduce(init.declarator, ty) {
                | Some(leaf) => leaf,
                | None => {
                    fault = Some(InferenceFault::DeductionMismatch);
                    self.types.unknown()
                },
            }
        };
        // Clang deduces each declarator and requires one type for them all.
        let key = std::ptr::from_ref(d).addr();
        let ty = if fault.is_none() && !self.types.unanalyzed(ty) {
            match self.inferred_leaves.get(&key) {
                | Some(&first) if !same_type(first, ty) => {
                    fault = Some(InferenceFault::InconsistentDeduction);
                    self.types.unknown()
                },
                | Some(_) => ty,
                | None => {
                    _ = self.inferred_leaves.insert(key, ty);
                    ty
                },
            }
        } else {
            ty
        };
        if let Some(fault) = fault {
            self.error(
                SemanticErrorKind::InvalidInference(fault),
                init.declarator.source_vectors,
                init.declarator.identifier().map(|name| name.name),
                None,
            );
        }
        self.values.push(ty);
        self.work.push(Work::InferQualified(d, index));
        self.work
            .push(Work::Qualify(spec.type_qualifiers, spec.source_vectors));
    }

    /// Queues the binding of an inferred declarator whose qualified type is
    /// on the value stack.
    pub(super) fn infer_binding(&mut self, d: &'tu Declaration<'tu>, index: usize) {
        let base = self.take_type();
        let init = d.init_declarators[index];
        self.work.push(Work::DeclarationDeclarator(
            init.declarator,
            inferred_specifiers(d.declaration_specifiers),
            base,
            true,
        ));
    }

    /// Initializes the bound object from the initializer analyzed before
    /// it was bound, as `Work::InitializeDone` does for an ordinary one.
    pub(super) fn infer_initialization(
        &mut self,
        d: &'tu Declaration<'tu>,
        index: usize,
        omitted: bool,
        references: ReferenceMark,
    ) {
        let init = d.init_declarators[index];
        let initializer = init.initializer.expect("checked by declaration_fault");
        self.correspondence
            .initializer(init.declarator, initializer, omitted);
        if let Some(entry) = init
            .declarator
            .identifier()
            .and_then(|name| self.lookup(Namespace::Ordinary, name.name))
        {
            self.claim_references(references, entry.binding);
            self.work
                .push(Work::InitializeDone(entry.binding, initializer));
            self.functions.initializer = Some(entry.binding);
        }
    }

    /// The type an inferred declarator gives its innermost type: the
    /// operand's type for a plain identifier, or under Clang's deduction
    /// what remains after matching each pointer, array or function
    /// derivation against the operand's type. `None` when they do not
    /// match.
    fn deduce(&self, declarator: Declarator<'tu>, ty: TypeId) -> Option<TypeId> {
        let mut derivations = ArenaVec::new_in(self.scratch);
        derivations_of(declarator, &mut derivations);
        let mut ty = ty;
        for derivation in derivations.iter().rev() {
            ty = match (derivation, self.types.nodes[ty.index]) {
                | (Derivation::Pointer, TypeKind::Pointer(target)) => target,
                | (Derivation::Array, TypeKind::Array(element, _)) => element,
                | (Derivation::Function, TypeKind::Function { result, .. }) => result,
                | _ => return None,
            };
        }
        Some(ty)
    }

    /// Reports a use of the identifier an inferred declaration is
    /// declaring inside its own initializer: C23 `auto` under every
    /// identity, and `__auto_type` under Clang's, which declares the
    /// object before reading its initializer. A declaration inside the
    /// initializer that shadows the name is used normally. Returns whether
    /// it reported.
    /// C23 (N3220): §6.2.1 paragraph 7, p. 36; PDF p. 49.
    pub(super) fn inferred_self_reference(&mut self, name: Identifier) -> bool {
        let clang = self.context.configuration.compiler_identity().rules() == RuleSet::Clang;
        let Some(inference) = self.inferences.iter().rev().copied().find(|inference| {
            inference.name == Some(name.name)
                && (clang || inference.keyword == InferenceKeyword::Auto)
        }) else {
            return false;
        };
        if self
            .lookup(Namespace::Ordinary, name.name)
            .is_some_and(|entry| entry.binding >= inference.bindings)
        {
            return false;
        }
        self.error(
            SemanticErrorKind::InvalidInference(InferenceFault::SelfReference),
            name.source_vectors,
            Some(name.name),
            None,
        );
        true
    }

    /// GCC rejects a tag declared or defined inside a C23 `auto`
    /// initializer, where C23 leaves it implementation-defined
    /// (`in_underspecified_init`); Clang accepts it.
    /// C23 (N3220): §6.7.1 paragraph 12, p. 98; PDF p. 111.
    pub(super) fn underspecified_tag(&mut self, name: Option<Identifier>, source: SourceVectors) {
        if self.context.configuration.compiler_identity().rules() == RuleSet::Gcc
            && self
                .inferences
                .iter()
                .any(|inference| inference.keyword == InferenceKeyword::Auto)
        {
            self.error(
                SemanticErrorKind::InvalidInference(InferenceFault::TagInInitializer),
                source,
                name.map(|name| name.name),
                None,
            );
        }
    }

    /// Rejects `__auto_type` or C23 `auto` inference in a type name,
    /// member, parameter or function definition, where GCC and Clang
    /// accept none. Returns whether it reported.
    /// C23 (N3220): §6.7.10 paragraph 1, p. 134; PDF p. 147.
    pub(super) fn misplaced_inference(
        &mut self,
        specifiers: TypeSpecifiers<'tu>,
        fault: InferenceFault,
        source: SourceVectors,
    ) -> bool {
        if inference_keyword(specifiers).is_none() {
            return false;
        }
        self.error(SemanticErrorKind::InvalidInference(fault), source, None, None);
        true
    }
}

/// Whether a declaration's type is inferred from its initializer.
pub(super) fn inferred(spec: DeclarationSpecifiers<'_>) -> bool {
    inference_keyword(spec.type_specifiers).is_some()
}

/// The specifiers an inferred declaration binds with: a lone C23 `auto`
/// asks for inference and is no storage-class specifier, so a file-scope
/// object has external linkage and static duration.
/// C23 (N3220): §6.7.2 paragraph 4, p. 99; PDF p. 112.
fn inferred_specifiers(spec: DeclarationSpecifiers<'_>) -> DeclarationSpecifiers<'_> {
    if spec.storage_class == Some(StorageClass::Auto) && !spec.auto_with_storage_class {
        DeclarationSpecifiers {
            storage_class: None,
            ..spec
        }
    } else {
        spec
    }
}

fn inference_keyword(specifiers: TypeSpecifiers<'_>) -> Option<InferenceKeyword> {
    match specifiers {
        | TypeSpecifiers::Extended(ExtendedType::Inferred) => Some(InferenceKeyword::Auto),
        | TypeSpecifiers::Extended(ExtendedType::AutoType) => Some(InferenceKeyword::AutoType),
        | _ => None,
    }
}

/// Clang compares deduced types without their storage alignment.
fn same_type(left: TypeId, right: TypeId) -> bool {
    left.index == right.index && left.qualifiers == right.qualifiers
}

/// Appends a declarator's derivations in the order they apply to the
/// declaration's base type: its pointers, then its suffixes from the last
/// written, then those of a parenthesized inner declarator.
/// C99: §6.7.5 paragraphs 4-6, pp. 114-115; PDF pp. 126-127.
fn derivations_of(declarator: Declarator<'_>, out: &mut ArenaVec<'_, Derivation>) {
    let mut current = Some(declarator);
    while let Some(declarator) = current {
        current = None;
        for _ in declarator.pointer.levels {
            out.push(Derivation::Pointer);
        }
        for direct in declarator.kind.iter().rev() {
            match *direct {
                | DirectDeclarator::Array { .. } => out.push(Derivation::Array),
                | DirectDeclarator::Function { .. }
                | DirectDeclarator::KAndRStyleFunction { .. } => out.push(Derivation::Function),
                | DirectDeclarator::Parenthesized(inner) => current = Some(inner.declarator),
                | DirectDeclarator::MsModifier(..)
                | DirectDeclarator::AsmLabel(_)
                | DirectDeclarator::Attributes(_)
                | DirectDeclarator::Identifier(_) => {},
            }
        }
    }
}

/// One declarator derivation, as Clang's deduction matches it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Derivation {
    Pointer,
    Array,
    Function,
}

/// Which keyword asks for inference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InferenceKeyword {
    /// C23 `auto` without a type specifier.
    Auto,
    /// GNU `__auto_type`.
    AutoType,
}

/// An inferred declaration whose initializer is being analyzed.
#[derive(Clone, Copy, Debug)]
pub(super) struct Inference {
    name:       Option<StringCacheId>,
    /// The bindings that existed before the initializer, which a use of
    /// `name` must not find.
    bindings:   usize,
    keyword:    InferenceKeyword,
    /// The object's storage duration, which its specifiers decide.
    duration:   Duration,
    /// Whether the object has block scope and no linkage.
    local:      bool,
}

impl Inference {
    pub(super) const fn duration(self) -> Duration {
        self.duration
    }

    /// Whether the object is a block-scope static or thread-local one.
    pub(super) fn static_local(self) -> bool {
        self.local && self.duration.is_static_or_thread()
    }
}

/// Why an inferred declaration is invalid. Where C23 leaves a form
/// implementation-defined, the fault is reported only under the identity
/// whose compiler rejects it.
/// C23 (N3220): §6.7.10 paragraphs 1-2, pp. 134-135; PDF pp. 147-148.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InferenceFault {
    MissingInitializer,
    InitializerList,
    /// GCC: more than one declarator.
    MultipleDeclarators,
    /// GCC: a declarator other than an identifier.
    NotPlainIdentifier,
    /// Clang: the identifier declares an array.
    ArrayDeclarator,
    /// Clang: the identifier declares a function.
    FunctionDeclarator,
    /// A type name, member, parameter, typedef or function definition.
    NotAllowedHere,
    SelfReference,
    BitField,
    /// Clang: the initializer does not match the declarator's derivations.
    DeductionMismatch,
    /// Clang: declarators of one declaration deduce different types.
    InconsistentDeduction,
    /// C23 `auto` redeclares an identifier of the same scope.
    Redeclared,
    /// GCC: a tag declared inside a C23 `auto` initializer.
    TagInInitializer,
    /// C23 `auto` beside another storage class with a type specifier.
    StorageWithoutInference,
}

impl InferenceFault {
    /// The short label of the diagnostic.
    pub(super) const fn label(self) -> &'static str {
        match self {
            | Self::MissingInitializer => "no initializer",
            | Self::InitializerList => "braced initializer",
            | Self::MultipleDeclarators => "second declarator",
            | Self::NotPlainIdentifier => "not a plain identifier",
            | Self::ArrayDeclarator => "array declarator",
            | Self::FunctionDeclarator => "function declarator",
            | Self::NotAllowedHere => "inferred type here",
            | Self::SelfReference => "used in its own initializer",
            | Self::BitField => "bit-field operand",
            | Self::DeductionMismatch => "incompatible initializer",
            | Self::InconsistentDeduction => "deduced as a different type",
            | Self::Redeclared => "already declared in this scope",
            | Self::TagInInitializer => "tag declared here",
            | Self::StorageWithoutInference => "auto with a type specifier",
        }
    }

    /// The message and note of the diagnostic.
    pub(super) const fn explanation(self) -> (&'static str, &'static str) {
        match self {
            | Self::MissingInitializer => (
                "a declaration with an inferred type requires an initializer",
                "C23 §6.7.10p2: an inferred object definition has the form `direct-declarator = \
                 assignment-expression`",
            ),
            | Self::InitializerList => (
                "an inferred type cannot come from a braced initializer list",
                "C23 §6.7.10p2: the type is that of an assignment expression",
            ),
            | Self::MultipleDeclarators => (
                "an inferred type may only be used with a single declarator",
                "C23 §6.7.1p12: an underspecified declaration of more than one identifier is \
                 implementation-defined; GCC rejects it",
            ),
            | Self::NotPlainIdentifier => (
                "an inferred type requires a plain identifier as declarator",
                "C23 §6.7.10p2: a declarator other than an identifier with attributes is \
                 implementation-defined; GCC rejects it",
            ),
            | Self::ArrayDeclarator => (
                "an inferred type cannot declare an array",
                "C23 §6.7.10p2: a declarator other than an identifier is implementation-defined; \
                 Clang deduces through pointer and function declarators but not an array",
            ),
            | Self::FunctionDeclarator => (
                "an inferred type cannot declare a function",
                "C23 §6.7.10p2 and §6.7.1p12: an inferred declaration defines an object",
            ),
            | Self::NotAllowedHere => (
                "type inference is only allowed in an initialized object declaration",
                "C23 §6.7.10p1-2: only an object definition with an initializer infers its type; \
                 GCC and Clang accept __auto_type in the same places",
            ),
            | Self::SelfReference => (
                "a variable with an inferred type is used in its own initializer",
                "C23 §6.2.1p7: an underspecified identifier's scope starts when its definition is \
                 completed",
            ),
            | Self::BitField => (
                "an inferred type cannot come from a bit-field",
                "GCC rejects __auto_type of a bit-field member and Clang any inferred type that \
                 refers to a bit-field: no declaration can name a bit-field's type",
            ),
            | Self::DeductionMismatch => (
                "the initializer's type does not match the inferred declarator",
                "C23 §6.7.10p2: a declarator other than an identifier is implementation-defined; \
                 Clang deduces the type its pointer and function derivations leave",
            ),
            | Self::InconsistentDeduction => (
                "the inferred type differs between declarators",
                "C23 §6.7.1p12: an underspecified declaration of more than one identifier is \
                 implementation-defined; Clang requires one deduced type",
            ),
            | Self::Redeclared => (
                "an underspecified declaration redeclares an identifier of the same scope",
                "C23 §6.7.1p12: redeclaring an identifier of the same scope this way is \
                 implementation-defined; GCC and Clang reject it",
            ),
            | Self::TagInInitializer => (
                "a tag is declared in an underspecified object initializer",
                "C23 §6.7.1p12: declaring a tag inside an underspecified declaration is \
                 implementation-defined; GCC rejects it",
            ),
            | Self::StorageWithoutInference => (
                "auto beside another storage-class specifier requires an inferred type",
                "C23 §6.7.2p4: auto joins another storage-class specifier only if the type is \
                 inferred from an initializer",
            ),
        }
    }
}
