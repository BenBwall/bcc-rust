//! Phase-7 model of what Clang's code generation emits without optimization,
//! for the two forms the statement walk cannot decide on its own: a `switch`
//! whose controlling expression folds, and a call that does not return. Both
//! set the walk's `dead` flag (`functions.rs`, `State::dead`), which
//! approximates Clang's insertion state: a reference in code Clang skips
//! creates no LLVM value, so it ranks no function in Clang's creation order
//! and selects no DLL import.
//!
//! - **Folded switches.** `CodeGenFunction::EmitSwitchStmt` folds the promoted
//!   controlling expression, selects the matching case, else the `default`
//!   label, and emits only the statements `CollectStatementsForCase` gathers
//!   from that label up to the `break` that leaves the switch. Its other case
//!   labels then resume nothing. When the collection fails (a label or a
//!   declaration it would skip, a `break` it cannot drop, a case range), Clang
//!   emits the switch as usual. To know the selection before the body,
//!   [`Analyzer::prepare_folded_switch`] analyzes the case labels' expressions
//!   first, when no declaration in the body can change the names they see.
//! - **Calls that do not return.** A call to a function declared `noreturn`
//!   (GNU, `__declspec`, `_Noreturn`, `[[noreturn]]`), to a library function
//!   Clang knows does not return, or to `__builtin_unreachable` leaves code
//!   generation in a new block without predecessors. Code later in the same
//!   expression is still emitted there, and a `&&`, `||` or `?:` whose
//!   condition does not fold joins reachable code again. At the end of an
//!   expression statement, `CodeGenFunction::EmitStmt` erases a block without
//!   predecessors that the statement created, and the statements that follow
//!   are skipped until a label. A statement expression's value statement is the
//!   exception: Clang emits it even after a jump and keeps its block.
//!
//! For `switch (0) { case 1: h(0); break; default: g(0); }`, the condition
//! folds to 0, no case matches, and `default` collects `g(0)`. The body
//! starts skipped, `case 1` keeps it skipped, so the reference to `h` is
//! dead, and `default` resumes the code around the switch. For
//! `die(), g(0); h(0);` with `die` declared `noreturn`, `g` is still
//! referenced, but `h` is not.
//!
//! Read [`Analyzer::prepare_folded_switch`], [`Analyzer::fold_switch`] and
//! [`Analyzer::expression_emitted`] first.
//!
//! C99: §6.8.4.2 paragraphs 4-5, p. 134; PDF p. 146 (switch selection).
//! C99: §6.8.6.3 paragraph 2, p. 138; PDF p. 150 (break).
//! C99: §6.8.3 paragraph 2, p. 132; PDF p. 144 (expression statements).
//! C99: §7.20.4.1 paragraph 2, p. 315; PDF p. 327 (`abort`); §7.20.4.3
//! paragraph 6, p. 316; PDF p. 328 (`exit`); §7.20.4.4 paragraph 2, p. 316;
//! PDF p. 328 (`_Exit`); §7.13.2.1 paragraph 4, p. 244; PDF p. 256
//! (`longjmp`).
//! C11: §6.7.4 paragraph 8, p. 126; PDF p. 144 (`_Noreturn`).
//! GNU extension: GCC manual, "Common Function Attributes", `noreturn`.
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>

