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
            SpecifierExtension,
            SpecifierExtensionKind,
            declaration_syntax::{
                DeclarationSpecifiers,
                StructOrUnionSpecifier,
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
    /// `constructor`, `destructor` or being an alias's target.
    exempt:     bool,
    /// A function declared `inline`.
    inline:     bool,
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

/// Working state for the warnings this module implements.
pub(super) struct UnusedState<'s> {
    /// Whether a warning that needs identifier uses recorded is on.
    pub(super) tracking: bool,
    /// Whether a warning about discarded values is on, as Clang's is by
    /// default.
    pub(super) values:   bool,
    variables:           ArenaVec<'s, Usage>,
    file_scope:          ArenaMap<'s, StringCacheId, Usage>,
    writes:              ArenaMap<'s, usize, WriteSite>,
    /// The commas that separate an attribute's arguments, which bcc parses as
    /// one expression: their left operands are arguments, not discarded.
    separators:          ArenaMap<'s, usize, ()>,
    /// Tags declared `unused`, whose variables both compilers exempt.
    exempt_tags:         ArenaMap<'s, StringCacheId, ()>,
}

impl<'s> UnusedState<'s> {
    pub(super) fn new(scratch: &'s Bump, configuration: CompilerConfiguration) -> Self {
        let on = |groups: &[G]| {
            !matches!(
                configuration.compiler_identity(),
                CompilerIdentity::Tcc | CompilerIdentity::Msvc
            ) && groups
                .iter()
                .any(|&group| configuration.diagnostic_severity(group).is_some())
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
            variables: ArenaVec::new_in(scratch),
            file_scope: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            writes: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            separators: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            exempt_tags: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
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

    /// Exempts the declaration of `binding` when its specifiers name a type
    /// whose typedef is `unused`, as both compilers do.
    pub(super) fn note_declared_type(
        &mut self,
        binding: usize,
        spec: DeclarationSpecifiers<'tu>,
        kind: BindingKind,
        name: Identifier,
    ) {
        if kind == BindingKind::Function && spec.function_specifiers.is_inline {
            self.unused.file_scope.entry(name.name).or_default().inline = true;
        }
        if let TypeSpecifiers::StructOrUnion(record) = spec.type_specifiers
            && self.attribute_chain_exempts(record.attributes)
        {
            self.unused.variable(binding).exempt = true;
        }
        if let TypeSpecifiers::TypedefName(typedef) = spec.type_specifiers
            && let Some(entry) = self.lookup(Namespace::Ordinary, typedef.name)
            && self
                .unused
                .variables
                .get(entry.binding)
                .is_some_and(|usage| usage.exempt)
        {
            self.unused.variable(binding).exempt = true;
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

    /// A function an `alias` attribute names.
    pub(super) fn exempt_alias_target(&mut self, name: &str) {
        let name = self.context.string_cache.intern(name);
        self.unused.file_scope.entry(name).or_default().exempt = true;
    }

    /// A structure or union type specifier: one declared `unused` exempts
    /// its variables, which name it however they like.
    pub(super) fn note_record_specifier(&mut self, record: &StructOrUnionSpecifier<'tu>) {
        if let Some(name) = record.identifier
            && self.attribute_chain_exempts(record.attributes)
        {
            _ = self.unused.exempt_tags.insert(name.name, ());
        }
    }

    fn attribute_chain_exempts(&self, mut chain: Option<&SpecifierExtension<'tu>>) -> bool {
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

    /// Whether `ty`, or its elements, is a tag declared `unused`.
    fn type_is_exempt(&self, mut ty: TypeId) -> bool {
        while let TypeKind::Array(element, _) = self.types.nodes[ty.index] {
            ty = element;
        }
        match self.types.nodes[ty.index] {
            | TypeKind::Tag(id) => self.types.tags[id]
                .name
                .is_some_and(|name| self.unused.exempt_tags.contains_key(&name)),
            | _ => false,
        }
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
        if data.linkage == Linkage::Internal
            || self.types.unanalyzed(data.ty)
            || self.type_is_exempt(data.ty)
        {
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
        if !gcc && data.linkage == Linkage::External {
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
        self.unused_rule(rule, name.source_vectors, Some(name.name));
    }

    /// Reports file-scope objects with internal linkage: GCC at the last
    /// declaration, Clang at the first.
    /// C99: §6.2.2p3, p. 30; PDF p. 42.
    pub(super) fn report_unused_file_scope(&mut self) {
        if !self.unused.tracking {
            return;
        }
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
            if usage.exempt || self.types.unanalyzed(data.ty) || self.type_is_exempt(data.ty) {
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
            } else if !gcc && usage.references == 0 && self.in_main_file(site) {
                self.unused_rule(UnusedRule::ButSetGlobal, site, Some(data.name.name));
            }
        }
    }

    /// Reports a defined internal function nothing refers to: GCC leaves an
    /// `inline` one alone and counts no reference from its own body, and
    /// Clang leaves one alone only outside the main file.
    pub(super) fn report_unused_function(&mut self, name: Identifier, site: SourceVectors) {
        if !self.unused.tracking {
            return;
        }
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

    /// Reports a label nothing jumps to.
    pub(super) fn report_unused_label(&mut self, name: Identifier) {
        if self.unused.tracking {
            let rule = if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
                UnusedRule::GccLabel
            } else {
                UnusedRule::ClangLabel
            };
            self.unused_rule(rule, name.source_vectors, Some(name.name));
        }
    }

    /// Schedules the check of a statement's discarded value for after its
    /// analysis.
    /// C99: §6.8.3p2, p. 132; PDF p. 144.
    pub(super) fn discarded_statement_value(&mut self, expression: &'tu Expression<'tu>) {
        if self.unused.tracking {
            self.note_discarded(expression);
        }
        if self.unused.values {
            self.work.push(super::Work::StatementWork(
                super::statements::StatementWork::DiscardedValue(expression, self.semantic_errors),
            ));
        }
    }

    /// Checks the expression whose value a statement discards, now analyzed
    /// without an error: neither compiler has anything to say about the value
    /// of an invalid expression.
    /// C99: §6.8.3p2, p. 132; PDF p. 144.
    pub(super) fn check_discarded_value(&mut self, e: &'tu Expression<'tu>) {
        if self.context.configuration.compiler_identity().rules() == RuleSet::Clang {
            self.clang_discarded_value(e);
        } else {
            self.gcc_discarded_value(e);
        }
    }

    /// GCC's `emit_side_effect_warnings`.
    fn gcc_discarded_value(&mut self, e: &'tu Expression<'tu>) {
        if self.is_void_or_unknown(e) {
            return;
        }
        if !self.computes_more(e) {
            self.unused_rule(UnusedRule::GccNoEffect, Self::operator_site(e), None);
            return;
        }
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
            if !self.computes_more(right)
                && !self.is_void_or_unknown(right)
                && !matches!(right.kind, E::Cast { .. })
            {
                self.unused_rule(UnusedRule::GccCommaRight, Self::operator_site(comma), None);
            }
        } else if self.gcc_value_is_unused(e) {
            self.unused_rule(UnusedRule::GccValueComputed, Self::operator_site(e), None);
        }
    }

    /// GCC's `warn_if_unused_value`, for an expression with side effects:
    /// the conservative part of it, which leaves out a cast and a conditional.
    fn gcc_value_is_unused(&self, e: &'tu Expression<'tu>) -> bool {
        if self.expression_info(e).ty.is_volatile() {
            return false;
        }
        match e.kind {
            | E::Binary {
                operator: B::LogicalAnd | B::LogicalOr,
                right_expression,
                ..
            } => matches!(
                Self::without_parentheses(right_expression).kind,
                E::Call { .. } | E::Identifier(_) | E::Constant(_)
            ),
            | E::Binary { operator, .. } => !Self::is_assignment(operator) && operator != B::Comma,
            | E::Unary { operator, .. } => matches!(
                operator,
                U::Plus | U::Minus | U::BitwiseNot | U::LogicalNot | U::Indirection
            ),
            | E::DirectMember { .. } | E::IndirectMember { .. } => true,
            | _ => false,
        }
    }

    /// Clang's `DiagnoseUnusedExprResult`, which says nothing about an
    /// expression a macro's body wrote.
    fn clang_discarded_value(&mut self, e: &'tu Expression<'tu>) {
        if let Some((rule, site)) = self.clang_unused_result(e)
            && !self
                .context
                .written_by_macro_body(Self::operator_site(Self::without_parentheses(e)))
        {
            let rule = match Self::without_parentheses(e).kind {
                | E::Binary {
                    operator: B::Equal, ..
                } => UnusedRule::ClangEquality,
                | E::Binary {
                    operator: B::NotEqual,
                    ..
                } => UnusedRule::ClangInequality,
                | E::Binary {
                    operator:
                        B::LessThan | B::GreaterThan | B::LessThanOrEqual | B::GreaterThanOrEqual,
                    ..
                } => UnusedRule::ClangRelational,
                | _ => rule,
            };
            self.unused_rule(rule, site, None);
        }
    }

    /// Clang's `Expr::isUnusedResultAWarning`: the rule and place of the
    /// warning, if discarding `e` deserves one.
    fn clang_unused_result(&self, e: &'tu Expression<'tu>) -> Option<(UnusedRule, SourceVectors)> {
        let e = Self::without_parentheses(e);
        if self.is_void_or_unknown(e) {
            return None;
        }
        let result =
            |e: &'tu Expression<'tu>| Some((UnusedRule::ClangResult, Self::operator_site(e)));
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
                    Some((UnusedRule::ClangVoidPointerCast, Self::operator_site(e)))
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
            if self.clang_unused_result(left).is_some()
                && !self
                    .context
                    .written_by_macro_body(Self::operator_site(Self::without_parentheses(left)))
            {
                self.unused_rule(UnusedRule::ClangCommaLeft, Self::operator_site(left), None);
            }
        } else if !self.is_void_or_unknown(left) && !self.computes_more(left) {
            self.unused_rule(UnusedRule::GccCommaLeft, Self::operator_site(comma), None);
        }
    }

    /// Whether evaluating `e` does anything but compute its value: an
    /// assignment, an update, a call, a volatile access, or a construct this
    /// does not read.
    /// C99: §5.1.2.3p2, p. 13; PDF p. 25.
    fn computes_more(&self, e: &'tu Expression<'tu>) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push(e);
        while let Some(e) = work.pop() {
            match e.kind {
                | E::Parenthesized { expression } => work.push(expression),
                | E::Unary {
                    operator,
                    operand_expression,
                } => {
                    if matches!(
                        operator,
                        U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement
                    ) || self.expression_info(e).ty.is_volatile()
                    {
                        return true;
                    }
                    work.push(operand_expression);
                },
                | E::Binary {
                    operator,
                    left_expression,
                    right_expression,
                } => {
                    if Self::is_assignment(operator) || self.expression_info(e).ty.is_volatile() {
                        return true;
                    }
                    work.push(left_expression);
                    work.push(right_expression);
                },
                | E::Cast {
                    operand_expression, ..
                } => work.push(operand_expression),
                | E::DirectMember {
                    base_expression, ..
                }
                | E::IndirectMember {
                    base_expression, ..
                } => {
                    if self.expression_info(e).ty.is_volatile() {
                        return true;
                    }
                    work.push(base_expression);
                },
                | E::Conditional(c) => {
                    work.push(c.condition_expression);
                    work.push(c.then_expression);
                    work.push(c.else_expression);
                },
                | E::Identifier(_) =>
                    if self.expression_info(e).ty.is_volatile() {
                        return true;
                    },
                | E::Constant(_)
                | E::StringLiteral(_)
                | E::SizeofExpr(_)
                | E::SizeofType(_)
                | E::AlignofExpr(_)
                | E::AlignofType(_)
                | E::Boolean(_)
                | E::Nullptr => {},
                | _ => return true,
            }
        }
        false
    }

    const fn is_assignment(operator: B) -> bool {
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

    fn without_parentheses(mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
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

    fn unused_rule(&mut self, rule: UnusedRule, site: SourceVectors, name: Option<StringCacheId>) {
        self.group_diagnostic(
            rule.group(),
            SemanticErrorKind::Unused(rule),
            site,
            name,
            None,
        );
    }
}
