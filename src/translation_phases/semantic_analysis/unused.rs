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
    ReportOrder,
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
        Context,
        ReadingPosition,
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
                ExpressionSlot,
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
    referenced:  bool,
    /// GCC's `DECL_READ_P`.
    read:        bool,
    /// Clang's `RefsMinusAssignments`, whose subtraction happens by not
    /// counting the reference that an assignment discards.
    references:  u32,
    /// A function's references outside its own body, which GCC counts.
    outside:     bool,
    /// `unused`, `maybe_unused`, `used`, `cleanup`, or for a function
    /// `constructor`, `destructor` or being a symbol attribute's target.
    exempt:      bool,
    /// A function declared `inline`.
    inline:      bool,
    /// A function declared GCC's `weakref`, which is defined elsewhere.
    weakref:     bool,
    /// What the name's type says, as [`Analyzer::type_mark`] reads it.
    marked:      bool,
    /// A local object declared with an initializer, which `cl` counts as no
    /// reference.
    initialized: bool,
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
    /// Whether the warnings follow `cl`'s rules: under its identity, only
    /// those it numbers and bcc gives its code (C4100, C4101, C4189, C4018,
    /// C4389, C4706) are on.
    pub(super) cl: bool,
    /// The warnings `cl` has shown under a setting that shows them once, one
    /// bit per index of [`crate::configuration::CL_CODED_WARNINGS`].
    pub(super) cl_once: u32,
    /// Whether a warning about discarded values is on, as Clang's is by
    /// default.
    pub(super) values: bool,
    /// Whether a warning about a comparison the types decide may be on.
    pub(super) type_limits: bool,
    /// Whether `-Wbool-operation` may be on.
    pub(super) bool_operation: bool,
    /// Whether a warning about a hidden declaration may be on.
    pub(super) shadow: bool,
    /// Whether `-Wempty-body` may be on.
    pub(super) empty_body: bool,
    /// Whether a warning about operators' grouping may be on.
    pub(super) parentheses: bool,
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
    /// rules: see [`Analyzer::note_scope_end`].
    scope_ends: Option<ArenaMap<'s, usize, SourceVectors>>,
    /// GCC's semantic errors so far that a warning group reports, which
    /// leave out the warnings `-Werror` makes errors (other semantic errors
    /// count in `Analyzer::hard_errors`).
    pub(super) gcc_errors: usize,
    /// Where GCC's first error before semantic analysis was read: GCC counts
    /// it once it has read that far.
    prior_error: Option<ReadingPosition>,
    /// Where the function GCC finished last ended.
    function_end: Option<ReadingPosition>,
    /// The functions being analyzed under GCC's rules, innermost last.
    gcc_functions: ArenaVec<'s, GccFunction>,
    /// The typedefs the left blocks of those functions declared, which GCC
    /// reports at the end of the function.
    local_typedefs: ArenaVec<'s, (usize, Identifier)>,
    /// What GCC's middle end says of reaching a function's end, which it
    /// reports once the unit is parsed: the group, the place and the words.
    late_return_flow: ArenaVec<'s, (G, SourceVectors, &'static str)>,
    /// What `cl`'s code generator says of the same: the function's binding,
    /// its closing brace, and whether it has no `return` with a value
    /// (C4716).
    cl_return_flow: ArenaVec<'s, (usize, SourceVectors, bool)>,
    /// The binding that defines each internal function, where GCC's symbol
    /// table gains the function.
    function_symbols: ArenaMap<'s, StringCacheId, usize>,
    /// The variables `cl` reports at the end of the function, from the
    /// blocks left so far: each block's own before those of the blocks it
    /// holds, in the order the blocks begin.
    cl_pending: ArenaVec<'s, (usize, Identifier)>,
}

/// Where a function's state begins in what [`UnusedState`] keeps for GCC.
#[derive(Clone, Copy)]
struct GccFunction {
    /// The scope of its parameters and body.
    scope:    usize,
    /// Its labels, and those of the functions it nests, from here on in the
    /// analyzer's labels.
    labels:   usize,
    /// Its typedefs from here on in [`UnusedState::local_typedefs`].
    typedefs: usize,
}