use super::{
    Analyzer,
    BindingKind,
    BindingOrigin,
    ConstantClass,
    ExtendedType,
    Integer,
    Linkage,
    SpecifierExtension,
    SyntaxOperand,
    TypeId,
    Work,
    functions::FunctionWork,
    scopes::Namespace,
    types::{
        Scalar,
        TypeKind,
    },
};
use crate::{
    builtins::BuiltinId,
    translation_phases::parsing::{
        declaration_syntax::{
            DeclarationSpecifiers,
            Declarator,
            StructDeclaration,
            StructDeclarator,
            TypeSpecifiers,
        },
        syntax::{
            BinaryOperator,
            BlockItem,
            ConstantExpressionSlot,
            Expression,
            ExpressionSlot,
            ExpressionType,
            ForInitializer,
            Statement,
            StatementType,
            UnaryOperator,
        },
    },
    util::{
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Starts folding switch `index` when code generation folds its
    /// controlling `condition`, of promoted type `ty`: schedules its case
    /// labels' expressions, then [`Self::fold_switch`], before its `body`.
    /// A body whose declarations could change what those expressions name
    /// is emitted unfolded instead, as is one this model does not cover.
    /// C99: §6.8.4.2 paragraphs 4-5, p. 134; PDF p. 146.
    pub(super) fn prepare_folded_switch(
        &mut self,
        index: usize,
        condition: Option<&'tu Expression<'tu>>,
        body: &'tu Statement<'tu>,
        ty: TypeId,
    ) {
        if self.tainted {
            return;
        }
        let Some((bits, signed)) = self.integer_type(ty) else {
            return;
        };
        let Some(value) = condition.and_then(|condition| self.folded_integer(condition)) else {
            return;
        };
        let Some((labels, labeled)) = self.switch_labels(body) else {
            return;
        };
        _ = self.functions.folds.insert(
            index,
            FoldedSwitch {
                value: value.cast(bits, signed),
                body,
                labels,
                labeled,
                phase: Phase::Pending,
            },
        );
        self.work.push(Work::FunctionWork(FunctionWork::FoldSwitch(
            index, bits, signed,
        )));
        for label in labels.iter().rev() {
            if let StatementType::Case(ConstantExpressionSlot::Parsed(expression), _) = label.kind {
                self.work.push(Work::Expression(expression.expression()));
            }
        }
    }

    /// Selects the statements folded switch `index` emits, as Clang's
    /// `FindCaseStatementsForValue` does: it scans the labels from the last,
    /// fails at a case range, stops at the case whose value matches, and
    /// otherwise takes `default`, or elides a body without labels. The
    /// labels' values are converted to the promoted type (`bits`, `signed`).
    /// C99: §6.8.4.2 paragraph 5, p. 134; PDF p. 146.
    pub(super) fn fold_switch(&mut self, index: usize, bits: u32, signed: bool) {
        let Some(fold) = self.functions.folds.get(&index).copied() else {
            return;
        };
        let mut default = None;
        let mut matched = None;
        let mut phase = None;
        for &label in fold.labels.iter().rev() {
            match label.kind {
                | StatementType::Default(_) => default = Some(label),
                | StatementType::Case(ConstantExpressionSlot::Parsed(expression), _) => {
                    let Some(value) = self.case_value(expression.expression(), bits, signed) else {
                        phase = Some(Phase::Unfolded);
                        break;
                    };
                    if value.order_key() == fold.value.order_key() {
                        matched = Some(label);
                        break;
                    }
                },
                | _ => {
                    phase = Some(Phase::Unfolded);
                    break;
                },
            }
        }
        let selected = matched.or(default);
        let phase = phase.unwrap_or_else(|| match selected {
            | None if fold.labeled => Phase::Unfolded,
            | None => Phase::Before {
                selected: None,
                end:      None,
            },
            | Some(label) => self.collect_case(fold.body, label),
        });
        if let Some(fold) = self.functions.folds.get_mut(&index) {
            fold.phase = phase;
        }
    }

    /// The skipped state that case or default `label` of the innermost
    /// switch gives the statement it labels: the code around the switch,
    /// unless the switch is folded. Then only the selected label resumes it,
    /// and labels inside the selected statements change nothing.
    /// C99: §6.8.4.2 paragraph 4, p. 134; PDF p. 146.
    pub(super) fn switch_label_dead(&mut self, label: &'tu Statement<'tu>) -> bool {
        let around = self.functions.switch_dead;
        let dead = self.functions.dead;
        let Some(fold) = self
            .statements
            .switch
            .and_then(|index| self.functions.folds.get_mut(&index))
        else {
            return around;
        };
        match fold.phase {
            | Phase::Before {
                selected: Some(selected),
                end,
            } if std::ptr::eq(selected, label) => {
                fold.phase = Phase::Collecting(end);
                around
            },
            | Phase::Before { .. } | Phase::Ended(_) => true,
            | Phase::Collecting(_) => dead,
            | Phase::Pending | Phase::Unfolded => around,
        }
    }

    /// Whether a case label of the innermost switch makes skipped code that
    /// holds it emitted again, as an ordinary label does. Clang never emits
    /// the statements of a switch it skips, or those a folded switch does
    /// not select; it does emit a skipped statement holding a case label
    /// among the statements it selects, as in an unfolded switch.
    /// C99: §6.8.1 paragraph 4, p. 132; PDF p. 144.
    pub(super) fn case_labels_resume(&self) -> bool {
        !self.functions.switch_dead
            && self
                .statements
                .switch
                .and_then(|index| self.functions.folds.get(&index))
                .is_none_or(|fold| !matches!(fold.phase, Phase::Before { .. } | Phase::Ended(_)))
    }

    /// Ends a folded switch's selected statements at the `break` that
    /// `CollectStatementsForCase` stops at; Clang emits nothing for it.
    /// C99: §6.8.6.3 paragraph 2, p. 138; PDF p. 150.
    pub(super) fn switch_break(&mut self, statement: &'tu Statement<'tu>) {
        let dead = self.functions.dead;
        if let Some(fold) = self
            .statements
            .switch
            .and_then(|index| self.functions.folds.get_mut(&index))
            && let Phase::Collecting(Some(end)) = fold.phase
            && std::ptr::eq(end, statement)
        {
            fold.phase = Phase::Ended(dead);
        }
    }

    /// At the end of switch `index`'s body, passes the skipped state its
    /// selected statements ended in to the code after it
    /// ([`FunctionWork::SwitchEnd`]). An unfolded switch, or an elided body,
    /// leaves the code around the switch.
    pub(super) fn finish_folded_switch(&mut self, index: usize) {
        self.functions.switch_after = match self.functions.folds.remove(&index).map(|f| f.phase) {
            | Some(Phase::Collecting(_)) => Some(self.functions.dead),
            | Some(Phase::Ended(after)) => Some(after),
            | _ => None,
        };
    }

    /// The integer code generation folds `condition` to at `-O0`, as
    /// `CodeGenFunction::ConstantFoldsToSimpleInteger` does: an integer
    /// constant, or a comma expression whose left operand folds (so has no
    /// side effects) and whose right operand folds.
    /// C99: §6.6 paragraphs 6 and 10, pp. 95-96; PDF pp. 107-108.
    pub(super) fn folded_integer(&self, condition: &'tu Expression<'tu>) -> Option<Integer> {
        let mut condition = condition;
        loop {
            match condition.kind {
                | ExpressionType::Parenthesized { expression } => condition = expression,
                | ExpressionType::Binary {
                    operator: BinaryOperator::Comma,
                    left_expression,
                    right_expression,
                } => {
                    _ = self.folded_integer(left_expression)?;
                    condition = right_expression;
                },
                | _ => break,
            }
        }
        let info = self.expression_info(condition);
        (info.constant == ConstantClass::Arithmetic)
            .then_some(info.integer)
            .flatten()
    }

    /// Begins expression statement `statement`. A statement expression's
    /// value statement is emitted even when the code before it is skipped,
    /// as `CodeGenFunction::EmitCompoundStmtWithoutScope` ensures an
    /// insertion point for it when the statement expression has a value.
    /// C99: §6.8.3 paragraph 2, p. 132; PDF p. 144.
    pub(super) fn begin_expression_statement(
        &mut self,
        statement: &'tu Statement<'tu>,
        expression: &'tu Expression<'tu>,
    ) {
        self.functions.serial += 1;
        let serial = self.functions.serial;
        let value = self
            .functions
            .value_statements
            .remove(&std::ptr::from_ref(statement).addr())
            .map(|()| (self.functions.dead, self.reference_count()));
        if let Some((dead, _)) = value {
            self.functions.dead = false;
            if dead {
                self.functions.orphan = Some(serial);
            }
        }
        self.work
            .push(Work::FunctionWork(FunctionWork::ExpressionStatementEnd {
                expression,
                serial,
                value,
            }));
    }

    /// Ends expression statement number `serial`. Clang's `EmitStmt` erases
    /// a block without predecessors that the statement created and skips
    /// what follows. A statement expression's value statement with a value
    /// keeps its block for the enclosing expression; one without a value is
    /// an ordinary statement, skipped after a jump (`value`: the skipped
    /// state and reference count before it).
    pub(super) fn end_expression_statement(
        &mut self,
        expression: &'tu Expression<'tu>,
        serial: usize,
        value: Option<(bool, usize)>,
    ) {
        let void = matches!(
            self.types.nodes[self.expression_info(expression).ty.index],
            TypeKind::Scalar(Scalar::Void)
        );
        match value {
            | Some(_) if !void => {},
            | Some((true, start)) => {
                self.skip_references(start..self.reference_count());
                self.functions.dead = true;
            },
            | _ =>
                if self.functions.orphan.is_some_and(|since| since >= serial) {
                    self.functions.dead = true;
                    self.functions.orphan = None;
                },
        }
    }

    /// Begins statement expression `block`, emitted in the code around it,
    /// and records its value statement: the last item, after its labels.
    pub(super) fn begin_statement_expression(&mut self, block: &'tu Statement<'tu>) {
        self.work
            .push(Work::FunctionWork(FunctionWork::StatementExpressionEnd(
                self.functions.dead,
            )));
        if self.functions.dead {
            return;
        }
        let StatementType::Compound { items } = block.kind else {
            return;
        };
        let Some(&BlockItem::Statement(mut last)) = items.last() else {
            return;
        };
        loop {
            match last.kind {
                | StatementType::Label(_, child) => last = child,
                | StatementType::Attributed(attributed) => last = attributed.statement,
                | _ => break,
            }
        }
        if matches!(
            last.kind,
            StatementType::Expression(ExpressionSlot::Parsed(_))
        ) {
            _ = self
                .functions
                .value_statements
                .insert(std::ptr::from_ref(last).addr(), ());
        }
    }

    /// Follows the blocks code generation creates for expression `e`, once
    /// it is analyzed: an evaluated call that does not return leaves a block
    /// without predecessors, and a `&&`, `||` or `?:` whose condition does
    /// not fold joins its operands in a block with predecessors.
    /// C99: §6.5.2.2 paragraph 10, p. 72; PDF p. 84 (calls);
    /// §6.5.13-§6.5.15, pp. 89-91; PDF pp. 101-103.
    pub(super) fn expression_emitted(&mut self, e: &'tu Expression<'tu>) {
        match e.kind {
            | ExpressionType::Call {
                function_expression,
                ..
            } =>
                if !self.functions.dead
                    && self.functions.sizeof.is_none()
                    && self.functions.reference_context.is_none()
                    && self.call_does_not_return(function_expression)
                {
                    self.functions.orphan = Some(self.functions.serial);
                },
            | ExpressionType::Conditional(conditional)
            | ExpressionType::OmittedConditional(conditional) =>
                if self
                    .folded_condition(conditional.condition_expression)
                    .is_none()
                {
                    self.functions.orphan = None;
                },
            | ExpressionType::Binary {
                operator: BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr,
                left_expression,
                ..
            } =>
                if self.folded_condition(left_expression).is_none() {
                    self.functions.orphan = None;
                },
            | _ => {},
        }
    }

    /// Whether a call through `callee` does not return, as LLVM's
    /// `CallBase::doesNotReturn` decides for the call Clang emits: the
    /// called function is `__builtin_unreachable`, `__builtin_abort`, a
    /// function declared `noreturn`, or one that Clang's library builtins
    /// list as not returning; or the callee is an object, parameter or
    /// member whose function type is `noreturn`. A cast keeps a function
    /// designator the direct callee, but gives an object's value its own
    /// type. C99: §6.5.2.2 paragraph 1, p. 71; PDF p. 83.
    fn call_does_not_return(&self, callee: &'tu Expression<'tu>) -> bool {
        let mut callee = callee;
        let mut converted = false;
        loop {
            match callee.kind {
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Unary {
                    operator: UnaryOperator::Indirection | UnaryOperator::AddressOf,
                    operand_expression: expression,
                } => callee = expression,
                | ExpressionType::Cast {
                    operand_expression, ..
                } => {
                    converted = true;
                    callee = operand_expression;
                },
                | _ => break,
            }
        }
        match callee.kind {
            | ExpressionType::Identifier(_) => {
                let Some(binding) = self.expression_info(callee).binding else {
                    return false;
                };
                let data = self.bindings[binding];
                match data.origin {
                    | BindingOrigin::Intrinsic(id) =>
                        matches!(id, BuiltinId::Unreachable | BuiltinId::LibraryAbort),
                    | BindingOrigin::OpaqueIntrinsic => false,
                    | BindingOrigin::User | BindingOrigin::ImplicitOrdinary => match data.kind {
                        | BindingKind::Function =>
                            self.functions.noreturn.contains_key(&binding)
                                || self.library_does_not_return(binding),
                        | BindingKind::Object | BindingKind::Parameter =>
                            !converted && self.functions.noreturn.get(&binding) == Some(&true),
                        | BindingKind::Typedef | BindingKind::Enumerator => false,
                    },
                }
            },
            | ExpressionType::DirectMember {
                base_expression,
                member,
            }
            | ExpressionType::IndirectMember {
                base_expression,
                member,
            } if !converted => {
                let mut ty = self.expression_info(base_expression).ty;
                if matches!(callee.kind, ExpressionType::IndirectMember { .. })
                    && let TypeKind::Pointer(inner) = self.types.nodes[ty.index]
                {
                    ty = inner;
                }
                matches!(self.types.nodes[ty.index], TypeKind::Tag(tag)
                    if self.functions.noreturn_fields.contains_key(&(tag, member.name)))
            },
            | _ => false,
        }
    }

    /// Whether `binding` is a library function that Clang's builtin table
    /// marks as not returning, which every declaration of it inherits from
    /// Clang's implicit one, whatever its type: `abort`, `exit`, `_Exit` and
    /// `longjmp`, and in GNU modes `_exit`, `_longjmp` and `siglongjmp`. A
    /// freestanding translation unit has no library builtins, as Clang's
    /// `-ffreestanding` implies `-fno-builtin`; an internal function of the
    /// same name is another function.
    /// C99: §7.20.4.1 paragraph 2, p. 315; PDF p. 327; §7.20.4.3 paragraph 6,
    /// p. 316; PDF p. 328; §7.20.4.4 paragraph 2, p. 316; PDF p. 328;
    /// §7.13.2.1 paragraph 4, p. 244; PDF p. 256.
    fn library_does_not_return(&self, binding: usize) -> bool {
        let data = self.bindings[binding];
        if !self.context.configuration.hosted() || data.linkage != Linkage::External {
            return false;
        }
        match self.context.string_cache.at(data.name.name) {
            | "abort" | "exit" | "_Exit" | "longjmp" => true,
            | "_exit" | "_longjmp" | "siglongjmp" => self.context.configuration.gnu_extensions(),
            | _ => false,
        }
    }

    /// Records whether the declaration of `binding` says that what it calls
    /// does not return ([`super::functions`]'s `State::noreturn`): the
    /// `_Noreturn` specifier of a function, a `noreturn` attribute written
    /// on a declaration whose type holds a function type, a typedef or
    /// `__typeof__` operand carrying one, or an earlier declaration of the
    /// same entity (`previous`), as Clang merges the attribute and the
    /// function type's bit. Clang gives a written attribute to the innermost
    /// function type, so it is not the function's own when the function
    /// returns a function pointer.
    /// C11: §6.7.4 paragraph 8, p. 126; PDF p. 144.
    /// GNU extension: GCC manual, "Common Function Attributes", `noreturn`.
    pub(super) fn record_noreturn(
        &mut self,
        binding: usize,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Declarator<'tu>,
        previous: Option<usize>,
    ) {
        let data = self.bindings[binding];
        let inherited =
            previous.and_then(|previous| self.functions.noreturn.get(&previous).copied());
        // The type walks run only when a declaration names the fact, so
        // deep pointer chains cost nothing elsewhere.
        let typed = || {
            self.noreturn_type_named(spec, spec.extensions, Some(declarator))
                && self.holds_function_type(data.ty)
                && match self.types.nodes[data.ty.index] {
                    | TypeKind::Function { result, .. } => !self.holds_function_type(result),
                    | _ => true,
                }
        };
        let fact = match data.kind {
            | BindingKind::Function | BindingKind::Object | BindingKind::Typedef
                if inherited == Some(true) || typed() =>
                Some(true),
            | BindingKind::Function
                if inherited.is_some() || spec.function_specifiers.is_noreturn =>
                Some(false),
            | _ => None,
        };
        if let Some(fact) = fact {
            _ = self.functions.noreturn.insert(binding, fact);
        }
    }

    /// Records a prototype parameter whose function pointer type does not
    /// return; the body's parameter binding copies the fact.
    pub(super) fn record_parameter_noreturn(
        &mut self,
        binding: usize,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Option<Declarator<'tu>>,
    ) {
        if self.noreturn_type_named(spec, spec.extensions, declarator)
            && self.holds_function_type(self.bindings[binding].ty)
        {
            _ = self.functions.noreturn.insert(binding, true);
        }
    }

    /// Records member `name` of tag `tag`, of type `ty`, when its function
    /// type does not return.
    pub(super) fn record_field_noreturn(
        &mut self,
        tag: usize,
        name: StringCacheId,
        declarator: StructDeclarator<'tu>,
        declaration: StructDeclaration<'tu>,
        ty: TypeId,
    ) {
        let spec = DeclarationSpecifiers {
            type_specifiers: declaration.type_specifiers,
            ..DeclarationSpecifiers::default()
        };
        if (self.noreturn_type_named(spec, declaration.extensions, declarator.declarator)
            || self.noreturn_written(declarator.attributes, None))
            && self.holds_function_type(ty)
        {
            _ = self.functions.noreturn_fields.insert((tag, name), ());
        }
    }

    /// Whether a declaration names a `noreturn` function type: by an
    /// attribute in `extensions` or `declarator`, or by the typedef name or
    /// `__typeof__` operand of its specifiers. The declared type must still
    /// hold a function type for the fact to apply.
    fn noreturn_type_named(
        &self,
        spec: DeclarationSpecifiers<'tu>,
        extensions: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Option<Declarator<'tu>>,
    ) -> bool {
        if self.noreturn_written(extensions, declarator) {
            return true;
        }
        match spec.type_specifiers {
            | TypeSpecifiers::TypedefName(name) => self
                .lookup(Namespace::Ordinary, name.name)
                .is_some_and(|entry| self.functions.noreturn.get(&entry.binding) == Some(&true)),
            | TypeSpecifiers::Extended(&ExtendedType::Typeof {
                operand: SyntaxOperand::Expression(mut operand),
                ..
            }) => {
                while let ExpressionType::Parenthesized { expression } = operand.kind {
                    operand = expression;
                }
                matches!(operand.kind, ExpressionType::Identifier(_))
                    && self
                        .expression_info(operand)
                        .binding
                        .is_some_and(|binding| {
                            self.functions.noreturn.get(&binding) == Some(&true)
                                || (self.bindings[binding].kind == BindingKind::Function
                                    && self.library_does_not_return(binding))
                        })
            },
            | _ => false,
        }
    }

    /// Whether a `noreturn` attribute, in GNU, `gnu::`, standard or
    /// `__declspec` spelling, is written in `extensions` or `declarator`.
    fn noreturn_written(
        &self,
        extensions: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Option<Declarator<'tu>>,
    ) -> bool {
        if declarator
            .is_none_or(|declarator| !super::attributes::written_on(extensions, declarator))
            && extensions.is_none()
        {
            return false;
        }
        self.declaration_attribute_lists(extensions, declarator)
            .iter()
            .any(|list| {
                !list.recovered
                    && super::attributes::known_names(list, self.context)
                        .any(|(_, name)| matches!(name, "noreturn" | "Noreturn"))
            })
    }

    /// Whether `ty` is a function type, or a pointer to or array of one.
    fn holds_function_type(&self, ty: TypeId) -> bool {
        let mut ty = ty;
        loop {
            match self.types.nodes[ty.index] {
                | TypeKind::Function { .. } => return true,
                | TypeKind::Pointer(inner) | TypeKind::Array(inner, _) => ty = inner,
                | _ => return false,
            }
        }
    }

    /// This switch's case and default labels in `body`, in source order, and
    /// whether the body holds an ordinary label anywhere. `None` when a
    /// declaration in the body precedes one of the labels in its scope, so
    /// that the label's expression cannot be analyzed before the body, or
    /// when the body holds a statement this model does not cover (Microsoft
    /// structured exception handling, named jumps).
    fn switch_labels(
        &self,
        body: &'tu Statement<'tu>,
    ) -> Option<(&'s [&'tu Statement<'tu>], bool)> {
        // Each entry: a statement, whether its case labels are this switch's,
        // and whether a declaration in an enclosing block of the body
        // precedes it.
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((body, true, false));
        let mut labels = ArenaVec::new_in(self.scratch);
        let mut labeled = false;
        while let Some((statement, own, declared)) = work.pop() {
            match statement.kind {
                | StatementType::Case(_, child) | StatementType::Default(child) => {
                    if own {
                        if declared {
                            return None;
                        }
                        labels.push(statement);
                    }
                    work.push((child, own, declared));
                },
                | StatementType::CaseRange(range) => {
                    if own {
                        if declared {
                            return None;
                        }
                        labels.push(statement);
                    }
                    work.push((range.statement, own, declared));
                },
                | StatementType::Label(_, child) => {
                    labeled = true;
                    work.push((child, own, declared));
                },
                | StatementType::Attributed(attributed) =>
                    work.push((attributed.statement, own, declared)),
                | StatementType::Compound { items } => {
                    let start = work.len();
                    let mut declared = declared;
                    for item in items {
                        match item {
                            | BlockItem::Statement(child) => {
                                work.push((child, own, declared));
                                declared |= matches!(
                                    child.kind,
                                    StatementType::Declaration(_) | StatementType::LocalLabels(_)
                                );
                            },
                            | BlockItem::Declaration(_) | BlockItem::FunctionDefinition(_) =>
                                declared = true,
                        }
                    }
                    work[start..].reverse();
                },
                | StatementType::If {
                    condition_expression,
                    then_statement,
                    else_statement,
                } => {
                    let declared =
                        declared || matches!(condition_expression, ExpressionSlot::Selection(_));
                    if let Some(child) = else_statement {
                        work.push((child, own, declared));
                    }
                    work.push((then_statement, own, declared));
                },
                | StatementType::Switch {
                    condition_expression,
                    body_statement,
                } => work.push((
                    body_statement,
                    false,
                    declared || matches!(condition_expression, ExpressionSlot::Selection(_)),
                )),
                | StatementType::While {
                    condition_expression,
                    body_statement,
                }
                | StatementType::DoWhile {
                    condition_expression,
                    body_statement,
                } => work.push((
                    body_statement,
                    own,
                    declared || matches!(condition_expression, ExpressionSlot::Selection(_)),
                )),
                | StatementType::For(for_loop) => work.push((
                    for_loop.body_statement,
                    own,
                    declared
                        || matches!(for_loop.initializer, Some(ForInitializer::Declaration(_))),
                )),
                | StatementType::Seh(_)
                | StatementType::SehLeave
                | StatementType::NamedBreak(_)
                | StatementType::NamedContinue(_) => return None,
                | _ => {},
            }
        }
        Some((self.scratch.alloc_slice_copy(&labels), labeled))
    }

    /// Clang's `CollectStatementsForCase` for `body` and the selected
    /// `label`: the folded switch's phase before its body, with the `break`
    /// the collection ends at, if any, or [`Phase::Unfolded`] when it fails.
    /// The statements it collects run from the label to that `break`, or to
    /// the end of the body. It fails on a label or a declaration it would
    /// skip, a `break` inside a collected statement other than a block, a
    /// block left without a `break` after a declaration, and a label it
    /// cannot reach through blocks and labels.
    fn collect_case(&self, body: &'tu Statement<'tu>, label: &'tu Statement<'tu>) -> Phase<'tu> {
        let mut frames = ArenaVec::new_in(self.scratch);
        let mut found = false;
        let mut end = None;
        let mut visit = Some((body, true));
        let mut outcome = None;
        loop {
            // Visits one statement, while `searching` for the label: its
            // outcome, or none when it entered a block.
            if let Some((mut statement, mut searching)) = visit.take() {
                outcome = loop {
                    match statement.kind {
                        | StatementType::Case(_, child) | StatementType::Default(child) => {
                            if searching && std::ptr::eq(statement, label) {
                                found = true;
                                searching = false;
                            }
                            statement = child;
                        },
                        | StatementType::CaseRange(range) => {
                            if searching && std::ptr::eq(statement, label) {
                                found = true;
                                searching = false;
                            }
                            statement = range.statement;
                        },
                        | StatementType::Break if !searching => {
                            end = Some(statement);
                            break Some(Collected::Success);
                        },
                        | StatementType::Compound { items } => {
                            frames.push(CollectFrame {
                                block: statement,
                                items: items.as_slice(),
                                next: 0,
                                searching,
                                started_live: !searching,
                                declared: false,
                            });
                            break None;
                        },
                        | _ if searching =>
                            break Some(if self.statement_contains_label(statement, true) {
                                Collected::Failure
                            } else {
                                Collected::Success
                            }),
                        | _ =>
                            break Some(if contains_break(self.scratch, statement) {
                                Collected::Failure
                            } else {
                                Collected::FallThrough
                            }),
                    }
                };
            }
            let Some(frame) = frames.last_mut() else {
                return match outcome {
                    | Some(Collected::Success | Collected::FallThrough) if found => Phase::Before {
                        selected: Some(label),
                        end,
                    },
                    | _ => Phase::Unfolded,
                };
            };
            // The outcome of the block's item just visited.
            if let Some(result) = outcome.take() {
                let finished = match (result, frame.searching) {
                    | (Collected::Failure, _) => Some(Collected::Failure),
                    // The item held the label and ended at its `break`.
                    | (Collected::Success, true) if found => Some(if frame.declared {
                        Collected::Failure
                    } else {
                        self.rest_unlabeled(frame)
                    }),
                    | (Collected::Success, true) | (Collected::FallThrough, false) => None,
                    | (Collected::FallThrough, true) => {
                        frame.searching = false;
                        frame.declared.then_some(Collected::Failure)
                    },
                    | (Collected::Success, false) => Some(self.rest_unlabeled(frame)),
                };
                if let Some(finished) = finished {
                    _ = frames.pop();
                    outcome = Some(finished);
                    continue;
                }
            }
            if let Some(&item) = frame.items.get(frame.next) {
                frame.next += 1;
                frame.declared |= might_add_declaration(self.scratch, item);
                match item {
                    | BlockItem::Statement(statement) => visit = Some((statement, frame.searching)),
                    | BlockItem::Declaration(_) | BlockItem::FunctionDefinition(_) =>
                        outcome = Some(if frame.searching {
                            Collected::Success
                        } else {
                            Collected::FallThrough
                        }),
                }
            } else {
                // A block left without its `break` ends its declarations'
                // lifetimes: Clang then emits it whole, if it started in the
                // selected statements and holds no `break`.
                let frame = frames.pop().expect("a block is being collected");
                outcome = Some(if frame.searching {
                    Collected::Success
                } else if frame.declared
                    && (!frame.started_live || contains_break(self.scratch, frame.block))
                {
                    Collected::Failure
                } else {
                    Collected::FallThrough
                });
            }
        }
    }

    /// Clang requires the items after a collection's `break` to be
    /// skippable: success unless one holds an ordinary label.
    fn rest_unlabeled(&self, frame: &CollectFrame<'tu>) -> Collected {
        let labeled = frame.items[frame.next..].iter().any(|item| {
            matches!(item, BlockItem::Statement(statement) if self.statement_contains_label(statement, true))
        });
        if labeled {
            Collected::Failure
        } else {
            Collected::Success
        }
    }
}

