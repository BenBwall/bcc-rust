//! GCC's and Clang's warnings about what a program declares or computes and
//! never uses: `-Wunused-variable`, `-Wunused-parameter`,
//! `-Wunused-but-set-variable`, `-Wunused-but-set-parameter`,
//! `-Wunused-label`, `-Wunused-function`, `-Wunused-const-variable` and
//! `-Wunused-value`. None of these is a C constraint; each is a warning
//! C99 §5.1.1.3p1 permits.
//!
//! An identifier's analysis records its use. The two compilers count
//! differently, so both counts are kept:
//!
//! - GCC asks whether a variable was referenced (`TREE_USED`) and read
//!   (`DECL_READ_P`). A discarded `=` writes its target, through parentheses,
//!   `.` and array subscripts; every other reference reads, `a++` and `a += 1`
//!   included.
//! - Clang counts references and subtracts discarded assignments
//!   (`RefsMinusAssignments`): `=`, a compound assignment or `++` or `--`
//!   discarded with a bare identifier as its operand.
//!
//! Leaving a scope reports its variables, and the end of the translation unit
//! reports file-scope objects and functions. A compilation with none of these
//! warnings on records nothing: [`UnusedState::tracking`] is false.
//! C99: §5.1.1.3p1, p. 11; PDF p. 23.

use rustc_hash::FxBuildHasher;

use super::{
    Analyzer,
    BindingKind,
    Integer,
    Linkage,
    ScopeKind,
    SemanticErrorKind,
    ValueCategory,
    scopes::Namespace,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::{
        CompilerConfiguration,
        CompilerIdentity,
        DiagnosticGroup as G,
        RuleSet,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            ExtendedType,
            SpecifierExtension,
            SpecifierExtensionKind,
            SyntaxOperand,
            declaration_syntax::{
                DeclarationSpecifiers,
                Declarator,
                DirectDeclarator,
                TypeQualifiers,
                TypeSpecifiers,
            },
            syntax::{
                BinaryOperator as B,
                Expression,
                ExpressionType as E,
                Identifier,
                Statement,
                StatementType,
                UnaryOperator as U,
            },
        },
    },
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// What this translation unit did with one variable, or with one file-scope
/// name, which all its declarations share.
#[derive(Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent fact about the name's uses."
)]
struct Usage {
    /// GCC's `TREE_USED` and Clang's `isReferenced`.
    referenced: bool,
    /// GCC's `DECL_READ_P`.
    read:       bool,
    /// Clang's `RefsMinusAssignments`, whose subtraction happens by not
    /// counting the reference that an assignment discards.
    references: u32,
    /// A function's references outside its own body, which GCC counts.
    outside:    bool,
    /// `unused`, `maybe_unused`, `used`, `cleanup`, or for a function
    /// `constructor`, `destructor` or being a symbol attribute's target.
    exempt:     bool,
    /// A function declared `inline`.
    inline:     bool,
    /// A function declared GCC's `weakref`, which is defined elsewhere.
    weakref:    bool,
    /// What the name's type says, as [`Analyzer::type_mark`] reads it.
    marked:     bool,
}

/// How a discarded assignment writes the identifier expression it names.
#[derive(Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent rule of one of the two compilers."
)]
struct WriteSite {
    /// GCC does not count the reference as a read.
    gcc:            bool,
    /// Only while the identifier is an array: `a[0] = 1` reads a pointer `a`.
    gcc_array:      bool,
    /// Clang does not count the reference.
    clang:          bool,
    /// Clang counts it after all when the variable is volatile: `a += 1` and
    /// `a++`, not `a = 1`.
    clang_compound: bool,
}

/// Working state for the warnings this module implements, and whether
/// the other warnings an identifier-free check serves are on.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag gates an independent group of warnings."
)]
pub(super) struct UnusedState<'s> {
    /// Whether a warning that needs identifier uses recorded is on.
    pub(super) tracking: bool,
    /// Whether a warning about discarded values is on, as Clang's is by
    /// default.
    pub(super) values: bool,
    /// Whether a warning about what a `switch` handles may be on.
    pub(super) switches: bool,
    /// Whether a warning about reaching a function's end may be on.
    pub(super) return_flow: bool,
    /// Whether `-Wsign-compare` is on.
    pub(super) sign_compare: bool,
    /// Whether `-Wmissing-field-initializers` is on.
    pub(super) missing_fields: bool,
    /// Where GCC's `input_location` last moved to a tag.
    pub(super) last_tag: Option<SourceVectors>,
    variables: ArenaVec<'s, Usage>,
    file_scope: ArenaMap<'s, StringCacheId, Usage>,
    writes: ArenaMap<'s, usize, WriteSite>,
    /// The statements that give a statement expression its value.
    statement_expression_values: ArenaMap<'s, usize, ()>,
    /// The commas that separate an attribute's arguments, which bcc parses as
    /// one expression: their left operands are arguments, not discarded.
    separators: ArenaMap<'s, usize, ()>,
    /// Tags declared `unused`: true for a definition that says so, which is
    /// the only one GCC's type carries; false for a forward declaration, which
    /// Clang's definition inherits.
    unused_tags: ArenaMap<'s, usize, bool>,
    /// Where each scope that can hold a variable ends, kept only under GCC's
    /// rules in a unit with diagnostic pragmas: see
    /// [`Analyzer::note_scope_end`].
    scope_ends: Option<ArenaMap<'s, usize, SourceVectors>>,
}

impl<'s> UnusedState<'s> {
    /// `pragmas` says a diagnostic pragma may turn on what the command line
    /// leaves off, so that only the report can tell.
    pub(super) fn new(
        scratch: &'s Bump,
        configuration: CompilerConfiguration,
        pragmas: bool,
    ) -> Self {
        let on = |groups: &[G]| {
            !matches!(
                configuration.compiler_identity(),
                CompilerIdentity::Tcc | CompilerIdentity::Msvc
            ) && (pragmas
                || groups
                    .iter()
                    .any(|&group| configuration.diagnostic_severity(group).is_some()))
        };
        let tracking = on(&[
            G::UnusedVariable,
            G::UnusedConstVariable,
            G::UnusedParameter,
            G::UnusedButSetVariable,
            G::UnusedButSetGlobal,
            G::UnusedButSetParameter,
            G::UnusedLabel,
            G::UnusedFunction,
        ]);
        let values = on(&[G::UnusedValue, G::UnusedComparison]);
        Self {
            tracking,
            values,
            switches: on(&[
                G::Switch,
                G::SwitchEnum,
                G::SwitchDefault,
                G::SwitchOutsideRange,
            ]),
            return_flow: on(&[G::ReturnType, G::InvalidNoreturn, G::GccNoreturn]),
            sign_compare: on(&[G::SignCompare]),
            missing_fields: on(&[G::MissingFieldInitializers]),
            last_tag: None,
            variables: ArenaVec::new_in(scratch),
            file_scope: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            writes: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            statement_expression_values: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            separators: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            unused_tags: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            scope_ends: (pragmas && configuration.compiler_identity() == CompilerIdentity::Gcc)
                .then(|| ArenaMap::with_hasher_in(FxBuildHasher, scratch)),
        }
    }