impl<'s> UnusedState<'s> {
    /// `pragmas` says a diagnostic pragma may turn on what the command line
    /// leaves off, so that only the report can tell.
    /// `prior_error` is where the first error GCC counts that comes before
    /// semantic analysis was read.
    pub(super) fn new(
        scratch: &'s Bump,
        configuration: CompilerConfiguration,
        pragmas: bool,
        prior_error: Option<ReadingPosition>,
    ) -> Self {
        let enabled = |groups: &[G]| {
            pragmas
                || groups
                    .iter()
                    .any(|&group| configuration.diagnostic_severity(group).is_some())
        };
        let cl = configuration.compiler_identity() == CompilerIdentity::Msvc;
        let on = |groups: &[G]| {
            !matches!(
                configuration.compiler_identity(),
                CompilerIdentity::Tcc | CompilerIdentity::Msvc
            ) && enabled(groups)
        };
        let cl_on = |groups: &[G]| cl && enabled(groups);
        let tracking = on(&[
            G::UnusedVariable,
            G::UnusedConstVariable,
            G::UnusedParameter,
            G::UnusedButSetVariable,
            G::UnusedButSetGlobal,
            G::UnusedButSetParameter,
            G::UnusedLabel,
            G::UnusedFunction,
            G::UnusedLocalTypedefs,
            G::UnusedLocalTypedef,
        ]) || cl_on(&[
            G::UnusedVariable,
            G::UnusedParameter,
            G::UnusedButSetVariable,
        ]);
        let values = on(&[G::UnusedValue, G::UnusedComparison]);
        Self {
            tracking,
            cl,
            cl_once: 0,
            values,
            type_limits: on(&[
                G::TypeLimits,
                G::TautologicalConstantCompare,
                G::TautologicalConstantOutOfRangeCompare,
                G::TautologicalUnsignedZeroCompare,
                G::TautologicalTypeLimitCompare,
            ]),
            shadow: on(&[G::Shadow, G::ShadowLocal, G::ShadowCompatibleLocal]),
            empty_body: on(&[G::EmptyBody]),
            parentheses: on(&[
                G::Parentheses,
                G::ChainedComparison,
                G::LogicalOpParentheses,
                G::BitwiseOpParentheses,
                G::ShiftOpParentheses,
                G::ParenthesesEquality,
                G::BitwiseConditionalParentheses,
                G::LogicalNotParentheses,
                G::DanglingElse,
                G::SwitchBool,
            ]) || cl_on(&[G::Parentheses]),
            switches: on(&[
                G::Switch,
                G::SwitchEnum,
                G::SwitchDefault,
                G::SwitchOutsideRange,
            ]),
            // TCC warns of a reachable end by default, as its code
            // generator finds it.
            return_flow: on(&[G::ReturnType, G::InvalidNoreturn, G::GccNoreturn])
                || (configuration.compiler_identity() == CompilerIdentity::Tcc
                    && configuration.diagnostic_severity(G::ReturnType).is_some())
                || cl,
            sign_compare: on(&[G::SignCompare]) || cl_on(&[G::SignCompare, G::SignCompareEquality]),
            bool_operation: on(&[G::BoolOperation]),
            missing_fields: on(&[G::MissingFieldInitializers]),
            last_tag: None,
            variables: ArenaVec::new_in(scratch),
            file_scope: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            writes: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            statement_expression_values: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            separators: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            unused_tags: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            scope_ends: (tracking && configuration.compiler_identity() == CompilerIdentity::Gcc)
                .then(|| ArenaMap::with_hasher_in(FxBuildHasher, scratch)),
            gcc_errors: 0,
            prior_error,
            function_end: None,
            gcc_functions: ArenaVec::new_in(scratch),
            local_typedefs: ArenaVec::new_in(scratch),
            late_return_flow: ArenaVec::new_in(scratch),
            cl_return_flow: ArenaVec::new_in(scratch),
            function_symbols: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            cl_pending: ArenaVec::new_in(scratch),
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
    /// GCC's words for a typedef a block declares and never uses.
    GccLocalTypedef,
    /// Clang's.
    ClangLocalTypedef,
    /// `cl`'s C4100: a parameter nothing names.
    ClParameter,
    /// `cl`'s C4101: a local variable nothing names.
    ClVariable,
    /// `cl`'s C4189: a local variable only its initializer names.
    ClInitialized,
}

impl UnusedRule {
    pub(super) const fn group(self) -> G {
        match self {
            | Self::Variable | Self::GccFileObject | Self::ClVariable => G::UnusedVariable,
            | Self::GccFileConstant | Self::ClangFileConstant => G::UnusedConstVariable,
            | Self::Parameter | Self::ClParameter => G::UnusedParameter,
            | Self::ButSetVariable | Self::ClInitialized => G::UnusedButSetVariable,
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
            | Self::GccLocalTypedef => G::UnusedLocalTypedefs,
            | Self::ClangLocalTypedef => G::UnusedLocalTypedef,
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
            | Self::GccLocalTypedef => ("typedef", "locally defined but not used"),
            | Self::ClangLocalTypedef => ("unused typedef", ""),
            // `cl`'s own words are `Analyzer::msvc_message`'s.
            | Self::ClParameter => ("", "unreferenced parameter"),
            | Self::ClVariable => ("", "unreferenced local variable"),
            | Self::ClInitialized => ("", "local variable is initialized but not referenced"),
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
        // A typedef declared again in its block: GCC merges the two and says
        // nothing of either, and Clang counts the first as used.
        if kind == BindingKind::Typedef
            && self.unused.tracking
            && let Some(prior) = self.lookup(Namespace::Ordinary, name.name)
            && prior.scope == self.scope
            && self.bindings[prior.binding].kind == BindingKind::Typedef
        {
            self.unused.variable(prior.binding).referenced = true;
            if gcc {
                self.unused.variable(binding).referenced = true;
            }
        }
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
    /// Clang's `Sema::ActOnPopScope` reports a scope's variables and
    /// typedefs only when no unrecoverable error occurred in it, so an
    /// undeclared name silences them where a warning `-Werror` promotes
    /// does not; its parameters are reported with the function.
    pub(super) fn report_unused_in_scope(&mut self) {
        if self.scope == 0 || self.scopes[self.scope].kind == ScopeKind::Prototype {
            return;
        }
        if self.unused.cl {
            self.leave_scope_as_cl();
            return;
        }
        if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
            let order = self.scope_order(self.scope);
            self.with_report_order(order, Self::pop_scope_as_gcc);
            return;
        }
        let trapped = self.context.configuration.compiler_identity().rules() == RuleSet::Clang
            && self.hard_errors != self.scope_hard_errors[self.scope];
        let mut next = self.scope_entries[self.scope].head.get();
        while let Some(link) = next {
            next = link.next;
            let entry = self.entries[link.value];
            if entry.namespace == Namespace::Ordinary
                && !(trapped && self.bindings[entry.binding].kind != BindingKind::Parameter)
            {
                self.report_unused_variable(entry.binding, entry.name);
            }
        }
    }

    /// `cl`'s report of a function's unreferenced parameters and local
    /// variables, which waits for the end of the function, and stops after
    /// the unit's first error: its parameters
    /// and outermost block's variables, then each inner block's, in the
    /// order the blocks begin. `cl` orders the names of one block by its
    /// symbol table, which bcc does not model; bcc keeps their order.
    /// Microsoft Learn: "Compiler Warning (level 4) C4100", "Compiler
    /// Warning (level 3) C4101", "Compiler Warning (level 4) C4189".
    /// C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42.
    fn leave_scope_as_cl(&mut self) {
        let scope = self.scope;
        let mut own = ArenaVec::new_in(self.scratch);
        let mut next = self.scope_entries[scope].head.get();
        while let Some(link) = next {
            next = link.next;
            let entry = self.entries[link.value];
            if entry.namespace == Namespace::Ordinary
                && matches!(
                    self.bindings[entry.binding].kind,
                    BindingKind::Object | BindingKind::Parameter
                )
            {
                own.push((entry.binding, entry.name));
            }
        }
        own.reverse();
        // The blocks this one holds were left before it, so their variables
        // are the last pending.
        let pending = &self.unused.cl_pending;
        let mut first = pending.len();
        while first > 0 && self.scope_holds(scope, self.bindings[pending[first - 1].0].scope) {
            first -= 1;
        }
        if self.scopes[scope].kind != ScopeKind::Function {
            let count = own.len();
            self.unused.cl_pending.extend(own);
            self.unused.cl_pending[first..].rotate_right(count);
            return;
        }
        let pending =
            std::mem::replace(&mut self.unused.cl_pending, ArenaVec::new_in(self.scratch));
        // `cl` reports none of these once the unit has had an error, a
        // warning `/WX` promotes aside.
        if self.hard_errors != 0 || self.prior_error_read() {
            return;
        }
        let order = match self.functions.current {
            | Some(function) => ReportOrder::After(self.context.file_location(
                Context::last_source_vector(function.syntax.body.source_vectors),
            )),
            | None => ReportOrder::AtSite,
        };
        self.with_report_order(order, |analyzer| {
            for &(binding, name) in own.iter().chain(&pending) {
                analyzer.report_unused_as_cl(binding, name);
            }
        });
    }

    /// Whether `inner` is `scope` or a block within it.
    fn scope_holds(&self, scope: usize, mut inner: usize) -> bool {
        loop {
            if inner == scope {
                return true;
            }
            match self.scopes[inner].parent {
                | Some(parent) => inner = parent,
                | None => return false,
            }
        }
    }

    /// Reports a parameter or local variable nothing names, if `cl` does:
    /// not one with static storage or linkage, and as C4189 one only its
    /// initializer names, under the settings at its declaration.
    fn report_unused_as_cl(&mut self, binding: usize, name: Identifier) {
        let data = self.bindings[binding];
        let usage = *self.unused.variable(binding);
        if usage.referenced
            || usage.exempt
            || data.linkage != Linkage::None
            || data.duration != super::Duration::Automatic
            || self.types.unanalyzed(data.ty)
            || matches!(
                self.context.string_cache.at(name.name),
                "__func__" | "__FUNCTION__" | "__PRETTY_FUNCTION__"
            )
        {
            return;
        }
        let rule = match data.kind {
            | BindingKind::Parameter => UnusedRule::ClParameter,
            | _ if usage.initialized => UnusedRule::ClInitialized,
            | _ => UnusedRule::ClVariable,
        };
        self.unused_rule(rule, name.source_vectors, Some(name.name));
    }

    /// Records that the local object `binding` is declared with an
    /// initializer.
    pub(super) fn note_initialized(&mut self, binding: usize) {
        if self.unused.cl {
            self.unused.variable(binding).initialized = true;
        }
    }

    /// GCC's `pop_scope`, which walks the scope's bindings from the latest
    /// back: it reports each variable, and each label the scope holds, in the
    /// reverse of the order they were bound in. A label is bound where it is
    /// first named, and an ordinary one in the function's outermost block.
    /// Parameters and typedefs wait for the end of the function, and a
    /// typedef outside one is never reported.
    /// GCC: `pop_scope` in `c-decl.cc`; `bind`, which links a binding before
    /// the scope's others.
    /// C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42.
    fn pop_scope_as_gcc(&mut self) {
        let scope = self.scope;
        let function = self.functions.current.map(|function| function.id);
        // The labels only a function's scope holds, latest first.
        let mut labels = match self.unused.gcc_functions.last() {
            | Some(record) if record.scope == scope => self.label_count(),
            | _ => 0,
        };
        let first_label = self
            .unused
            .gcc_functions
            .last()
            .map_or(0, |record| record.labels);
        let mut next = self.scope_entries[scope].head.get();
        loop {
            // A label first named when `bound` entries had been installed
            // was bound before the entry installed at that index.
            let label = (labels > first_label).then(|| labels - 1);
            let label_first = match (next, label) {
                | (Some(link), Some(label)) => self.label_binding(label) > link.value,
                | (None, Some(_)) => true,
                | (_, None) => false,
            };
            if label_first {
                labels -= 1;
                if let Some(function) = function {
                    self.report_unused_function_label(labels, function);
                }
                continue;
            }
            let Some(link) = next else {
                break;
            };
            next = link.next;
            let entry = self.entries[link.value];
            match entry.namespace {
                | Namespace::Ordinary => match self.bindings[entry.binding].kind {
                    | BindingKind::Object => self.report_unused_variable(entry.binding, entry.name),
                    | BindingKind::Typedef if !self.unused.gcc_functions.is_empty() =>
                        self.unused.local_typedefs.push((entry.binding, entry.name)),
                    | _ => {},
                },
                | Namespace::Label => self.report_label_if_unused(entry.binding),
                | _ => {},
            }
        }
    }

    /// Where GCC's reports on leaving `scope` fall among the others: at its
    /// closing brace.
    fn scope_order(&mut self, scope: usize) -> ReportOrder {
        let end = self
            .unused
            .scope_ends
            .as_ref()
            .and_then(|ends| ends.get(&scope))
            .copied();
        end.map_or(ReportOrder::AtSite, |end| {
            ReportOrder::After(self.context.file_location(Context::last_source_vector(end)))
        })
    }

    /// Begins GCC's state for the function whose scope was just entered.
    pub(super) fn begin_function_reports(&mut self) {
        if self.unused.tracking
            && self.context.configuration.compiler_identity() == CompilerIdentity::Gcc
        {
            let record = GccFunction {
                scope:    self.scope,
                labels:   self.label_count(),
                typedefs: self.unused.local_typedefs.len(),
            };
            self.unused.gcc_functions.push(record);
        }
    }

    /// Where GCC's reports at the end of the current function fall among the
    /// others: at its closing brace.
    pub(super) fn function_end_order(&mut self) -> ReportOrder {
        match self.functions.current {
            | Some(function)
                if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc =>
                ReportOrder::After(self.context.file_location(Context::last_source_vector(
                    function.syntax.body.source_vectors,
                ))),
            | _ => ReportOrder::AtSite,
        }
    }

    /// Holds a warning about a function's end that GCC's middle end gives,
    /// which lowers the unit's functions once it is parsed, and not under
    /// `-fsyntax-only`.
    /// GCC: `pass_warn_function_return` in `tree-cfg.cc`; `compile_file` in
    /// `toplev.cc`.
    pub(super) fn late_return_flow_warning(
        &mut self,
        group: G,
        site: SourceVectors,
        text: &'static str,
    ) {
        if !self.context.configuration.syntax_only() {
            self.unused.late_return_flow.push((group, site, text));
        }
    }

    /// Holds `cl`'s warning about the end of the function `binding`, which
    /// its code generator gives once the front end is done, and not under
    /// `/Zs`.
    pub(super) fn late_cl_return_flow(&mut self, binding: usize, end: SourceVectors, always: bool) {
        // `/Zs` generates no code.
        if !self.context.configuration.syntax_only() {
            self.unused.cl_return_flow.push((binding, end, always));
        }
    }

    /// Reports `cl`'s C4715 or C4716, which its code generator says of each
    /// function it generates in turn, if the front end reported no error
    /// (`/WX`'s included) and the settings at the function's name show it,
    /// every time (`once` aside): as an error under `/we`, and under `/WX` as
    /// a warning that `error C2220` leads, before each one. Their numbers
    /// have no group of bcc's. Microsoft Learn: "Compiler Error C2220".
    fn report_cl_return_flow(&mut self) {
        use crate::configuration::ClWarningState as State;
        if self.context.error_count() != 0 || self.unused.cl_return_flow.is_empty() {
            return;
        }
        let scratch = self.scratch;
        let generated = self.cl_generated_functions(scratch);
        let outer = std::mem::replace(&mut self.report_order, ReportOrder::UnitEnd);
        for i in 0..self.unused.cl_return_flow.len() {
            let (binding, end, always) = self.unused.cl_return_flow[i];
            if !generated.contains_key(&self.function_identity(binding)) {
                continue;
            }
            let name = self.bindings[binding].name;
            // Not those at its closing brace, where the warning goes.
            let configuration = self.context.configuration_at(name.source_vectors);
            let code = if always { 4716 } else { 4715 };
            let Some(state) = configuration.cl_warnings().shown(code) else {
                continue;
            };
            let promoted = state != State::Error && configuration.warnings_are_errors();
            let severity = if state == State::Error || promoted {
                crate::translation_phases::ErrorSeverity::Error
            } else {
                crate::translation_phases::ErrorSeverity::Warning
            };
            self.report_with_severity(
                SemanticErrorKind::ClReturnFlow { always, promoted },
                end,
                Some(name.name),
                severity,
            );
        }
        self.report_order = outer;
    }

    /// Reports the warnings [`Self::late_return_flow_warning`] held, in the
    /// order of their functions.
    pub(super) fn report_late_return_flow(&mut self) {
        if self.unused.cl {
            self.report_cl_return_flow();
            return;
        }
        for i in 0..self.unused.late_return_flow.len() {
            let (group, site, text) = self.unused.late_return_flow[i];
            self.return_flow_warning(group, site, text);
        }
    }

    /// Notes the definition of an internal function at `binding`.
    pub(super) fn note_function_symbol(&mut self, binding: usize) {
        let data = self.bindings[binding];
        if self.unused.tracking && data.linkage == Linkage::Internal {
            _ = self.unused.function_symbols.insert(data.name.name, binding);
        }
    }

    /// What GCC's `finish_function` reports once it has left the function's
    /// scope and said whether it has a `return`: the parameters only set, in
    /// their order, the typedefs its blocks declared and nothing named, in
    /// theirs, unless an error has been reported, and then the parameters
    /// nothing named.
    /// GCC: `finish_function` in `c-decl.cc`;
    /// `maybe_warn_unused_local_typedefs` and `do_warn_unused_parameter` in
    /// `c-warn.cc`.
    /// C99: §6.9.1p9, p. 142; PDF p. 154; §6.7.7, p. 123; PDF p. 135.
    pub(super) fn report_unused_at_function_end(&mut self) {
        let Some(record) = self.unused.gcc_functions.pop() else {
            return;
        };
        let mut parameters = ArenaVec::new_in(self.scratch);
        let mut next = self.scope_entries[record.scope].head.get();
        while let Some(link) = next {
            next = link.next;
            let entry = self.entries[link.value];
            if entry.namespace == Namespace::Ordinary
                && self.bindings[entry.binding].kind == BindingKind::Parameter
            {
                parameters.push(entry);
            }
        }
        parameters.reverse();
        for entry in &parameters {
            self.report_unused_parameter(entry.binding, entry.name, true);
        }
        // `maybe_warn_unused_local_typedefs` warns only while `errorcount`
        // equals the count it last warned at, which starts at zero.
        let typedefs = record.typedefs;
        if typedefs < self.unused.local_typedefs.len()
            && self.hard_errors == 0
            && self.unused.gcc_errors == 0
            && !self.prior_error_read()
        {
            self.unused.local_typedefs[typedefs..].sort_unstable_by_key(|&(binding, _)| binding);
            for i in typedefs..self.unused.local_typedefs.len() {
                let (binding, name) = self.unused.local_typedefs[i];
                self.report_unused_typedef(binding, name);
            }
        }
        self.unused.local_typedefs.truncate(typedefs);
        for entry in &parameters {
            self.report_unused_parameter(entry.binding, entry.name, false);
        }
    }

    /// Whether GCC has counted an error from before semantic analysis by the
    /// end of the current function: one read at or before its closing brace,
    /// which GCC has read and no further, or any when the input ends in it.
    /// GCC: `c_parser_declaration_or_fndef` in `c-parser.cc`, which calls
    /// `finish_function` once `c_parser_compound_statement` has consumed the
    /// closing brace, before it reads the next token.
    /// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    fn prior_error_read(&mut self) -> bool {
        let Some(first) = self.unused.prior_error else {
            return false;
        };
        let Some(function) = self.functions.current else {
            return true;
        };
        if !function.syntax.closed {
            return true;
        }
        let close = Context::last_source_vector(function.syntax.body.source_vectors);
        if close.length() == 0 {
            return true;
        }
        let vector = self.context.file_vector(close);
        let end = self.context.reading_position(
            close,
            vector.source_file_index,
            vector.index,
            self.unused.function_end,
        );
        if end.is_some() {
            self.unused.function_end = end;
        }
        end.is_none_or(|end| first <= end)
    }

    /// Reports the parameter `binding` if it is only set (`set_only`) or if
    /// nothing names it.
    fn report_unused_parameter(&mut self, binding: usize, name: Identifier, set_only: bool) {
        if self.unused.variable(binding).referenced == set_only {
            self.report_unused_variable(binding, name);
        }
    }

    fn report_unused_variable(&mut self, binding: usize, name: Identifier) {
        let data = self.bindings[binding];
        let parameter = match data.kind {
            | BindingKind::Parameter => true,
            | BindingKind::Object => false,
            | BindingKind::Typedef => {
                self.report_unused_typedef(binding, name);
                return;
            },
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

    /// Reports a typedef a block declares and nothing names. Unlike a
    /// variable's, GCC's warning asks the settings at the typedef: it reports
    /// at the end of the function through `warning_at`, which looks the
    /// pragmas up at the location it is given, so a `push` and `pop` around
    /// the declaration silence it.
    /// GCC: `maybe_warn_unused_local_typedefs`; Clang:
    /// `Sema::DiagnoseUnusedDecl`.
    /// C99: §6.7.7, p. 123; PDF p. 135.
    fn report_unused_typedef(&mut self, binding: usize, name: Identifier) {
        let usage = *self.unused.variable(binding);
        if usage.referenced || usage.marked || usage.exempt || self.is_marked(binding) {
            return;
        }
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        let rule = if gcc {
            UnusedRule::GccLocalTypedef
        } else {
            UnusedRule::ClangLocalTypedef
        };
        self.unused_rule_read_at(
            rule,
            name.source_vectors,
            Some(name.name),
            name.source_vectors,
        );
    }

    /// Notes a type specifier that names the typedef `binding`.
    /// C99: §6.7.7p3, p. 123; PDF p. 135.
    pub(super) fn note_typedef_use(&mut self, binding: usize) {
        if self.unused.tracking {
            self.unused.variable(binding).referenced = true;
        }
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
            .map_or(otherwise, |&end| Context::last_source_vector(end))
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
            && !self.unused.cl
            && if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
                !self.context.configuration.syntax_only()
            } else {
                self.context.error_count() == 0
            }
    }

    /// Reports file-scope objects with internal linkage: GCC at the last
    /// declaration, Clang at the first. GCC reports them among the defined
    /// internal functions in `functions` (the latest declaration, the name and
    /// the definition), in the reverse of the order its symbol table gained
    /// them: an object at its first declaration and a function at its
    /// definition.
    /// GCC: `check_global_declaration`, which `analyze_functions` calls for
    /// each symbol, in `cgraphunit.cc`; `symtab_node::register_symbol` in
    /// `symtab.cc`.
    /// C99: §6.2.2p3, p. 30; PDF p. 42.
    pub(super) fn report_unused_file_scope(
        &mut self,
        functions: &[(usize, Identifier, SourceVectors)],
    ) {
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        let mut sites: ArenaMap<'_, StringCacheId, (usize, usize)> =
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
            // Clang marks the first declaration in the main file, so a
            // header's object that the main file declares again is still
            // reported there (`MarkUnusedFileScopedDecl` and
            // `ShouldWarnIfUnusedFileScopedDecl` in `SemaDecl.cpp`); with no
            // declaration in the main file the site stays `usize::MAX`.
            let in_main = !gcc && self.in_main_file(data.name.source_vectors);
            let (first, site) = sites
                .entry(entry.name.name)
                .or_insert((entry.binding, if gcc { entry.binding } else { usize::MAX }));
            *first = (*first).min(entry.binding);
            if gcc {
                *site = (*site).max(entry.binding);
            } else if in_main {
                *site = (*site).min(entry.binding);
            }
        }
        // Each object's or function's place in GCC's symbol table, its own
        // place, and for a function its name and definition.
        let mut symbols = ArenaVec::new_in(self.scratch);
        symbols.extend(sites.values().map(|&(first, site)| (first, site, None)));
        if gcc {
            symbols.extend(functions.iter().map(|&(latest, name, definition)| {
                let defined = self
                    .unused
                    .function_symbols
                    .get(&name.name)
                    .copied()
                    .unwrap_or(latest);
                (defined, latest, Some((name, definition)))
            }));
            symbols.sort_unstable_by_key(|&(symbol, ..)| std::cmp::Reverse(symbol));
        } else {
            symbols.sort_unstable_by_key(|&(_, site, _)| site);
        }
        for (first, binding, function) in symbols {
            if let Some((name, definition)) = function {
                self.report_unused_function(name, definition);
                continue;
            }
            if binding == usize::MAX {
                continue;
            }
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
                    // Clang leaves the internal objects of headers alone
                    // (`ShouldWarnIfUnusedFileScopedDecl` in `SemaDecl.cpp`).
                    if self.in_main_file(site) {
                        self.unused_rule(UnusedRule::Variable, site, Some(data.name.name));
                    }
                } else if !data.ty.is_volatile() {
                    self.unused_rule(UnusedRule::GccFileObject, site, Some(data.name.name));
                }
            } else if !gcc
                && usage.references == 0
                // Clang classifies these at the first declaration, wherever
                // the main file declares the object again.
                && self.in_main_file(self.bindings[first].name.source_vectors)
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
        if !self.unused.tracking || self.unused.cl {
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

    /// GCC's `emit_side_effect_warnings`, after `c_fully_fold`: at the place
    /// of the expression's tree, or, when it has none, at the statement's
    /// first token.
    fn gcc_discarded_value(&mut self, e: &'tu Expression<'tu>) {
        if self.is_void_or_unknown(e) {
            return;
        }
        let start = GccLocation::point(Context::first_token_vector(e.source_vectors));
        let (folded, location) = self.gcc_folded(e);
        let location = location.unwrap_or(start);
        if !self.computes_more(folded, true) {
            self.gcc_unused_rule(UnusedRule::GccNoEffect, location);
            return;
        }
        self.gcc_unused_effectful_value(folded, location);
    }

    /// What GCC says of a discarded expression with side effects at the
    /// place `locus`, as a statement or a comma's left operand: of a comma,
    /// about its last right operand, and otherwise `warn_if_unused_value`. A
    /// cast to the operand's own type is the operand itself, at the cast's
    /// place.
    fn gcc_unused_effectful_value(&mut self, e: &'tu Expression<'tu>, locus: GccLocation) {
        let inner = self.gcc_relocated_operand(e);
        if let E::Binary {
            operator: B::Comma, ..
        } = inner.kind
        {
            // Each comma of the right spine has a place, which a cast or
            // parentheses around it give.
            let mut at = locus;
            let mut right = inner;
            while let E::Binary {
                operator: B::Comma,
                right_expression,
                ..
            } = right.kind
            {
                right = self.gcc_relocated_operand(right_expression);
                if matches!(
                    right.kind,
                    E::Binary {
                        operator: B::Comma,
                        ..
                    }
                ) {
                    at = self.gcc_location(right_expression).unwrap_or(at);
                }
            }
            let right = Self::without_parentheses(right);
            if !self.computes_more(right, true)
                && !self.is_void_or_unknown(right)
                && !matches!(right.kind, E::Cast { .. })
            {
                self.gcc_unused_rule(UnusedRule::GccCommaRight, at);
            }
        } else if self.computes_more(e, false)
            && let Some(at) = self.gcc_unused_value_in(inner, locus)
        {
            self.gcc_unused_rule(UnusedRule::GccValueComputed, at);
        }
    }

    /// GCC's `c_fully_fold` of a discarded expression: a comma whose left
    /// operand has no side effect and whose right operand is no constant
    /// folds to the right operand, which takes the comma's place
    /// (`fold_binary_loc`, then `protected_set_expr_location`) unless it is
    /// a name, which has none; the outermost such comma's place stays.
    /// Returns the folded expression and its place.
    fn gcc_folded(&self, e: &'tu Expression<'tu>) -> (&'tu Expression<'tu>, Option<GccLocation>) {
        let location = self.gcc_location(e);
        let mut folded = e;
        while let E::Binary {
            operator: B::Comma,
            left_expression,
            right_expression,
        } = self.gcc_relocated_operand(folded).kind
            && !self.computes_more(left_expression, true)
            && !self.gcc_tree_constant(right_expression)
        {
            folded = right_expression;
        }
        if std::ptr::eq(folded, e) {
            (e, location)
        } else {
            (folded, self.gcc_location(folded).and(location))
        }
    }

    /// Whether GCC's tree for `e` is `TREE_CONSTANT`: an arithmetic or
    /// address constant, or a string literal's address.
    fn gcc_tree_constant(&self, e: &'tu Expression<'tu>) -> bool {
        let info = self.expression_info(e);
        info.integer.is_some()
            || matches!(
                info.constant,
                super::ConstantClass::Arithmetic | super::ConstantClass::Address
            )
            || matches!(Self::without_parentheses(e).kind, E::StringLiteral(_))
    }

    /// `e` without its parentheses and the casts to the operand's own type
    /// around it, which GCC's `build_c_cast` leaves out: the tree GCC builds
    /// for `e`, at `e`'s place.
    /// C99: §6.5.4p4, p. 81; PDF p. 93.
    pub(super) fn gcc_relocated_operand(
        &self,
        mut e: &'tu Expression<'tu>,
    ) -> &'tu Expression<'tu> {
        loop {
            e = Self::without_parentheses(e);
            match e.kind {
                | E::Cast {
                    operand_expression, ..
                } if self.gcc_same_type_cast(e, operand_expression) => e = operand_expression,
                | _ => return e,
            }
        }
    }

    /// Whether the cast `e` of `operand` is to the operand's own type, which
    /// GCC's `build_c_cast` drops.
    fn gcc_same_type_cast(&self, e: &'tu Expression<'tu>, operand: &'tu Expression<'tu>) -> bool {
        let (to, from) = (self.expression_info(e).ty, self.expression_info(operand).ty);
        self.types.non_atomic(to).unqualified() == self.types.non_atomic(from).unqualified()
    }

    /// Where GCC places the tree for `e`: its caret at the operator, and the
    /// range around it the whole expression, parentheses included
    /// (`set_c_expr_source_range` in `c-parser.cc`). A cast's caret is its
    /// `(`, a conditional's its `:` and a subscript's its `[`; a comma's is
    /// the comma, with no range unless parentheses give it one. A name, a
    /// constant and a constant GCC folds have no place of their own.
    fn gcc_location(&self, e: &'tu Expression<'tu>) -> Option<GccLocation> {
        let inner = Self::without_parentheses(e);
        if self.expression_info(inner).integer.is_some() {
            return None;
        }
        let operator = Self::operator_site(inner);
        let caret = match inner.kind {
            // A name cast to its own type is a `NON_LVALUE_EXPR` folding
            // drops.
            | E::Cast {
                operand_expression, ..
            } if self.gcc_same_type_cast(inner, operand_expression)
                && self.gcc_location(operand_expression).is_none() =>
                return None,
            | E::Binary {
                operator: B::Subscript,
                ..
            }
            | E::Cast { .. } => Context::first_token_vector(operator),
            | E::Conditional(_) | E::OmittedConditional(_) => Context::last_source_vector(operator),
            | E::Binary { .. }
            | E::Unary { .. }
            | E::DirectMember { .. }
            | E::IndirectMember { .. } => operator,
            | _ => return None,
        };
        let extent = if std::ptr::eq(e, inner)
            && matches!(
                inner.kind,
                E::Binary {
                    operator: B::Comma,
                    ..
                }
            ) {
            SourceVectors::default()
        } else {
            e.source_vectors
        };
        Some(GccLocation { caret, extent })
    }

    /// GCC's `warn_if_unused_value`, for an expression with side effects:
    /// where it says the value is unused, if it does. Each tree it looks
    /// into that has a place of its own moves the place there. It looks
    /// through `NON_LVALUE_EXPR` and `NOP_EXPR`, which a cast is unless it
    /// changes the representation, and a volatile read, an atomic read and a
    /// conditional have an effect of their own. The right operand of `&&` or
    /// `||` is compared with zero at the operator, unless it is a truth
    /// value already.
    /// C99: §6.5.13p3, p. 89; PDF p. 101; §6.5.14p3, p. 89; PDF p. 101.
    fn gcc_unused_value_at(
        &self,
        e: &'tu Expression<'tu>,
        locus: GccLocation,
    ) -> Option<GccLocation> {
        let locus = self.gcc_location(e).unwrap_or(locus);
        self.gcc_unused_value_in(Self::without_parentheses(e), locus)
    }

    /// [`Self::gcc_unused_value_at`] of `e`, whose place is `locus` already.
    fn gcc_unused_value_in(
        &self,
        e: &'tu Expression<'tu>,
        locus: GccLocation,
    ) -> Option<GccLocation> {
        if self.is_effectful_read(e) {
            return None;
        }
        match e.kind {
            | E::Parenthesized { expression } =>
                self.gcc_unused_value_in(Self::without_parentheses(expression), locus),
            | E::Unary {
                operator: U::Plus,
                operand_expression,
            } => self.gcc_unused_value_at(operand_expression, locus),
            | E::Cast {
                operand_expression, ..
            } => {
                // A cast to the operand's own type is the operand, at the
                // cast's place.
                if self.gcc_same_type_cast(e, operand_expression) {
                    return self
                        .gcc_unused_value_in(Self::without_parentheses(operand_expression), locus);
                }
                // Folding drops a widening cast under another cast.
                let mut written = operand_expression;
                let mut operand = Self::without_parentheses(operand_expression);
                while let E::Cast {
                    operand_expression: inner,
                    ..
                } = operand.kind
                {
                    let stripped = Self::without_parentheses(inner);
                    if !self.gcc_cast_keeps_representation(
                        self.expression_info(operand).ty,
                        self.expression_info(stripped).ty,
                    ) {
                        break;
                    }
                    written = inner;
                    operand = stripped;
                }
                let (to, from) = (self.expression_info(e).ty, self.expression_info(operand).ty);
                if self.is_void_or_unknown(e) {
                    None
                } else if self.gcc_cast_keeps_representation(to, from) {
                    self.gcc_unused_value_at(written, locus)
                } else {
                    (self.types.nodes[to.index] == TypeKind::Scalar(Scalar::Bool)
                        || !matches!(
                            operand.kind,
                            E::Conditional(_)
                                | E::Binary {
                                    operator: B::Assignment,
                                    ..
                                }
                        ))
                    .then_some(locus)
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
                    } => self.gcc_unused_value_at(right_expression, locus),
                    // `(b = 1) != 0` folds to `(b = 1, 1)`, whose constant
                    // GCC lets pass.
                    | E::Binary {
                        operator: B::Assignment,
                        right_expression,
                        ..
                    } if self.expression_info(right_expression).integer.is_some() => None,
                    | E::Binary {
                        operator: B::Comma, ..
                    }
                    | E::Conditional(_) => None,
                    | E::Binary {
                        operator:
                            B::Equal
                            | B::NotEqual
                            | B::LessThan
                            | B::GreaterThan
                            | B::LessThanOrEqual
                            | B::GreaterThanOrEqual,
                        ..
                    }
                    | E::Unary {
                        operator: U::LogicalNot,
                        ..
                    } => self.gcc_location(right_expression).or(Some(locus)),
                    | _ => Some(GccLocation::point(Self::operator_site(e))),
                }
            },
            | E::Binary { operator, .. } =>
                (!Self::is_assignment(operator) && operator != B::Comma).then_some(locus),
            | E::Unary { operator, .. } => matches!(
                operator,
                U::Minus | U::BitwiseNot | U::LogicalNot | U::Indirection
            )
            .then_some(locus),
            | E::DirectMember { .. } | E::IndirectMember { .. } => Some(locus),
            | _ => None,
        }
    }

    /// Reports `rule` at the caret of `location`, with GCC's range around
    /// it.
    fn gcc_unused_rule(&mut self, rule: UnusedRule, location: GccLocation) {
        self.with_gcc_extent(location.extent, |analyzer| {
            analyzer.unused_rule(rule, location.caret, None);
        });
    }

    /// Clang's `DiagnoseUnusedExprResult`, which says nothing about an
    /// expression a macro's body wrote, or about a statement expression whose
    /// value a macro gave ([`Self::clang_macro_statement_expression`]), and,
    /// comparisons aside, which report at once, about one it does not reach
    /// (`DiagIfReachable`).
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
            && !self.clang_macro_statement_expression(e, site)
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

    /// Whether `e` is a statement expression, unparenthesized, whose warning
    /// `site` a macro's expansion placed, from its body or an argument:
    /// Clang's `DiagnoseUnused` (`SemaStmt.cpp`) takes it for a function-like
    /// macro used as a statement (`isa<StmtExpr>(E) && Loc.isMacroID()`),
    /// where `Loc` is the place `isUnusedResultAWarning` takes from the
    /// value statement. GNU extension: GCC manual, "Statement Exprs".
    fn clang_macro_statement_expression(
        &self,
        e: &'tu Expression<'tu>,
        site: SourceVectors,
    ) -> bool {
        matches!(e.kind, E::StatementExpression(_))
            && site.length() != 0
            && self.context.produced_by_macro_expansion(site)
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
            // The value of a statement expression is its last statement's:
            // of an expression statement, that expression's warning, and
            // otherwise one at its `(`.
            // GNU extension: GCC manual, "Statement Exprs".
            | E::StatementExpression(block) => match self.value_statement_of(block) {
                | Some(Statement {
                    kind: StatementType::Expression(ExpressionSlot::Parsed(value)),
                    ..
                }) => self.clang_unused_result(value),
                | _ => Some((
                    UnusedRule::ClangResult,
                    Context::first_token_vector(e.source_vectors),
                    e,
                )),
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
                && let Some((_, site, anchor)) = self.clang_unused_result(left)
                && !self
                    .context
                    .written_by_macro_body(Self::operator_site(Self::without_parentheses(left)))
                && !self.clang_macro_statement_expression(left, site)
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
            // GCC's `build_compound_expr` reports it at the comma unless the
            // operand has a place of its own.
            let locus = self
                .gcc_location(left)
                .unwrap_or_else(|| GccLocation::point(Self::operator_site(comma)));
            self.gcc_unused_effectful_value(left, locus);
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
    /// no narrower, or between a pointer and an integer of its size. A cast
    /// to the operand's own type, `bool` included, is none at all: GCC's
    /// `build_c_cast` returns the operand, so `(_Bool) b++` and
    /// `(_Bool) __builtin_add_overflow(a, b, &r)` say nothing.
    /// C99: §6.3.1.3p1, p. 43; PDF p. 55; §6.3.2.3p5-6, p. 47; PDF p. 59;
    /// §6.5.4p4, p. 81; PDF p. 93.
    fn gcc_cast_keeps_representation(&self, to: TypeId, from: TypeId) -> bool {
        if self.types.non_atomic(to).unqualified() == self.types.non_atomic(from).unqualified() {
            return true;
        }
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

/// Where GCC places a tree: the caret, and the range around it, which is
/// empty when GCC's location names no range beyond the caret's token.
#[derive(Clone, Copy)]
struct GccLocation {
    caret:  SourceVectors,
    extent: SourceVectors,
}

impl GccLocation {
    /// A token's own place, with no range around it.
    fn point(caret: SourceVectors) -> Self {
        Self {
            caret,
            extent: SourceVectors::default(),
        }
    }
}