/// Clang's `containsBreak`: whether `statement` holds a `break` that leaves
/// it, outside the switches and loops it contains.
/// C99: §6.8.6.3 paragraph 2, p. 138; PDF p. 150.
fn contains_break(scratch: &crate::util::bump::Bump, statement: &Statement<'_>) -> bool {
    let mut work = ArenaVec::new_in(scratch);
    work.push(statement);
    while let Some(statement) = work.pop() {
        match statement.kind {
            | StatementType::Break => return true,
            | StatementType::Compound { items } =>
                for item in items {
                    if let BlockItem::Statement(child) = item {
                        work.push(child);
                    }
                },
            | StatementType::If {
                then_statement,
                else_statement,
                ..
            } => {
                work.push(then_statement);
                work.extend(else_statement);
            },
            | StatementType::Label(_, child)
            | StatementType::Case(_, child)
            | StatementType::Default(child) => work.push(child),
            | StatementType::CaseRange(range) => work.push(range.statement),
            | StatementType::Attributed(attributed) => work.push(attributed.statement),
            | _ => {},
        }
    }
    false
}

/// Clang's `mightAddDeclToScope`: whether block item `item` declares
/// something in the enclosing block, rather than in a block of its own.
/// C99: §6.2.1 paragraph 4, pp. 29-30; PDF pp. 41-42.
fn might_add_declaration(scratch: &crate::util::bump::Bump, item: BlockItem<'_>) -> bool {
    let BlockItem::Statement(statement) = item else {
        return true;
    };
    let mut work = ArenaVec::new_in(scratch);
    work.push(statement);
    while let Some(statement) = work.pop() {
        match statement.kind {
            | StatementType::Declaration(_) | StatementType::LocalLabels(_) => return true,
            | StatementType::Label(_, child)
            | StatementType::Case(_, child)
            | StatementType::Default(child) => work.push(child),
            | StatementType::CaseRange(range) => work.push(range.statement),
            | StatementType::Attributed(attributed) => work.push(attributed.statement),
            | _ => {},
        }
    }
    false
}