    fn variable(&mut self, binding: usize) -> &mut Usage {
        if self.variables.len() <= binding {
            self.variables.resize(binding + 1, Usage::default());
        }
        &mut self.variables[binding]
    }
}

/// One warning, in GCC's or Clang's terms where those differ; the group a
/// rule belongs to is the compiler's too.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum UnusedRule {
    Variable,
    Parameter,
    ButSetVariable,
    ButSetParameter,
    /// Clang: an internal object only assigned to.
    ButSetGlobal,
    /// GCC's words for a file-scope object.
    GccFileObject,
    GccFileConstant,
    /// Clang's words for a file-scope constant.
    ClangFileConstant,
    GccLabel,
    ClangLabel,
    GccFunction,
    ClangFunction,
    GccNoEffect,
    GccValueComputed,
    GccCommaLeft,
    GccCommaRight,
    ClangResult,
    ClangEquality,
    ClangInequality,
    ClangRelational,
    ClangCommaLeft,
    ClangVoidPointerCast,
}

impl UnusedRule {
    pub(super) const fn group(self) -> G {
        match self {
            | Self::Variable | Self::GccFileObject => G::UnusedVariable,
            | Self::GccFileConstant | Self::ClangFileConstant => G::UnusedConstVariable,
            | Self::Parameter => G::UnusedParameter,
            | Self::ButSetVariable => G::UnusedButSetVariable,
            | Self::ButSetParameter => G::UnusedButSetParameter,
            | Self::ButSetGlobal => G::UnusedButSetGlobal,
            | Self::GccLabel | Self::ClangLabel => G::UnusedLabel,
            | Self::GccFunction | Self::ClangFunction => G::UnusedFunction,
            | Self::ClangEquality | Self::ClangInequality | Self::ClangRelational =>
                G::UnusedComparison,
            | Self::GccNoEffect
            | Self::GccValueComputed
            | Self::GccCommaLeft
            | Self::GccCommaRight
            | Self::ClangResult
            | Self::ClangCommaLeft
            | Self::ClangVoidPointerCast => G::UnusedValue,
        }
    }