/// What collecting one statement of a folded switch's body found.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Collected {
    /// The switch cannot be folded.
    Failure,
    /// The statement was collected; collection continues after it.
    FallThrough,
    /// The statement was skipped while searching for the label, or the
    /// collection ended at a `break` in it.
    Success,
}

/// A block of a folded switch's body being collected.
struct CollectFrame<'tu> {
    block:        &'tu Statement<'tu>,
    items:        &'tu [BlockItem<'tu>],
    next:         usize,
    /// Still searching for the selected label.
    searching:    bool,
    /// The block began inside the selected statements.
    started_live: bool,
    /// An item may declare something: one skipped while searching, which
    /// fails the collection once the label is found, then one collected.
    declared:     bool,
}

/// A switch whose controlling expression code generation folds, while its
/// body is analyzed.
#[derive(Clone, Copy)]
pub(super) struct FoldedSwitch<'tu, 's> {
    /// The promoted controlling value.
    value:   Integer,
    body:    &'tu Statement<'tu>,
    /// This switch's case and default labels, in source order.
    labels:  &'s [&'tu Statement<'tu>],
    /// The body holds an ordinary label, which keeps Clang from eliding it.
    labeled: bool,
    phase:   Phase<'tu>,
}

/// Where the analysis of a folded switch's body stands.
#[derive(Clone, Copy)]
enum Phase<'tu> {
    /// The case labels' expressions are being analyzed.
    Pending,
    /// The selected label, if any, and the `break` its statements end at
    /// are ahead. Without a selected label, Clang elides the whole body.
    Before {
        selected: Option<&'tu Statement<'tu>>,
        end:      Option<&'tu Statement<'tu>>,
    },
    /// In the selected statements, which end at this `break`, if any.
    Collecting(Option<&'tu Statement<'tu>>),
    /// The selected statements ended in this skipped state.
    Ended(bool),
    /// Clang emits the switch without folding it.
    Unfolded,
}