    /// The words before and after the name, which is quoted between them;
    /// a rule that names nothing has only the first.
    pub(super) const fn words(self) -> (&'static str, &'static str) {
        match self {
            | Self::Variable | Self::ClangFileConstant => ("unused variable", ""),
            | Self::Parameter => ("unused parameter", ""),
            | Self::ButSetVariable | Self::ButSetGlobal => ("variable", "set but not used"),
            | Self::ButSetParameter => ("parameter", "set but not used"),
            | Self::GccFileObject | Self::GccFileConstant | Self::GccFunction =>
                ("", "defined but not used"),
            | Self::GccLabel => ("label", "defined but not used"),
            | Self::ClangLabel => ("unused label", ""),
            | Self::ClangFunction => ("unused function", ""),
            | Self::GccNoEffect => ("statement with no effect", ""),
            | Self::GccValueComputed => ("value computed is not used", ""),
            | Self::GccCommaLeft => ("left-hand operand of comma expression has no effect", ""),
            | Self::GccCommaRight => ("right-hand operand of comma expression has no effect", ""),
            | Self::ClangResult => ("expression result unused", ""),
            | Self::ClangEquality => ("equality comparison result unused", ""),
            | Self::ClangInequality => ("inequality comparison result unused", ""),
            | Self::ClangRelational => ("relational comparison result unused", ""),
            | Self::ClangCommaLeft => ("left operand of comma operator has no effect", ""),
            | Self::ClangVoidPointerCast => (
                "expression result unused; should this cast be to 'void'?",
                "",
            ),
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            | Self::GccLabel | Self::ClangLabel => "never jumped to",
            | Self::ButSetVariable | Self::ButSetParameter | Self::ButSetGlobal =>
                "assigned but never read",
            | Self::GccNoEffect
            | Self::GccValueComputed
            | Self::GccCommaLeft
            | Self::GccCommaRight
            | Self::ClangResult
            | Self::ClangEquality
            | Self::ClangInequality
            | Self::ClangRelational
            | Self::ClangCommaLeft
            | Self::ClangVoidPointerCast => "result is discarded",
            | _ => "never used",
        }
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Records the identifier expression `e`, which names `binding`.
    pub(super) fn note_identifier_use(&mut self, binding: usize, e: &'tu Expression<'tu>) {
        let data = self.bindings[binding];
        let site = self
            .unused
            .writes
            .get(&std::ptr::from_ref(e).addr())
            .copied()
            .unwrap_or_default();
        let array = matches!(self.types.nodes[data.ty.index], TypeKind::Array(..));
        let written_gcc = site.gcc && (!site.gcc_array || array);
        let written_clang = site.clang && !(site.clang_compound && data.ty.is_volatile());
        self.note_reference(binding, written_gcc, written_clang);
    }

    /// Counts one reference to `binding`, which only writes it under the
    /// compilers that say so.
    pub(super) fn note_reference(
        &mut self,
        binding: usize,
        written_gcc: bool,
        written_clang: bool,
    ) {
        let data = self.bindings[binding];
        let current = self.functions.current.and_then(|function| function.binding);
        let own_body =
            current.is_some_and(|function| self.bindings[function].name.name == data.name.name);
        let usage = if data.linkage == Linkage::Internal {
            self.unused.file_scope.entry(data.name.name).or_default()
        } else {
            self.unused.variable(binding)
        };
        usage.referenced = true;
        usage.read |= !written_gcc;
        usage.outside |= !own_body;
        if !written_clang {
            usage.references = usage.references.saturating_add(1);
        }
    }

    /// Records a discarded expression's assignments, before it is analyzed.
    /// C99: §6.8.3p2, p. 132; PDF p. 144.
    pub(super) fn note_discarded(&mut self, expression: &'tu Expression<'tu>) {
        // Clang looks only at the expression itself: a parenthesized
        // assignment, or one on a comma's right, stays a reference. A comma's
        // left operand comes here again when the comma is analyzed.
        let mut bare = true;
        let mut e = expression;
        loop {
            match e.kind {
                | E::Parenthesized { expression } => {
                    bare = false;
                    e = expression;
                },
                | E::Binary {
                    operator: B::Comma,
                    right_expression,
                    ..
                } => {
                    bare = false;
                    e = right_expression;
                },
                | E::Binary {
                    operator: B::Assignment,
                    left_expression,
                    ..
                } => {
                    self.note_gcc_write(left_expression);
                    if bare {
                        self.note_clang_write(left_expression, false);
                    }
                    return;
                },
                | E::Binary {
                    operator:
                        B::MultiplicationAssignment
                        | B::DivisionAssignment
                        | B::ModuloAssignment
                        | B::AdditionAssignment
                        | B::SubtractionAssignment
                        | B::LeftShiftAssignment
                        | B::RightShiftAssignment
                        | B::BitwiseAndAssignment
                        | B::BitwiseXorAssignment
                        | B::BitwiseOrAssignment,
                    left_expression,
                    ..
                } => {
                    if bare {
                        self.note_clang_write(left_expression, true);
                    }
                    return;
                },
                | E::Unary {
                    operator:
                        U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement,
                    operand_expression,
                } => {
                    if bare {
                        self.note_clang_write(operand_expression, true);
                    }
                    return;
                },
                | _ => return,
            }
        }
    }

    /// GCC's write reaches the object through parentheses, `.` and array
    /// subscripts, but not through a pointer.
    fn note_gcc_write(&mut self, target: &'tu Expression<'tu>) {
        let mut array = false;
        let mut e = target;
        loop {
            match e.kind {
                | E::Parenthesized { expression } => e = expression,
                | E::DirectMember {
                    base_expression, ..
                } => e = base_expression,
                | E::Binary {
                    operator: B::Subscript,
                    left_expression,
                    ..
                } => {
                    array = true;
                    e = left_expression;
                },
                | E::Identifier(_) => {
                    let site = self
                        .unused
                        .writes
                        .entry(std::ptr::from_ref(e).addr())
                        .or_default();
                    site.gcc = true;
                    site.gcc_array = array;
                    return;
                },
                | _ => return,
            }
        }
    }

    fn note_clang_write(&mut self, target: &'tu Expression<'tu>, compound: bool) {
        if matches!(target.kind, E::Identifier(_)) {
            let site = self
                .unused
                .writes
                .entry(std::ptr::from_ref(target).addr())
                .or_default();
            site.clang = true;
            site.clang_compound = compound;
        }
    }

    /// Records what the declaration of `binding` takes from its type, when the
    /// declarator derives nothing from what its specifiers name: `U x` and
    /// `U (x)`, not `U *x` or `U x[2]`.
    ///
    /// GCC keeps `unused` on the type (`TREE_USED`), which a typedef or a tag's
    /// definition sets, a later typedef and `__typeof__` copy, and a variable
    /// takes as being used and read, at file scope too. No parameter does.
    /// GCC: `set_underlying_type` in `c-common.cc`, `grokdeclarator` in
    /// `c-decl.cc`.
    ///
    /// Clang looks at a local variable when it reports one, in
    /// [`Self::report_unused_variable`]: at the typedef its type names, the
    /// last one and no other, and at a tag. Clang: `ShouldDiagnoseUnusedDecl`.
    pub(super) fn note_declared_type(
        &mut self,
        binding: usize,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Declarator<'tu>,
        (kind, linkage): (BindingKind, Linkage),
        name: Identifier,
    ) {
        if kind == BindingKind::Function && spec.function_specifiers.is_inline {
            self.unused.file_scope.entry(name.name).or_default().inline = true;
        }
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        let marked = match kind {
            // C99 §6.7.7p3: a typedef declares no new type, and GCC merges a
            // redeclared one into the first.
            | BindingKind::Typedef =>
                self.lookup(Namespace::Ordinary, name.name)
                    .is_some_and(|prior| prior.scope == self.scope && self.is_marked(prior.binding))
                    || (gcc && Self::derives_nothing(Some(declarator)) && self.type_mark(spec)),
            | BindingKind::Object =>
                Self::derives_nothing(Some(declarator)) && self.type_mark(spec),
            | _ => false,
        };
        if !marked {
            return;
        }
        let usage = self.unused.variable(binding);
        usage.marked = true;
        if gcc && kind == BindingKind::Object {
            usage.exempt = true;
            if linkage == Linkage::Internal {
                self.unused.file_scope.entry(name.name).or_default().exempt = true;
            }
        }
    }

    /// Whether `declarator`'s type is the one its specifiers give.
    /// C99: §6.7.5p4-p6, p. 115; PDF p. 127.
    fn derives_nothing(declarator: Option<Declarator<'tu>>) -> bool {
        let Some(declarator) = declarator else {
            return true;
        };
        declarator.pointer.levels.is_empty()
            && declarator.kind.iter().all(|direct| match *direct {
                | DirectDeclarator::Parenthesized(inner) =>
                    Self::derives_nothing(Some(inner.declarator)),
                | DirectDeclarator::Identifier(_)
                | DirectDeclarator::Attributes(_)
                | DirectDeclarator::AsmLabel(_)
                | DirectDeclarator::MsModifier(..) => true,
                | _ => false,
            })
    }

    /// Whether the type `spec` names carries `unused`: for GCC, by `TREE_USED`
    /// at this point of the unit, and for Clang by its typedef's own attribute.
    /// `__typeof__` names what its operand does. Of an expression's type only
    /// a plain name is read.
    /// C23: §6.7.3.6, p. 117; PDF p. 130.
    fn type_mark(&mut self, spec: DeclarationSpecifiers<'tu>) -> bool {
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        match spec.type_specifiers {
            | TypeSpecifiers::TypedefName(typedef) =>
                (gcc || !spec.type_qualifiers.contains(TypeQualifiers::ATOMIC))
                    && self
                        .lookup(Namespace::Ordinary, typedef.name)
                        .is_some_and(|entry| self.is_marked(entry.binding)),
            | TypeSpecifiers::StructOrUnion(record) if gcc => self.tag_carries_unused(
                record.identifier,
                record.struct_declaration_list.is_some(),
                record.attributes,
            ),
            | TypeSpecifiers::Enum(enumeration) if gcc => self.tag_carries_unused(
                enumeration.name,
                enumeration.enumeration_list.is_some(),
                enumeration.attributes,
            ),
            // `_Atomic(T)` is a qualified variant of `T`, which copies its
            // `TREE_USED`. Clang's type is no longer the typedef.
            // C11: §6.7.2.4p4, p. 121; PDF p. 139.
            | TypeSpecifiers::Extended(ExtendedType::Atomic(name)) if gcc =>
                Self::derives_nothing(name.declarator)
                    && self.type_mark(name.declaration_specifiers),
            | TypeSpecifiers::Extended(ExtendedType::Typeof { operand, .. }) => match *operand {
                | SyntaxOperand::Type(name) =>
                    Self::derives_nothing(name.declarator)
                        && self.type_mark(name.declaration_specifiers),
                | SyntaxOperand::Expression(e) => {
                    let E::Identifier(name) = Self::without_parentheses(e).kind else {
                        return false;
                    };
                    self.lookup(Namespace::Ordinary, name.name)
                        .is_some_and(|entry| self.is_marked(entry.binding))
                },
            },
            | _ => false,
        }
    }

    /// `unused`, `maybe_unused`, `used`, `cleanup`, `constructor` or
    /// `destructor` on `binding`.
    pub(super) fn exempt_from_unused(&mut self, binding: usize) {
        let data = self.bindings[binding];
        if data.linkage == Linkage::Internal {
            self.unused
                .file_scope
                .entry(data.name.name)
                .or_default()
                .exempt = true;
        }
        self.unused.variable(binding).exempt = true;
    }

    /// A function or object a symbol attribute's string names.
    pub(super) fn exempt_symbol_target(&mut self, name: &str) {
        let name = self.context.string_cache.intern(name);
        self.unused.file_scope.entry(name).or_default().exempt = true;
    }

    /// An internal function GCC's `weakref` says is defined elsewhere.
    pub(super) fn note_weak_reference(&mut self, binding: usize) {
        let name = self.bindings[binding].name.name;
        self.unused.file_scope.entry(name).or_default().weakref = true;
    }

    /// Whether GCC takes an undefined internal function for a weak reference.
    pub(super) fn is_weak_reference(&self, name: StringCacheId) -> bool {
        self.unused
            .file_scope
            .get(&name)
            .is_some_and(|usage| usage.weakref)
    }

    /// A tag's specifier: a definition that says `unused` gives its type the
    /// attribute, and Clang's definition inherits it from a forward
    /// declaration, which GCC's does not. Once the type is complete, neither
    /// compiler reads the attribute on a mention of the tag.
    pub(in crate::translation_phases::semantic_analysis) fn note_tag_attributes(
        &mut self,
        tag: usize,
        attributes: Option<&SpecifierExtension<'tu>>,
        define: bool,
    ) {
        if !self.unused.tracking || !self.attribute_chain_exempts(attributes) {
            return;
        }
        if define {
            _ = self.unused.unused_tags.insert(tag, true);
        } else if !self.types.tags[tag].complete.get() {
            _ = self.unused.unused_tags.entry(tag).or_insert(false);
        }
    }

    /// `unused` on a typedef, which GCC's type carries by that spelling alone
    /// (`set_underlying_type` looks for it), and Clang by any.
    pub(in crate::translation_phases::semantic_analysis) fn note_typedef_attribute(
        &mut self,
        binding: usize,
        name: &str,
    ) {
        if name == "unused"
            || (name == "maybe_unused"
                && self.context.configuration.compiler_identity() != CompilerIdentity::Gcc)
        {
            self.unused.variable(binding).marked = true;
        }
    }

    fn is_marked(&self, binding: usize) -> bool {
        self.unused
            .variables
            .get(binding)
            .is_some_and(|usage| usage.marked)
    }

    /// Whether GCC's type for a tag's specifier carries `unused` by now.
    fn tag_carries_unused(
        &self,
        name: Option<Identifier>,
        body: bool,
        attributes: Option<&SpecifierExtension<'tu>>,
    ) -> bool {
        if body {
            return self.attribute_chain_exempts(attributes);
        }
        name.and_then(|name| self.lookup(Namespace::Tag, name.name))
            .is_some_and(|entry| self.unused.unused_tags.get(&entry.binding) == Some(&true))
    }

    pub(super) fn attribute_chain_exempts(
        &self,
        mut chain: Option<&SpecifierExtension<'tu>>,
    ) -> bool {
        while let Some(item) = chain {
            if let SpecifierExtensionKind::Attributes(attribute) = item.kind
                && super::attributes::known_names(attribute, self.context)
                    .any(|(_, name)| matches!(name, "unused" | "maybe_unused"))
            {
                return true;
            }
            chain = item.next;
        }
        false
    }

    /// Whether `ty`, or its elements, is a tag Clang's `Tag->hasAttr` finds
    /// `unused`, however the declaration names it.
    fn clang_tag_is_unused(&self, mut ty: TypeId) -> bool {
        while let TypeKind::Array(element, _) = self.types.nodes[ty.index] {
            ty = element;
        }
        matches!(
            self.types.nodes[ty.index],
            TypeKind::Tag(id) if self.unused.unused_tags.contains_key(&id)
        )
    }

    /// A parameter's definition binding continues its prototype binding,
    /// whose attributes and whose uses in later parameters' bounds count.
    pub(super) fn inherit_unused_exemption(&mut self, prototype: usize, parameter: usize) {
        if let Some(&usage) = self.unused.variables.get(prototype) {
            *self.unused.variable(parameter) = usage;
        }
    }

    /// Reports the variables and parameters of the scope being left.
    pub(super) fn report_unused_in_scope(&mut self) {
        if self.scope == 0 || self.scopes[self.scope].kind == ScopeKind::Prototype {
            return;
        }
        let mut next = self.scope_entries[self.scope].head.get();
        while let Some(link) = next {
            next = link.next;
            let entry = self.entries[link.value];
            if entry.namespace == Namespace::Ordinary {
                self.report_unused_variable(entry.binding, entry.name);
            }
        }
    }

    fn report_unused_variable(&mut self, binding: usize, name: Identifier) {
        let data = self.bindings[binding];
        let parameter = match data.kind {
            | BindingKind::Parameter => true,
            | BindingKind::Object => false,
            | _ => return,
        };
        if self.types.unanalyzed(data.ty) {
            return;
        }
        if data.linkage == Linkage::Internal {
            self.report_block_extern(data.kind, name);
            return;
        }
        let usage = *self.unused.variable(binding);
        // The object C99 §6.4.2.2p1 declares implicitly.
        if usage.exempt
            || matches!(
                self.context.string_cache.at(name.name),
                "__func__" | "__FUNCTION__" | "__PRETTY_FUNCTION__"
            )
        {
            return;
        }
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        // Clang leaves a block-scope `extern` to its own definition, and
        // has nothing to say about an incomplete type.
        if !gcc
            && (data.linkage == Linkage::External
                || (!parameter && self.clang_tag_is_unused(data.ty)))
        {
            return;
        }
        let set_only = if gcc {
            !usage.read
        } else {
            usage.references == 0
        };
        let rule = if !usage.referenced {
            if parameter {
                UnusedRule::Parameter
            } else if usage.marked {
                // Clang's typedef answers whether it is unused, not whether
                // it is only set; GCC's has made it used and read by now.
                return;
            } else {
                UnusedRule::Variable
            }
        } else if set_only && data.linkage == Linkage::None {
            if parameter {
                UnusedRule::ButSetParameter
            } else {
                UnusedRule::ButSetVariable
            }
        } else {
            return;
        };
        let settings_at = if gcc && rule == UnusedRule::Variable {
            self.scope_end_or(self.scope, name.source_vectors)
        } else {
            name.source_vectors
        };
        self.unused_rule_read_at(rule, name.source_vectors, Some(name.name), settings_at);
    }

    /// A block-scope `extern` naming a file-scope `static`: GCC's `pop_scope`
    /// reports it as it does a local variable, and it stands for the name's
    /// other reports. Clang reports only the file-scope declaration.
    /// C99: §6.2.2p4, p. 30; PDF p. 42.
    fn report_block_extern(&mut self, kind: BindingKind, name: Identifier) {
        if kind != BindingKind::Object
            || self.context.configuration.compiler_identity() != CompilerIdentity::Gcc
        {
            return;
        }
        let usage = self.unused.file_scope.entry(name.name).or_default();
        if usage.exempt || usage.referenced {
            return;
        }
        usage.referenced = true;
        let settings_at = self.scope_end_or(self.scope, name.source_vectors);
        self.unused_rule_read_at(
            UnusedRule::Variable,
            name.source_vectors,
            Some(name.name),
            settings_at,
        );
    }

    /// Notes that `source`, a compound or `for` statement, ends the scope just
    /// entered. GCC reports an unused variable and an unused label as it leaves
    /// their scope, and its `warning` asks the settings at the current token
    /// though it prints the entity's location: a pragma after the declaration
    /// and before the closing brace governs them, unlike every other warning
    /// here, which it asks at the entity.
    /// GCC: `pop_scope` in `c-decl.cc`.
    pub(super) fn note_scope_end(&mut self, source: SourceVectors) {
        let scope = self.scope;
        if let Some(ends) = &mut self.unused.scope_ends {
            _ = ends.insert(scope, source);
        }
    }

    /// Where the settings of a warning GCC reports on leaving `scope` hold:
    /// the scope's last token, or `otherwise` for a scope with no end noted.
    fn scope_end_or(&self, scope: usize, otherwise: SourceVectors) -> SourceVectors {
        self.unused
            .scope_ends
            .as_ref()
            .and_then(|ends| ends.get(&scope))
            .map_or(otherwise, |&end| {
                crate::translation_phases::context::Context::last_source_vector(end)
            })
    }

    /// The scope a label belongs to, for [`Self::scope_end_or`]: the current
    /// one for a local label, and the function's for any other.
    /// GNU extension: GCC manual, "Local Labels".
    /// C99: §6.2.1 paragraph 3, p. 29; PDF p. 41.
    pub(super) fn label_scope(&self, local: bool) -> usize {
        let mut scope = self.scope;
        if !local && self.unused.scope_ends.is_some() {
            while self.scopes[scope].kind != ScopeKind::Function {
                let Some(parent) = self.scopes[scope].parent else {
                    break;
                };
                scope = parent;
            }
        }
        scope
    }

    /// Whether the end of the unit reports what it left unused. GCC does that
    /// as it compiles the unit (`check_global_declaration` in `cgraphunit.cc`),
    /// so not under `-fsyntax-only`. Clang does under either, unless an error
    /// has been reported (`ActOnEndOfTranslationUnitFragment`).
    pub(super) fn reports_unused_at_unit_end(&self) -> bool {
        self.unused.tracking
            && if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
                !self.context.configuration.syntax_only()
            } else {
                self.context.error_count() == 0
            }
    }

    /// Reports file-scope objects with internal linkage: GCC at the last
    /// declaration, Clang at the first.
    /// C99: §6.2.2p3, p. 30; PDF p. 42.
    pub(super) fn report_unused_file_scope(&mut self) {
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        let mut sites: ArenaMap<'_, StringCacheId, usize> =
            ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut next = self.scope_entries[0].head.get();
        while let Some(link) = next {
            next = link.next;
            let entry = self.entries[link.value];
            if entry.namespace != Namespace::Ordinary {
                continue;
            }
            let data = self.bindings[entry.binding];
            if data.kind != BindingKind::Object || data.linkage != Linkage::Internal {
                continue;
            }
            let site = sites.entry(entry.name.name).or_insert(entry.binding);
            *site = if gcc {
                (*site).max(entry.binding)
            } else {
                (*site).min(entry.binding)
            };
        }
        let mut objects = ArenaVec::new_in(self.scratch);
        objects.extend(sites.values().copied());
        objects.sort_unstable();
        for binding in objects {
            let data = self.bindings[binding];
            let usage = self
                .unused
                .file_scope
                .get(&data.name.name)
                .copied()
                .unwrap_or_default();
            if usage.exempt || self.types.unanalyzed(data.ty) {
                continue;
            }
            let site = data.name.source_vectors;
            let constant = self.declares_constant(data.ty);
            if !usage.referenced {
                if constant {
                    if self.in_main_file(site) {
                        let rule = if gcc {
                            UnusedRule::GccFileConstant
                        } else {
                            UnusedRule::ClangFileConstant
                        };
                        self.unused_rule(rule, site, Some(data.name.name));
                    }
                } else if !gcc {
                    self.unused_rule(UnusedRule::Variable, site, Some(data.name.name));
                } else if !data.ty.is_volatile() {
                    self.unused_rule(UnusedRule::GccFileObject, site, Some(data.name.name));
                }
            } else if !gcc
                && usage.references == 0
                && self.in_main_file(site)
                && self
                    .context
                    .final_configuration()
                    .diagnostic_severity(G::UnusedButSetGlobal)
                    .is_some()
            {
                // Clang looks for these only if the group is on at the end of
                // the unit, and then classifies each at its declaration.
                self.unused_rule(UnusedRule::ButSetGlobal, site, Some(data.name.name));
            }
        }
    }

    /// Reports a defined internal function nothing refers to: GCC leaves an
    /// `inline` one alone and counts no reference from its own body, and
    /// Clang leaves one alone only outside the main file.
    pub(super) fn report_unused_function(&mut self, name: Identifier, site: SourceVectors) {
        let usage = self
            .unused
            .file_scope
            .get(&name.name)
            .copied()
            .unwrap_or_default();
        if usage.exempt {
            return;
        }
        if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
            if !usage.outside && !usage.inline {
                self.unused_rule(UnusedRule::GccFunction, site, Some(name.name));
            }
        } else if !usage.referenced && (!usage.inline || self.in_main_file(site)) {
            self.unused_rule(UnusedRule::ClangFunction, site, Some(name.name));
        }
    }

    /// Whether an `unused` attribute follows the label `statement` defines.
    pub(super) fn label_is_exempt(&self, statement: &'tu Statement<'tu>) -> bool {
        let StatementType::Label(_, child) = statement.kind else {
            return false;
        };
        let StatementType::Attributed(attributed) = child.kind else {
            return false;
        };
        super::attributes::known_names(attributed.attributes, self.context)
            .any(|(_, name)| matches!(name, "unused" | "maybe_unused"))
    }

    /// Reports a label nothing jumps to, defined at `definition`: GCC at the
    /// definition under the settings at the end of `scope`, and Clang at the
    /// name a local label's declaration gave it (`declared`) or else at the
    /// definition, under the settings there.
    pub(super) fn report_unused_label(
        &mut self,
        definition: Identifier,
        declared: Option<Identifier>,
        scope: usize,
    ) {
        if !self.unused.tracking {
            return;
        }
        if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
            let site = definition.source_vectors;
            let settings_at = self.scope_end_or(scope, site);
            self.unused_rule_read_at(
                UnusedRule::GccLabel,
                site,
                Some(definition.name),
                settings_at,
            );
        } else {
            let name = declared.unwrap_or(definition);
            self.unused_rule(UnusedRule::ClangLabel, name.source_vectors, Some(name.name));
        }
    }

    /// Records the statement that gives a statement expression its value,
    /// which discards nothing wherever the statement expression is.
    /// GNU extension: GCC manual, "Statement Exprs".
    pub(super) fn note_statement_expression_value(&mut self, statement: &'tu Statement<'tu>) {
        if self.unused.tracking || self.unused.values {
            _ = self
                .unused
                .statement_expression_values
                .insert(std::ptr::from_ref(statement).addr(), ());
        }
    }

    /// Schedules the check of the value `statement` discards for after its
    /// analysis.
    /// C99: §6.8.3p2, p. 132; PDF p. 144.
    pub(super) fn discarded_statement_value(
        &mut self,
        statement: &'tu Statement<'tu>,
        expression: &'tu Expression<'tu>,
    ) {
        if self
            .unused
            .statement_expression_values
            .contains_key(&std::ptr::from_ref(statement).addr())
        {
            return;
        }
        self.discarded_value(expression);
    }

    /// Schedules the check of a discarded value for after its analysis.
    /// C99: §6.8.3p2, p. 132; PDF p. 144; §6.8.5.3p1, p. 136; PDF p. 148.
    pub(super) fn discarded_value(&mut self, expression: &'tu Expression<'tu>) {
        if self.unused.tracking {
            self.note_discarded(expression);
        }
        if self.unused.values {
            self.work.push(super::Work::StatementWork(
                super::statements::StatementWork::DiscardedValue(
                    expression,
                    self.semantic_errors,
                    self.evaluation_inhibited(),
                ),
            ));
        }
    }

    /// Checks the expression whose value a statement discards, now analyzed
    /// without an error: neither compiler has anything to say about the value
    /// of an invalid expression. `inhibited` says it is an operand the program
    /// does not evaluate, which Clang can tell.
    /// C99: §6.8.3p2, p. 132; PDF p. 144.
    pub(super) fn check_discarded_value(&mut self, e: &'tu Expression<'tu>, inhibited: bool) {
        if self.context.configuration.compiler_identity().rules() == RuleSet::Clang {
            self.clang_discarded_value(e, inhibited);
        } else {
            self.gcc_discarded_value(e);
        }
    }

    /// GCC's `emit_side_effect_warnings`.
    fn gcc_discarded_value(&mut self, e: &'tu Expression<'tu>) {
        if self.is_void_or_unknown(e) {
            return;
        }
        if !self.computes_more(e, true) {
            self.unused_rule(UnusedRule::GccNoEffect, Self::gcc_statement_site(e), None);
            return;
        }
        self.gcc_unused_effectful_value(e);
    }

    /// What GCC says of a discarded expression with side effects, as a
    /// statement or a comma's left operand: of a comma, about its last right
    /// operand, and otherwise `warn_if_unused_value`.
    fn gcc_unused_effectful_value(&mut self, e: &'tu Expression<'tu>) {
        let e = Self::without_parentheses(e);
        if matches!(
            e.kind,
            E::Binary {
                operator: B::Comma,
                ..
            }
        ) {
            let mut comma = e;
            let mut right = e;
            while let E::Binary {
                operator: B::Comma,
                right_expression,
                ..
            } = right.kind
            {
                comma = right;
                right = Self::without_parentheses(right_expression);
            }
            if !self.computes_more(right, true)
                && !self.is_void_or_unknown(right)
                && !matches!(right.kind, E::Cast { .. })
            {
                self.unused_rule(UnusedRule::GccCommaRight, Self::operator_site(comma), None);
            }
        } else if self.computes_more(e, false) && self.gcc_value_is_unused(e) {
            self.unused_rule(UnusedRule::GccValueComputed, Self::operator_site(e), None);
        }
    }

    /// Where GCC reports a discarded expression with no effect: at its
    /// operator through parentheses, or, when it has no location of its own,
    /// at the statement's first token.
    fn gcc_statement_site(e: &'tu Expression<'tu>) -> SourceVectors {
        let inner = Self::without_parentheses(e);
        match inner.kind {
            | E::Identifier(_)
            | E::Constant(_)
            | E::StringLiteral(_)
            | E::SizeofExpr(_)
            | E::SizeofType(_)
            | E::AlignofExpr(_)
            | E::AlignofType(_)
            | E::Boolean(_)
            | E::Nullptr => Self::operator_site(e),
            | _ => Self::operator_site(inner),
        }
    }

    /// GCC's `warn_if_unused_value`, for an expression with side effects. It
    /// looks through `NON_LVALUE_EXPR` and `NOP_EXPR`, which a cast is unless
    /// it changes the representation, and a volatile read, an atomic read and
    /// a conditional have an effect of their own. The right operand of `&&`
    /// or `||` is compared with zero, unless it assigns.
    /// C99: §6.5.13p3, p. 89; PDF p. 101; §6.5.14p3, p. 89; PDF p. 101.
    fn gcc_value_is_unused(&self, e: &'tu Expression<'tu>) -> bool {
        if self.is_effectful_read(e) {
            return false;
        }
        match e.kind {
            | E::Parenthesized {
                expression: operand,
            }
            | E::Unary {
                operator: U::Plus,
                operand_expression: operand,
            } => self.gcc_value_is_unused(operand),
            | E::Cast {
                operand_expression, ..
            } => {
                // Folding drops a widening cast under another cast.
                let mut operand = Self::without_parentheses(operand_expression);
                while let E::Cast {
                    operand_expression: inner,
                    ..
                } = operand.kind
                {
                    let inner = Self::without_parentheses(inner);
                    if !self.gcc_cast_keeps_representation(
                        self.expression_info(operand).ty,
                        self.expression_info(inner).ty,
                    ) {
                        break;
                    }
                    operand = inner;
                }
                let (to, from) = (self.expression_info(e).ty, self.expression_info(operand).ty);
                if self.is_void_or_unknown(e) {
                    false
                } else if self.gcc_cast_keeps_representation(to, from) {
                    self.gcc_value_is_unused(operand)
                } else {
                    self.types.nodes[to.index] == TypeKind::Scalar(Scalar::Bool)
                        || !matches!(
                            operand.kind,
                            E::Conditional(_)
                                | E::Binary {
                                    operator: B::Assignment,
                                    ..
                                }
                        )
                }
            },
            | E::Binary {
                operator: B::LogicalAnd | B::LogicalOr,
                right_expression,
                ..
            } => {
                let right = Self::without_parentheses(right_expression);
                match right.kind {
                    | E::Binary {
                        operator: B::LogicalAnd | B::LogicalOr,
                        ..
                    } => self.gcc_value_is_unused(right),
                    | E::Binary { operator, .. } if Self::is_assignment(operator) => false,
                    | E::Binary {
                        operator: B::Comma, ..
                    }
                    | E::Conditional(_) => false,
                    | _ => true,
                }
            },
            | E::Binary { operator, .. } => !Self::is_assignment(operator) && operator != B::Comma,
            | E::Unary { operator, .. } => matches!(
                operator,
                U::Minus | U::BitwiseNot | U::LogicalNot | U::Indirection
            ),
            | E::DirectMember { .. } | E::IndirectMember { .. } => true,
            | _ => false,
        }
    }

    /// Clang's `DiagnoseUnusedExprResult`, which says nothing about an
    /// expression a macro's body wrote and, comparisons aside, which report at
    /// once, about one it does not reach (`DiagIfReachable`).
    fn clang_discarded_value(&mut self, e: &'tu Expression<'tu>, inhibited: bool) {
        let comparison = match Self::without_parentheses(e).kind {
            | E::Binary {
                operator: B::Equal, ..
            } => Some(UnusedRule::ClangEquality),
            | E::Binary {
                operator: B::NotEqual,
                ..
            } => Some(UnusedRule::ClangInequality),
            | E::Binary {
                operator: B::LessThan | B::GreaterThan | B::LessThanOrEqual | B::GreaterThanOrEqual,
                ..
            } => Some(UnusedRule::ClangRelational),
            | _ => None,
        };
        if inhibited && comparison.is_none() {
            return;
        }
        if let Some((rule, site, anchor)) = self.clang_unused_result(e)
            && !self
                .context
                .written_by_macro_body(Self::operator_site(Self::without_parentheses(e)))
        {
            if let Some(comparison) = comparison {
                self.unused_rule(comparison, site, None);
            } else {
                self.warn_if_reachable(
                    anchor,
                    rule.group(),
                    SemanticErrorKind::Unused(rule),
                    site,
                    None,
                );
            }
        }
    }

    /// Clang's `Expr::isUnusedResultAWarning`: the rule and place of the
    /// warning, if discarding `e` deserves one, and the expression it is about
    /// (`WarnE`), whose reachability decides: of a conditional, the last arm
    /// it read, so `1 ? a : b;` is silent and `0 ? a : b;` is not.
    fn clang_unused_result(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Option<(UnusedRule, SourceVectors, &'tu Expression<'tu>)> {
        let e = Self::without_parentheses(e);
        if self.is_void_or_unknown(e) {
            return None;
        }
        let result =
            |e: &'tu Expression<'tu>| Some((UnusedRule::ClangResult, Self::operator_site(e), e));
        match e.kind {
            | E::Unary {
                operator: U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement,
                ..
            } => None,
            | E::Unary {
                operator: U::Extension,
                operand_expression,
            } => self.clang_unused_result(operand_expression),
            | E::Binary { operator, .. } if Self::is_assignment(operator) => None,
            | E::Binary {
                operator: B::Comma,
                right_expression,
                ..
            } => {
                // `(a = b, 0)` hides an assignment's result in a macro.
                if matches!(
                    Self::without_parentheses(right_expression).kind,
                    E::Constant(_)
                ) && self
                    .expression_info(right_expression)
                    .integer
                    .is_some_and(Integer::is_zero)
                {
                    None
                } else {
                    self.clang_unused_result(right_expression)
                }
            },
            | E::Binary {
                operator: B::LogicalAnd | B::LogicalOr,
                left_expression,
                right_expression,
            } => {
                _ = self.clang_unused_result(left_expression)?;
                _ = self.clang_unused_result(right_expression)?;
                result(e)
            },
            | E::Conditional(c) => {
                _ = self.clang_unused_result(c.then_expression)?;
                self.clang_unused_result(c.else_expression)
            },
            | E::Cast { .. } => {
                let ty = self.expression_info(e).ty;
                if let TypeKind::Pointer(target) = self.types.nodes[ty.index]
                    && ty.qualifiers.is_empty()
                    && target.qualifiers.is_empty()
                    && self.types.nodes[target.index] == TypeKind::Scalar(Scalar::Void)
                {
                    Some((UnusedRule::ClangVoidPointerCast, Self::operator_site(e), e))
                } else {
                    result(e)
                }
            },
            | E::Identifier(_)
            | E::Constant(_)
            | E::StringLiteral(_)
            | E::SizeofExpr(_)
            | E::SizeofType(_)
            | E::AlignofExpr(_)
            | E::AlignofType(_)
            | E::Boolean(_)
            | E::Nullptr
            | E::Binary { .. }
            | E::Unary { .. }
            | E::DirectMember { .. }
            | E::IndirectMember { .. } =>
                if self.expression_info(e).ty.is_volatile() {
                    None
                } else {
                    result(e)
                },
            | _ => None,
        }
    }

    /// Marks the commas between the arguments of the attribute expression
    /// `argument`.
    pub(in crate::translation_phases::semantic_analysis) fn note_attribute_argument(
        &mut self,
        mut argument: &'tu Expression<'tu>,
    ) {
        if self.unused.values {
            while let E::Binary {
                operator: B::Comma,
                left_expression,
                ..
            } = argument.kind
            {
                _ = self
                    .unused
                    .separators
                    .insert(std::ptr::from_ref(argument).addr(), ());
                argument = left_expression;
            }
        }
    }

    /// A comma's left operand, whose value no context needs.
    /// C99: §6.5.17p2, p. 94; PDF p. 106.
    pub(super) fn check_comma_left(
        &mut self,
        left: &'tu Expression<'tu>,
        comma: &'tu Expression<'tu>,
    ) {
        if self.functions.sizeof.is_some()
            || self
                .unused
                .separators
                .contains_key(&std::ptr::from_ref(comma).addr())
        {
            return;
        }
        if self.context.configuration.compiler_identity().rules() == RuleSet::Clang {
            if !self.evaluation_inhibited()
                && let Some((_, _, anchor)) = self.clang_unused_result(left)
                && !self
                    .context
                    .written_by_macro_body(Self::operator_site(Self::without_parentheses(left)))
            {
                let rule = UnusedRule::ClangCommaLeft;
                self.warn_if_reachable(
                    anchor,
                    rule.group(),
                    SemanticErrorKind::Unused(rule),
                    Self::operator_site(left),
                    None,
                );
            }
        } else if self.is_void_or_unknown(left) {
        } else if !self.computes_more(left, true) {
            self.unused_rule(UnusedRule::GccCommaLeft, Self::operator_site(comma), None);
        } else {
            self.gcc_unused_effectful_value(left);
        }
    }

    /// Whether evaluating `e` does anything but compute its value: an
    /// assignment, an update, a call, a volatile access, an atomic read, or a
    /// construct this does not read: GCC's `TREE_SIDE_EFFECTS`. An operand of
    /// `&` is a place and not read. A `sizeof` of a variable length array
    /// evaluates its operand and a cast to a variably modified type its
    /// bounds, which GCC keeps quiet about however it is used;
    /// `evaluated_types` counts those.
    /// C99: §5.1.2.3p2, p. 13; PDF p. 25; §6.5.3.2p3, p. 78; PDF p. 90;
    /// §6.5.3.4p2, p. 80; PDF p. 92; §6.7.5.2p4-5, p. 117; PDF p. 129.
    pub(super) fn computes_more(&self, e: &'tu Expression<'tu>, evaluated_types: bool) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((e, true));
        while let Some((e, read)) = work.pop() {
            if read && self.is_effectful_read(e) {
                return true;
            }
            match e.kind {
                | E::Parenthesized { expression } => work.push((expression, read)),
                | E::Unary {
                    operator: U::AddressOf,
                    operand_expression,
                } => work.push((operand_expression, false)),
                | E::Unary {
                    operator: U::Indirection,
                    operand_expression,
                } => work.push((operand_expression, true)),
                | E::Unary {
                    operator,
                    operand_expression,
                } => {
                    if matches!(
                        operator,
                        U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement
                    ) {
                        return true;
                    }
                    work.push((operand_expression, true));
                },
                | E::Binary {
                    operator,
                    left_expression,
                    right_expression,
                } => {
                    if Self::is_assignment(operator) {
                        return true;
                    }
                    work.push((left_expression, true));
                    work.push((right_expression, true));
                },
                | E::Cast {
                    operand_expression, ..
                } => {
                    if evaluated_types && self.types.variably_modified(self.expression_info(e).ty) {
                        return true;
                    }
                    work.push((operand_expression, true));
                },
                | E::SizeofExpr(_) | E::SizeofType(_) =>
                    if evaluated_types && !self.expression_info(e).ice {
                        return true;
                    },
                | E::DirectMember {
                    base_expression, ..
                } => work.push((base_expression, false)),
                | E::IndirectMember {
                    base_expression, ..
                } => work.push((base_expression, true)),
                | E::Conditional(c) => {
                    work.push((c.condition_expression, true));
                    work.push((c.then_expression, true));
                    work.push((c.else_expression, true));
                },
                | E::Identifier(_)
                | E::Constant(_)
                | E::StringLiteral(_)
                | E::AlignofExpr(_)
                | E::AlignofType(_)
                | E::Boolean(_)
                | E::Nullptr => {},
                | _ => return true,
            }
        }
        false
    }

    /// Whether a cast from `from` to `to` is a `NOP_EXPR`: to an integer type
    /// no narrower, or between a pointer and an integer of its size.
    /// C99: §6.3.1.3p1, p. 43; PDF p. 55; §6.3.2.3p5-6, p. 47; PDF p. 59.
    fn gcc_cast_keeps_representation(&self, to: TypeId, from: TypeId) -> bool {
        let pointer = |ty: TypeId| matches!(self.types.nodes[ty.index], TypeKind::Pointer(_));
        let width = |ty: TypeId| match self.types.nodes[self.types.non_atomic(ty).index] {
            | TypeKind::Pointer(_) => Some(self.types.target.pointer.size * 8),
            | TypeKind::Scalar(Scalar::Bool) => None,
            | _ => self.integer_type(ty).map(|(bits, _)| u64::from(bits)),
        };
        width(to)
            .zip(width(from))
            .is_some_and(|(to_bits, from_bits)| {
                if pointer(to) {
                    to_bits == from_bits
                } else {
                    to_bits >= from_bits
                }
            })
    }

    /// Whether reading the lvalue `e` is a side effect of its own: a volatile
    /// access, or for GCC `really_atomic_lvalue`, whose read is a call to
    /// `__atomic_load` unless a `register` object's address cannot be taken.
    /// C99: §6.7.3p6, p. 109; PDF p. 121; C11: §6.2.6.1p9, p. 45; PDF p. 63.
    pub(super) fn is_effectful_read(&self, e: &'tu Expression<'tu>) -> bool {
        let info = self.expression_info(e);
        if matches!(info.category, ValueCategory::Rvalue) {
            return false;
        }
        info.ty.is_volatile()
            || (matches!(self.types.nodes[info.ty.index], TypeKind::Atomic(_)) && !info.register)
    }

    pub(super) const fn is_assignment(operator: B) -> bool {
        matches!(
            operator,
            B::Assignment
                | B::MultiplicationAssignment
                | B::DivisionAssignment
                | B::ModuloAssignment
                | B::AdditionAssignment
                | B::SubtractionAssignment
                | B::LeftShiftAssignment
                | B::RightShiftAssignment
                | B::BitwiseAndAssignment
                | B::BitwiseXorAssignment
                | B::BitwiseOrAssignment
        )
    }

    fn is_void_or_unknown(&self, e: &'tu Expression<'tu>) -> bool {
        let ty = self.expression_info(e).ty;
        self.types.unanalyzed(ty) || self.types.nodes[ty.index] == TypeKind::Scalar(Scalar::Void)
    }

    pub(super) fn without_parentheses(mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
        while let E::Parenthesized { expression } = e.kind {
            e = expression;
        }
        e
    }

    fn operator_site(e: &'tu Expression<'tu>) -> SourceVectors {
        e.operator_source_vectors.unwrap_or(e.source_vectors)
    }

    /// Whether the type a declaration gives is `const`, an array's elements'
    /// included.
    fn declares_constant(&self, mut ty: TypeId) -> bool {
        while let TypeKind::Array(element, _) = self.types.nodes[ty.index] {
            ty = element;
        }
        ty.qualifiers.contains(TypeQualifiers::CONST)
    }

    fn in_main_file(&self, site: SourceVectors) -> bool {
        self.context.in_main_file(site)
    }

    /// Reports one warning. GCC's label warning goes by the invocation; its
    /// others go by the token's spelling too.
    fn unused_rule(&mut self, rule: UnusedRule, site: SourceVectors, name: Option<StringCacheId>) {
        self.unused_rule_read_at(rule, site, name, site);
    }

    /// [`Self::unused_rule`] under the settings at `settings_at`.
    fn unused_rule_read_at(
        &mut self,
        rule: UnusedRule,
        site: SourceVectors,
        name: Option<StringCacheId>,
        settings_at: SourceVectors,
    ) {
        let kind = SemanticErrorKind::Unused(rule);
        if rule == UnusedRule::GccLabel {
            self.group_diagnostic_read_at(rule.group(), kind, site, name, None, settings_at);
        } else {
            self.spelled_group_diagnostic(rule.group(), kind, site, name, settings_at);
        }
    }
}
