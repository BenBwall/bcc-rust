//! Pratt parsing and typed lowering of expressions: operands, calls, and the
//! assignments and increments nested in them, whose sequencing
//! `check_update` checks before the run and the machine checks again, on the
//! objects each access reaches, during it. C99: bounded subset of §6.5, pp.
//! 67-94; PDF pp. 79-106.

use super::{
    floating::{
        ConstantError,
        Floating,
        Real,
        Rounded,
    },
    integers::{
        Integer,
        Suffix,
    },
    lexer::{
        self,
        Token,
    },
    model::{
        Arithmetic,
        Clash,
        Diagnostic,
        Library,
        MAX_ARGUMENTS,
        MAX_CODE,
        MAX_DEPTH,
        Nested,
        Op,
        Place,
        SequencePoint,
        SourceRange,
        Type,
    },
    parser::{
        CallSite,
        Parser,
        compound_operator,
        is_identifier,
        structure_of,
    },
};

impl Parser<'_> {
    pub(super) fn expression(&mut self, minimum: u8) -> Result<usize, Diagnostic> {
        self.enter_nesting()?;
        let result = self.expression_inner(minimum);
        self.nesting -= 1;
        result
    }

    fn expression_inner(&mut self, minimum: u8) -> Result<usize, Diagnostic> {
        let mut left = self.prefix()?;
        while let Some((operator, precedence)) = binary(self.current().text) {
            if precedence < minimum {
                break;
            }
            self.position += 1;
            let right = self.expression(precedence + 1)?;
            let lhs = &self.expressions[left];
            let rhs = &self.expressions[right];
            let null = |expression: &Expr| matches!(expression.kind, ExprKind::Number(0, ..));
            let ty = match (operator, lhs.ty, rhs.ty) {
                // The integer promotions apply to each operand of a shift on
                // its own; the result has the promoted left operand's type
                // (C99 §6.5.7p3).
                | ("<<" | ">>", Type::Integer(left), Type::Integer(_)) =>
                    Type::Integer(left.promoted()),
                | ("<<" | ">>", ..) =>
                    return Err(Diagnostic::constraint(
                        combined(lhs.range, rhs.range),
                        "each operand of a shift must have an integer type (C99 §6.5.7p2)",
                        &super::clauses::SHIFT_CONSTRAINT,
                    )),
                // C99 §6.5.10p2, §6.5.11p2, §6.5.12p2: integer operands
                // only; the usual arithmetic conversions give the type.
                | ("&" | "^" | "|", Type::Integer(left), Type::Integer(right)) =>
                    Type::Integer(left.promoted().common(right.promoted()).0),
                | ("&" | "^" | "|", ..) =>
                    return Err(Diagnostic::constraint(
                        combined(lhs.range, rhs.range),
                        &format!(
                            "each operand of `{operator}` must have an integer type: the bitwise \
                             operators combine the bits of integers, and a floating value or a \
                             pointer has none to give (C99 §6.5.10p2, §6.5.11p2, §6.5.12p2)"
                        ),
                        &super::clauses::BITWISE_CONSTRAINT,
                    )),
                // C99 §6.5.13p2-3, §6.5.14p2-3: scalar operands, an int
                // result.
                | ("&&" | "||", left, right)
                    if (left.arithmetic().is_some() || left.is_pointer())
                        && (right.arithmetic().is_some() || right.is_pointer()) =>
                    Type::INT,
                | ("&&" | "||", ..) =>
                    return Err(Diagnostic::constraint(
                        combined(lhs.range, rhs.range),
                        &format!(
                            "each operand of `{operator}` must be a scalar, a number or a \
                             pointer, which it compares with 0; a structure is neither (C99 \
                             §6.5.13p2, §6.5.14p2)"
                        ),
                        &super::clauses::LOGICAL_CONSTRAINT,
                    )),
                | ("%", left, right)
                    if left.floating().is_some() && right.arithmetic().is_some()
                        || right.floating().is_some() && left.arithmetic().is_some() =>
                    return Err(Diagnostic::constraint(
                        combined(lhs.range, rhs.range),
                        "each operand of % must have an integer type: a floating division has no \
                         remainder to give (C99 §6.5.5p2)",
                        &super::clauses::MULTIPLICATIVE_CONSTRAINT,
                    )),
                | ("==" | "!=" | "<" | "<=" | ">" | ">=", left, right)
                    if left.arithmetic().is_some() && right.arithmetic().is_some() =>
                    Type::INT,
                | (_, left, right) if left.arithmetic().is_some() && right.arithmetic().is_some() =>
                    left.arithmetic()
                        .zip(right.arithmetic())
                        .map_or(Type::INT, |(left, right)| {
                            left.promoted().common(right.promoted()).ty()
                        }),
                // C99 §6.5.6p3, p9: two pointers to the same type subtract
                // to a ptrdiff_t, `long` on x86-64; §6.5.8p2: they order
                // as an int 0 or 1.
                | ("-" | "<" | "<=" | ">" | ">=", left, right)
                    if left.is_pointer() && right.is_pointer() =>
                {
                    let (constraint, clause) = if operator == "-" {
                        (
                            "subtracting two pointers needs pointers to the same type: the result \
                             counts elements, and elements of different types have different \
                             sizes (C99 §6.5.6p3)",
                            &super::clauses::SUBTRACTION_CONSTRAINT,
                        )
                    } else {
                        (
                            "ordering two pointers needs pointers to the same type (C99 §6.5.8p2)",
                            &super::clauses::RELATIONAL_CONSTRAINT,
                        )
                    };
                    if !same_pointee(left, right) {
                        return Err(Diagnostic::constraint(
                            combined(lhs.range, rhs.range),
                            constraint,
                            clause,
                        ));
                    }
                    if operator == "-" && matches!(left, Type::StructPointer(_)) {
                        return Err(self.error(
                            "arithmetic on structure pointers needs an array of structures, which \
                             is outside this subset",
                        ));
                    }
                    if operator == "-" {
                        Type::Integer(Integer::Long)
                    } else {
                        Type::INT
                    }
                },
                | ("==" | "!=", Type::Pointer(a, _), Type::Pointer(b, _)) if a == b => Type::INT,
                | ("==" | "!=", left, right) if left.is_pointer() && left == right => Type::INT,
                | (_, Type::Struct(_), _) | (_, _, Type::Struct(_)) =>
                    return Err(self.error(
                        "structures are not operands of arithmetic or comparison; compare their \
                         members",
                    )),
                | ("+" | "-", Type::StructPointer(_), _) | ("+", _, Type::StructPointer(_)) =>
                    return Err(self.error(
                        "arithmetic on structure pointers needs an array of structures, which is \
                         outside this subset",
                    )),
                | (
                    "+" | "-",
                    pointer @ (Type::Pointer(..) | Type::Indirect(..)),
                    Type::Integer(_),
                )
                | ("+", Type::Integer(_), pointer @ (Type::Pointer(..) | Type::Indirect(..))) =>
                    pointer,
                | ("==" | "!=", pointer, Type::Integer(_)) if pointer.is_pointer() && null(rhs) =>
                    Type::INT,
                | ("==" | "!=", Type::Integer(_), pointer) if pointer.is_pointer() && null(lhs) =>
                    Type::INT,
                | _ => return Err(self.error("unsupported operand types for this operator")),
            };
            let effect = lhs.effect || rhs.effect;
            left = self.node(
                ExprKind::Binary(operator, left, right),
                ty,
                false,
                combined(lhs.range, rhs.range),
                lhs.height.max(rhs.height) + 1,
            )?;
            self.expressions[left].effect = effect;
        }
        if minimum == 0 && self.at("?") {
            left = self.conditional(left)?;
        }
        if minimum == 0 && (self.at("=") || compound_operator(self.current().text).is_some()) {
            left = self.assignment(left)?;
        }
        Ok(left)
    }

    /// `target = value` or `target op= value`, after the left operand: an
    /// assignment expression. It groups from the right, so `x = y = 0`
    /// stores 0 into `y` and that value into `x`; its left operand must be
    /// a modifiable lvalue, and its value is the value stored, of the left
    /// operand's type (C99 §6.5.16p3). As a whole statement it is the
    /// subset's familiar store; nested in another expression the machine
    /// checks its sequencing (`check_update`).
    ///
    /// C99: §6.5.16 paragraphs 1-3, p. 91; PDF p. 103.
    fn assignment(&mut self, target: usize) -> Result<usize, Diagnostic> {
        let token = self.current();
        let operator = assignment_operator(token.text);
        self.position += 1;
        let value = self.expression(0)?;
        if self.expressions[target].readonly {
            return Err(self.readonly_error(target, false));
        }
        self.check_lvalue(target)?;
        let ty = self.expressions[target].ty;
        let (common, right_type) = if operator == "=" {
            self.require_type(value, ty)?;
            (None, None)
        } else {
            self.compound_types(target, value, operator)?
        };
        let [left, right] = [target, value].map(|at| &self.expressions[at]);
        let (range, height) = (
            combined(left.range, right.range),
            left.height.max(right.height) + 1,
        );
        let node = self.node(
            ExprKind::Assign {
                target,
                value,
                operator,
                common,
                right_type,
                clash: Clash::None,
            },
            ty,
            false,
            range,
            height,
        )?;
        self.expressions[node].effect = true;
        Ok(node)
    }

    /// `condition ? then : otherwise`, after its condition: the first
    /// operand is a scalar, and the other two must agree, both arithmetic
    /// (the result has their common type), the same structure, pointers to
    /// the same type, or a pointer and a null pointer constant. The third
    /// operand is itself a conditional expression, so `a ? b : c ? d : e`
    /// groups as `a ? b : (c ? d : e)`.
    ///
    /// C99: §6.5.15 paragraphs 2-5, p. 90; PDF p. 102.
    fn conditional(&mut self, condition: usize) -> Result<usize, Diagnostic> {
        self.position += 1;
        let then = self.expression(0)?;
        self.expect(":")?;
        let otherwise = self.expression(0)?;
        let [first, second, third] = [condition, then, otherwise].map(|at| &self.expressions[at]);
        let range = combined(first.range, third.range);
        if !(first.ty.arithmetic().is_some() || first.ty.is_pointer()) {
            return Err(Diagnostic::constraint(
                first.range,
                "the first operand of `?:` must be a scalar, a number or a pointer, which it \
                 compares with 0 (C99 §6.5.15p2)",
                &super::clauses::CONDITIONAL_FIRST,
            ));
        }
        let null = |expression: &Expr| matches!(expression.kind, ExprKind::Number(0, ..));
        let ty = match (second.ty, third.ty) {
            | (left, right) if left.arithmetic().is_some() && right.arithmetic().is_some() => left
                .arithmetic()
                .zip(right.arithmetic())
                .map_or(Type::INT, |(left, right)| {
                    left.promoted().common(right.promoted()).ty()
                }),
            | (left, right) if left == right && !left.is_array() => left,
            | (Type::Pointer(left, a), Type::Pointer(right, b)) if left == right =>
                Type::Pointer(left, a || b),
            | (pointer, Type::Integer(_)) if pointer.is_pointer() && null(third) => pointer,
            | (Type::Integer(_), pointer) if pointer.is_pointer() && null(second) => pointer,
            | _ =>
                return Err(Diagnostic::constraint(
                    combined(second.range, third.range),
                    "the second and third operands of `?:` must agree: both numbers, the same \
                     structure type, pointers to the same type, or a pointer and the null pointer \
                     constant 0 (C99 §6.5.15p3)",
                    &super::clauses::CONDITIONAL_CONSTRAINT,
                )),
        };
        let effect = first.effect || second.effect || third.effect;
        let height = first.height.max(second.height).max(third.height) + 1;
        let node = self.node(
            ExprKind::Conditional(condition, then, otherwise),
            ty,
            false,
            range,
            height,
        )?;
        self.expressions[node].effect = effect;
        Ok(node)
    }

    fn prefix(&mut self) -> Result<usize, Diagnostic> {
        let token = self.current();
        let range = token.range;
        let text = String::from(token.text);
        if self.at("(")
            && (is_type_word(self.peek(1)) || self.peek(1) == "size_t" && self.includes_stdio())
        {
            return self.cast();
        }
        if self.at("sizeof") {
            return self.size_of();
        }
        if self.at("++") || self.at("--") {
            self.position += 1;
            let child = self.expression(UNARY)?;
            return self.update(child, token, true);
        }
        if let Some(operator) = unary(&text) {
            self.position += 1;
            let child = self.expression(UNARY)?;
            let expression = &self.expressions[child];
            let readonly = expression.readonly;
            let ty = match (operator, expression.ty) {
                | ("+" | "-" | "~", Type::Integer(integer)) => Type::Integer(integer.promoted()),
                | ("~", _) =>
                    return Err(Diagnostic::constraint(
                        combined(range, expression.range),
                        "the operand of `~` must have an integer type: it flips the bits of an \
                         integer, and a floating value or a pointer has none to flip (C99 \
                         §6.5.3.3p1)",
                        &super::clauses::COMPLEMENT_CONSTRAINT,
                    )),
                | ("+" | "-", floating @ Type::Floating(_)) => floating,
                | ("!", ty) if ty.arithmetic().is_some() || ty.is_pointer() => Type::INT,
                | ("*", Type::StructPointer(id)) => self.complete_type(id)?,
                | ("*", pointer) if let Some(pointee) = pointer.pointee() => pointee,
                | ("&", ty @ (Type::Integer(_) | Type::Floating(_))) => {
                    self.check_lvalue(child)?;
                    Type::Pointer(ty.arithmetic().unwrap_or(Arithmetic::INT), readonly)
                },
                | ("&", Type::Struct(id)) => {
                    self.check_lvalue(child)?;
                    Type::StructPointer(id)
                },
                // The address of a pointer object is a pointer to a pointer
                // (C99 §6.5.3.2p3).
                | ("&", pointer) if let Some(address) = pointer.pointer_to() => {
                    self.check_lvalue(child)?;
                    address
                },
                | _ =>
                    return Err(self.error(
                        "unsupported unary operand type; pointers to numbers and to structures \
                         are modeled",
                    )),
            };
            let effect = expression.effect;
            let range = combined(range, self.expressions[child].range);
            let height = self.expressions[child].height + 1;
            let node = self.node(ExprKind::Unary(operator, child), ty, false, range, height)?;
            // `*p` for a `const char *p` or a `const int *p` designates an
            // object the program may not modify through it (C99 §6.3.2.1p1).
            self.expressions[node].readonly =
                operator == "*" && self.expressions[child].ty.points_to_const();
            self.expressions[node].effect = effect;
            return Ok(node);
        }
        let mut value = if self.take("(") {
            let mut value = self.expression(0)?;
            // In parentheses a comma is the comma operator (C99 §6.5.17);
            // between arguments it only separates them.
            while self.take(",") {
                let right = self.expression(0)?;
                let [left_operand, right_operand] = [value, right].map(|at| &self.expressions[at]);
                let effect = left_operand.effect || right_operand.effect;
                let (ty, height) = (
                    right_operand.ty,
                    left_operand.height.max(right_operand.height) + 1,
                );
                let range = combined(left_operand.range, right_operand.range);
                value = self.node(ExprKind::Comma(value, right), ty, false, range, height)?;
                self.expressions[value].effect = effect;
            }
            let closing = self.current().range;
            self.expect(")")?;
            self.expressions[value].parenthesized = true;
            self.expressions[value].range = combined(range, closing);
            value
        } else if text.starts_with('"') {
            let literal = self.take_literal()?;
            let (spelling, range) = (literal.spelling.clone(), literal.range);
            let macro_use = literal.macro_use;
            let index = self.string_literal(literal)?;
            let declared = self.statics[index].ty;
            let node = self.node(
                ExprKind::Name(spelling, declared, Place::Static(index)),
                declared.decayed(),
                false,
                range,
                1,
            )?;
            self.expressions[node].macro_use = macro_use;
            node
        } else if text.starts_with('\'') {
            let value = self.character_constant(token)?;
            self.position += 1;
            let node = self.node(ExprKind::Character(value, text), Type::INT, false, range, 1)?;
            self.expressions[node].macro_use = token.macro_use;
            node
        } else if text
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            let (kind, ty) = match Self::number_constant(token)? {
                | Number::Integer(value, integer) => (
                    ExprKind::Number(value, integer, text),
                    Type::Integer(integer),
                ),
                | Number::Real(rounded) => (
                    ExprKind::Real(rounded, text),
                    Type::Floating(rounded.value.format()),
                ),
            };
            self.position += 1;
            let node = self.node(kind, ty, false, range, 1)?;
            self.expressions[node].macro_use = token.macro_use;
            node
        } else if is_identifier(&text) {
            self.position += 1;
            let binding = self
                .symbols
                .iter()
                .rev()
                .find(|symbol| symbol.name == text)
                .map(|symbol| (symbol.ty, symbol.place, symbol.constant));
            if self.take("(") {
                if binding.is_some() {
                    return Err(self.error("calling a local object is not supported"));
                }
                if let Some(library) = self.library(&text) {
                    return self.library_call(library, range);
                }
                let mut arguments = Vec::new();
                let mut types = Vec::new();
                let mut height = 1;
                if !self.take(")") {
                    loop {
                        let argument = self.expression(0)?;
                        let expression = &self.expressions[argument];
                        types.push((
                            expression.ty,
                            matches!(expression.kind, ExprKind::Number(0, ..)),
                        ));
                        height = height.max(expression.height + 1);
                        arguments.push(argument);
                        if self.take(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                let defined = self
                    .signatures
                    .iter()
                    .find(|(function, _)| *function == text)
                    .map(|&(_, returns)| returns);
                let declared = defined.or_else(|| {
                    self.prototypes
                        .iter()
                        .find(|prototype| prototype.name == text)
                        .map(|prototype| prototype.returns)
                });
                self.calls.push(CallSite {
                    name: text.clone(),
                    arguments: types,
                    range,
                    declared: declared.is_some(),
                });
                let closing = self.tokens[self.position - 1].range;
                let returns = declared.unwrap_or(Type::INT);
                let effect = self.any_effect(&arguments);
                let node = self.node(
                    ExprKind::Call(text, arguments),
                    returns,
                    true,
                    combined(range, closing),
                    height,
                )?;
                self.expressions[node].effect = effect;
                node
            } else {
                let Some((declared, place, constant)) = binding else {
                    return Err(self.error("use of an undeclared identifier"));
                };
                // Arrays decay to pointers. A char's value keeps type char:
                // only the operators that need it promote it (see `promoted`).
                // A `const` object is no modifiable lvalue, and a `const`
                // array decays to a pointer to `const` elements.
                // C99: §6.3.2.1 paragraph 2, p. 46; PDF p. 58.
                let ty = declared.decayed_qualified(constant);
                let place = place.map_or_else(|| Place::Local(text.clone()), Place::Static);
                let node = self.node(ExprKind::Name(text, declared, place), ty, false, range, 1)?;
                self.expressions[node].readonly = constant && !declared.is_array();
                node
            }
        } else {
            return Err(self.error(
                "expected a subset expression; nested assignments/increments and unsupported \
                 syntax are rejected",
            ));
        };
        loop {
            if self.at(".") || self.at("->") {
                value = self.member(value)?;
                continue;
            }
            if self.at("++") || self.at("--") {
                let operator = self.current();
                self.position += 1;
                value = self.update(value, operator, false)?;
                continue;
            }
            if !self.take("[") {
                break;
            }
            let index = self.expression(0)?;
            let closing = self.current().range;
            self.expect("]")?;
            let base = &self.expressions[value];
            let subscript = &self.expressions[index];
            // One operand is the pointer and the other the integer, in
            // either order: `i[v]` is `*(i + v)` (C99 §6.5.2.1p1-2).
            let pointer = match (base.ty, subscript.ty) {
                | (pointer @ (Type::Pointer(..) | Type::Indirect(..)), Type::Integer(_))
                | (Type::Integer(_), pointer @ (Type::Pointer(..) | Type::Indirect(..))) =>
                    Some(pointer),
                | _ => None,
            };
            let Some((pointer, element)) =
                pointer.and_then(|pointer| pointer.pointee().map(|element| (pointer, element)))
            else {
                return Err(self.error(
                    "a subscript takes a pointer to a number or to a pointer and an integer \
                     index, in either order (C99 §6.5.2.1p1); arrays of structures are outside \
                     this subset",
                ));
            };
            let readonly = pointer.points_to_const();
            let effect = base.effect || subscript.effect;
            value = self.node(
                ExprKind::Index(value, index),
                element,
                false,
                combined(base.range, closing),
                base.height.max(subscript.height) + 1,
            )?;
            self.expressions[value].readonly = readonly;
            self.expressions[value].effect = effect;
        }
        Ok(value)
    }

    /// `++operand`, `--operand`, `operand++` or `operand--`, whose value is
    /// the new one for a prefix operator and the old one for a postfix one
    /// (C99 §6.5.2.4p2, §6.5.3.1p2). As a whole statement it is the
    /// subset's familiar update; nested in another expression the machine
    /// checks its sequencing (`check_update`).
    fn update(
        &mut self,
        operand: usize,
        operator: Token<'_>,
        prefix: bool,
    ) -> Result<usize, Diagnostic> {
        let expression = &self.expressions[operand];
        if !matches!(
            expression.ty,
            Type::Integer(_) | Type::Floating(_) | Type::Pointer(..) | Type::Indirect(..)
        ) {
            return Err(self.error("++ and -- update a number or a pointer to one in this subset"));
        }
        if expression.readonly {
            return Err(self.readonly_error(operand, true));
        }
        self.check_lvalue(operand)?;
        let delta = if operator.text == "++" { 1 } else { -1 };
        let range = if prefix {
            combined(operator.range, expression.range)
        } else {
            combined(expression.range, operator.range)
        };
        let (ty, height) = (expression.ty, expression.height + 1);
        let node = self.node(
            ExprKind::Update {
                target: operand,
                delta,
                prefix,
                clash: Clash::None,
            },
            ty,
            false,
            range,
            height,
        )?;
        self.expressions[node].effect = true;
        Ok(node)
    }

    /// Check the `++`, `--` and assignments nested in a full expression,
    /// and record on each what else in it could reach the object it
    /// updates. Two accesses to a variable named directly, an update and
    /// another use of its name or another update of it, must be separated
    /// by a sequence point: C99 makes them undefined otherwise, and the
    /// check refuses them before the run. A `*`, `[]`, `->` or call reaches
    /// an object only the run decides, so the machine compares the objects
    /// such accesses actually reach as it makes them (`sequencing.rs`). The
    /// sequence points inside an expression follow the first operand of
    /// `&&`, `||` and `?:` and the left operand of a comma operator, and
    /// precede a call, after its arguments; only one of the second and
    /// third operands of `?:` is evaluated. An assignment's operands are
    /// evaluated before its store, so its right operand may read the
    /// variable it stores into, as in `x = x + 1`.
    ///
    /// C99: §6.5 paragraph 2, p. 67; PDF p. 79.
    /// C99: §6.5.16 paragraph 3, p. 91; PDF p. 103.
    /// C99: Annex C paragraph 1, p. 439; PDF p. 451.
    pub(super) fn check_update(&mut self, expression: usize) -> Result<(), Diagnostic> {
        if !self.expressions[expression].effect {
            return Ok(());
        }
        let reach = self.reach(expression)?;
        let clash = |name: Option<&String>, node: usize| {
            if reach.checked.contains(&node) {
                Clash::Checked
            } else if reach.opaque.is_some()
                || name.is_some_and(|name| {
                    reach.names.contains(name)
                        || reach
                            .updates
                            .iter()
                            .filter(|(other, _)| other == name)
                            .count()
                            > 1
                })
            {
                Clash::Separated
            } else {
                Clash::None
            }
        };
        let mut found: Vec<(usize, Clash)> = reach
            .updates
            .iter()
            .map(|(name, node)| (*node, clash(Some(name), *node)))
            .collect();
        found.extend(reach.checked.iter().map(|&node| (node, Clash::Checked)));
        for (node, value) in found {
            if let ExprKind::Update { clash, .. } | ExprKind::Assign { clash, .. } =
                &mut self.expressions[node].kind
            {
                *clash = value;
            }
        }
        Ok(())
    }

    /// What `expression` can reach, after checking that no two of its
    /// operands that C leaves unsequenced clash over an updated variable.
    fn reach(&self, expression: usize) -> Result<Reach, Diagnostic> {
        let node = &self.expressions[expression];
        let mut reach = Reach::default();
        match &node.kind {
            | ExprKind::Name(name, ..) => reach.names.push(name.clone()),
            | ExprKind::Update { target, .. } => match &self.expressions[*target].kind {
                | ExprKind::Name(name, ..) => reach.updates.push((name.clone(), expression)),
                // The object `*p`, `a[i]` or `s.m` designates is the run's
                // to decide.
                | _ => {
                    reach = self.reach(*target)?;
                    reach = self.unsequenced(reach, Reach::checked(node.range, expression))?;
                },
            },
            | ExprKind::Assign { target, value, .. } => {
                let right = self.reach(*value)?;
                match &self.expressions[*target].kind {
                    | ExprKind::Name(name, ..) => {
                        // An update the right operand completes at a
                        // sequence point of its own comes before the store.
                        if right
                            .updates
                            .iter()
                            .any(|(other, at)| other == name && !right.sealed.contains(at))
                        {
                            return Err(self.unsequenced_error(name, node.range));
                        }
                        // The right operand's reads come before the store.
                        let opaque = right.opaque.is_some();
                        reach = right;
                        reach.names.retain(|other| other != name);
                        reach.updates.push((name.clone(), expression));
                        if opaque {
                            reach.checked.push(expression);
                        }
                    },
                    | _ => {
                        let left = self.reach(*target)?;
                        reach = self.unsequenced(left, right)?;
                        reach = self.unsequenced(reach, Reach::checked(node.range, expression))?;
                    },
                }
            },
            // `&v` takes the address of `v` without accessing it (C99
            // §6.5.3.2p3).
            | ExprKind::Unary("&", child)
                if matches!(self.expressions[*child].kind, ExprKind::Name(..)) => {},
            | ExprKind::Unary(operator, child) => {
                reach = self.reach(*child)?;
                if *operator == "*" {
                    reach = self.unsequenced(reach, Reach::opaque(node.range))?;
                }
            },
            | ExprKind::Cast(_, child) => reach = self.reach(*child)?,
            | ExprKind::Member { base, arrow, .. } => {
                reach = self.reach(*base)?;
                if *arrow {
                    reach = self.unsequenced(reach, Reach::opaque(node.range))?;
                }
            },
            | ExprKind::Index(base, index) => {
                let operands = self.unsequenced(self.reach(*base)?, self.reach(*index)?)?;
                reach = self.unsequenced(operands, Reach::opaque(node.range))?;
            },
            // A sequence point follows the first operand (C99 §6.5.13p4,
            // §6.5.14p4, §6.5.17p2).
            | ExprKind::Binary("&&" | "||", left, right) | ExprKind::Comma(left, right) => {
                reach = self.reach(*left)?;
                reach.seal();
                reach.join(self.reach(*right)?);
            },
            | ExprKind::Binary(_, left, right) =>
                reach = self.unsequenced(self.reach(*left)?, self.reach(*right)?)?,
            // A sequence point follows the first operand, and only one of
            // the others is evaluated (C99 §6.5.15p4).
            | ExprKind::Conditional(condition, then, otherwise) => {
                reach = self.reach(*condition)?;
                reach.seal();
                for operand in [then, otherwise] {
                    reach.join(self.reach(*operand)?);
                }
            },
            // The arguments are unsequenced with one another, and a sequence
            // point precedes the call (C99 §6.5.2.2p10).
            | ExprKind::Call(_, arguments) | ExprKind::Library(_, arguments) => {
                for argument in arguments {
                    reach = self.unsequenced(reach, self.reach(*argument)?)?;
                }
                reach.seal();
                reach.join(Reach::opaque(node.range));
            },
            | ExprKind::Number(..)
            | ExprKind::Real(..)
            | ExprKind::Character(..)
            | ExprKind::Sizeof(..) => {},
        }
        Ok(reach)
    }

    /// Join the reach of two operands that no sequence point separates,
    /// refusing an update of a variable in one that the other also reads
    /// or updates by name, and marking an update that the other's `*`,
    /// `[]`, `->` or call might reach for the run to check.
    fn unsequenced(&self, mut left: Reach, right: Reach) -> Result<Reach, Diagnostic> {
        let mut checked = Vec::new();
        for (updates, other) in [(&left, &right), (&right, &left)] {
            for (name, node) in &updates.updates {
                if other.names.contains(name) || other.updates.iter().any(|(it, _)| it == name) {
                    return Err(self.unsequenced_error(name, self.expressions[*node].range));
                }
                if other.opaque.is_some() {
                    checked.push(*node);
                }
            }
            if other.opaque.is_some() {
                checked.extend(updates.checked.iter().copied());
            }
        }
        left.join(right);
        left.checked.extend(checked);
        Ok(left)
    }

    /// Two accesses to the variable `name`, one an update, that no sequence
    /// point separates.
    fn unsequenced_error(&self, name: &str, range: SourceRange) -> Diagnostic {
        let whole = if self.in_condition {
            "condition"
        } else {
            "expression"
        };
        Diagnostic::new(
            "unsupported",
            range,
            &format!(
                "this {whole} updates `{name}` and also reads or updates it with no sequence \
                 point between the two; C99 makes that undefined whenever both are evaluated \
                 (§6.5p2). A sequence point follows the first operand of &&, || and ?: and the \
                 left operand of a comma operator"
            ),
        )
    }

    /// `sizeof unary-expression` or `sizeof (type-name)`: the size in bytes
    /// of the operand's type, a constant of type `size_t` (`unsigned long`
    /// on x86-64). The operand is not evaluated, and an array operand does
    /// not decay, so its size is the whole array's.
    ///
    /// C99: §6.5.3.4 paragraphs 2 and 4, p. 80; PDF p. 92.
    /// C99: §6.3.2.1 paragraph 3, p. 46; PDF p. 58 (no decay).
    fn size_of(&mut self) -> Result<usize, Diagnostic> {
        let keyword = self.current();
        self.position += 1;
        let (ty, range) = if self.at("(")
            && (is_type_word(self.peek(1)) || self.peek(1) == "size_t" && self.includes_stdio())
        {
            self.position += 1;
            let ty = self.type_name()?;
            let closing = self.current();
            self.expect(")")?;
            (ty, combined(keyword.range, closing.range))
        } else {
            let operand = self.expression(UNARY)?;
            let expression = &self.expressions[operand];
            if expression.effect {
                return Err(self.error("sizeof of an increment is outside this subset"));
            }
            let ty = match expression.kind {
                | ExprKind::Name(_, declared, _) | ExprKind::Member { declared, .. } => declared,
                | _ => expression.ty,
            };
            (ty, combined(keyword.range, expression.range))
        };
        let (size, _) = ty.size_align(&self.structures);
        self.node(
            ExprKind::Sizeof(size, ty),
            Type::Integer(Integer::UnsignedLong),
            false,
            range,
            1,
        )
    }

    /// A type name, as in a cast or `sizeof`: an arithmetic type or a
    /// structure, optionally `const`, optionally with pointers to it.
    ///
    /// C99: §6.7.6 paragraph 2, p. 122; PDF p. 134.
    fn type_name(&mut self) -> Result<Type, Diagnostic> {
        let start = self.current();
        if self.at("void") {
            return Err(self.error(
                "void is outside this subset: a cast to void, which discards a value, and void \
                 pointers both",
            ));
        }
        let base = self.base_type()?;
        let depth = self.pointer()?;
        if !self.at(")") {
            return Err(self.error(
                "a type name in this subset is an arithmetic type or a structure, or pointers to \
                 one (such as `unsigned int *`, `char **` or `double`); arrays and functions in a \
                 type name are outside it",
            ));
        }
        let range = combined(start.range, self.tokens[self.position - 1].range);
        self.declared_type(base, depth, None, range)
    }

    /// The constant `token`, a preprocessing number: a floating constant
    /// when it has a period, or an exponent (`e`, or `p` after `0x`), and
    /// otherwise an integer constant.
    ///
    /// C99: §6.4.4.2 paragraph 2, p. 58; PDF p. 70.
    pub(super) fn number_constant(token: Token<'_>) -> Result<Number, Diagnostic> {
        let text = token.text;
        let hexadecimal = text.starts_with("0x") || text.starts_with("0X");
        let floating = text.contains('.')
            || (hexadecimal && text.contains(['p', 'P']))
            || (!hexadecimal && text.contains(['e', 'E']));
        if floating {
            Self::floating_constant(token).map(Number::Real)
        } else {
            Self::integer_constant(token).map(|(value, integer)| Number::Integer(value, integer))
        }
    }

    /// The value of the floating constant `token`, the nearest value of its
    /// type, and whether that is exact. A value beyond the type's range
    /// breaks a constraint.
    ///
    /// C99: §6.4.4.2 paragraphs 3-4, p. 58; PDF p. 70.
    /// C99: §6.4.4 paragraph 2, p. 54; PDF p. 66.
    pub(super) fn floating_constant(token: Token<'_>) -> Result<Rounded, Diagnostic> {
        let text = token.text;
        Real::parse_constant(text).map_err(|error| match error {
            | ConstantError::Malformed => Diagnostic::new(
                "syntax",
                token.range,
                &format!(
                    "`{text}` is not a floating constant: it has digits with a period, an \
                     exponent such as e-3, or both, then an optional f or l suffix; a hexadecimal \
                     one needs a p exponent (C99 §6.4.4.2p1)"
                ),
            ),
            | ConstantError::TooLong => Diagnostic::limit(
                token.range,
                "a floating constant may have at most 256 characters here",
            ),
            | ConstantError::OutOfRange(format) => Diagnostic::constraint(
                token.range,
                &format!(
                    "`{text}` is beyond the largest {}, about {}: a constant's value must be in \
                     its type's range (C99 §6.4.4p2)",
                    format.spelling(),
                    format.maximum_text()
                ),
                &super::clauses::CONSTANT_RANGE,
            ),
        })
    }

    /// The value and type of the integer constant `token`: decimal, octal
    /// (a leading `0`) or hexadecimal (a leading `0x`) digits and an
    /// optional suffix. The base decides the list of types the constant may
    /// take as well as its value.
    ///
    /// C99: §6.4.4.1 paragraphs 1 and 3-5, pp. 54-56; PDF pp. 66-68.
    pub(super) fn integer_constant(token: Token<'_>) -> Result<(i128, Integer), Diagnostic> {
        let text = token.text;
        let error = |kind, message: &str| Err(Diagnostic::new(kind, token.range, message));
        let (radix, prefix) = match text.as_bytes() {
            | [b'0', b'x' | b'X', ..] => (16, 2),
            | [b'0', ..] => (8, 1),
            | _ => (10, 0),
        };
        let body = &text[prefix..];
        let digits = body
            .bytes()
            .take_while(|byte| byte.is_ascii_hexdigit() && (radix == 16 || byte.is_ascii_digit()))
            .count();
        let (number, suffix) = body.split_at(digits);
        if radix == 16 && number.is_empty() {
            return error(
                "syntax",
                &format!(
                    "`{text}` is not an integer constant: 0x must be followed by hexadecimal \
                     digits, 0 to 9 and a to f (C99 §6.4.4.1p1)"
                ),
            );
        }
        if radix == 8
            && let Some(digit) = number.bytes().find(|digit| *digit > b'7')
        {
            return error(
                "syntax",
                &format!(
                    "`{text}` starts with 0, which makes it an octal constant, and {} is not an \
                     octal digit: octal uses only 0 to 7 (C99 §6.4.4.1p3)",
                    char::from(digit)
                ),
            );
        }
        let Some(suffix) = Suffix::parse(suffix) else {
            return error(
                "syntax",
                &format!(
                    "`{text}` is not an integer constant: a suffix is u, l or ll, or u with l or \
                     ll, in either case (C99 §6.4.4.1p1)"
                ),
            );
        };
        // A lone `0` is an octal constant, whose value is the same in
        // every base.
        let number = if number.is_empty() { "0" } else { number };
        let Some(integer) = u64::from_str_radix(number, radix).ok().and_then(|value| {
            Integer::constant(i128::from(value), suffix, radix == 10).map(|ty| (value, ty))
        }) else {
            return error(
                "unsupported",
                &format!(
                    "no type in the list for `{text}` can hold its value, and bcc has no wider \
                     extended integer type to give it (C99 §6.4.4.1p5-6); the largest integer \
                     constant is 18446744073709551615, 0xFFFFFFFFFFFFFFFF, which needs a u suffix \
                     when written in decimal"
                ),
            );
        };
        Ok((i128::from(integer.0), integer.1))
    }

    /// `(type-name) operand`: a cast between arithmetic types, or between
    /// pointers to the same type that differ only in `const`. Its operand
    /// binds as a unary operator's does, so `(char)300 + 1` adds 1 to the
    /// cast.
    ///
    /// C99: §6.5.4 paragraphs 2 and 4, p. 81; PDF p. 93.
    fn cast(&mut self) -> Result<usize, Diagnostic> {
        let opening = self.current();
        self.position += 1;
        let start = self.current();
        if self.at("void") {
            return Err(self.error(
                "a cast to void, which discards a value, is outside this subset; write the \
                 expression as a statement of its own",
            ));
        }
        // A cast yields a value, not an object, so a qualifier on the type
        // it names changes nothing (C99 §6.5.4, footnote 89).
        let to = self.type_name()?;
        let closing = self.current();
        self.position += 1;
        let child = self.expression(UNARY)?;
        let operand = &self.expressions[child];
        let from = operand.ty;
        let allowed = match (from, to) {
            | (from, to) if from.arithmetic().is_some() && to.arithmetic().is_some() => true,
            | (Type::Pointer(a, _), Type::Pointer(b, _)) => a == b,
            | (from, to) if from.is_pointer() => from == to,
            | (Type::Struct(_) | Type::Array(..), _) | (_, Type::Struct(_)) =>
                return Err(Diagnostic::constraint(
                    combined(opening.range, operand.range),
                    "a cast converts a scalar, a number or a pointer, to a scalar type; a \
                     structure has no scalar value to convert (C99 §6.5.4p2)",
                    &super::clauses::CAST_CONSTRAINT,
                )),
            | _ => false,
        };
        if !allowed {
            return Err(Diagnostic::new(
                "unsupported",
                combined(start.range, closing.range),
                "this subset casts between arithmetic types (integer and floating), and between \
                 pointers to the same type that differ only in const; casts between integers and \
                 pointers, floating values and pointers, or different pointer types are outside it",
            ));
        }
        let effect = operand.effect;
        let range = combined(opening.range, operand.range);
        let height = operand.height + 1;
        let node = self.node(ExprKind::Cast(to, child), to, false, range, height)?;
        self.expressions[node].effect = effect;
        Ok(node)
    }

    /// `base.m` or `base->m`: the member's lvalue (C99 §6.5.2.3p3, p4).
    /// A member array decays at once, as a named array does.
    fn member(&mut self, base: usize) -> Result<usize, Diagnostic> {
        let arrow = self.take("->");
        if !arrow {
            self.expect(".")?;
        }
        let name = self.current();
        if !is_identifier(name.text) {
            return Err(self.error("expected a member name"));
        }
        self.position += 1;
        let operand = &self.expressions[base];
        if operand.call {
            return Err(self.error(
                "a member of a call's result is outside this subset; store the result first",
            ));
        }
        let id = match (arrow, operand.ty) {
            | (false, Type::Struct(id)) => id,
            | (true, Type::StructPointer(id)) => {
                let _complete = self.complete_type(id)?;
                id
            },
            | (false, _) => return Err(self.error("`.` needs a structure on its left")),
            | (true, _) =>
                return Err(self.error("`->` needs a pointer to a structure on its left")),
        };
        let Some(member) = self.structures[id]
            .members
            .iter()
            .find(|member| member.name == name.text)
        else {
            return Err(Diagnostic::new(
                "unsupported",
                name.range,
                &format!(
                    "struct {} has no member named `{}`",
                    self.structures[id].name, name.text
                ),
            ));
        };
        let declared = member.ty;
        let offset = i32::try_from(member.first).map_err(|_| self.error("object too large"))?;
        let ty = declared.decayed();
        let (range, height) = (combined(operand.range, name.range), operand.height + 1);
        let effect = operand.effect;
        let node = self.node(
            ExprKind::Member {
                base,
                name: String::from(name.text),
                offset,
                arrow,
                declared,
            },
            ty,
            false,
            range,
            height,
        )?;
        self.expressions[node].effect = effect;
        Ok(node)
    }

    /// `Struct(id)` when structure `id` is complete.
    fn complete_type(&self, id: usize) -> Result<Type, Diagnostic> {
        if self.complete.get(id) == Some(&true) {
            Ok(Type::Struct(id))
        } else {
            Err(self.error("this structure type is incomplete here"))
        }
    }

    fn node(
        &mut self,
        kind: ExprKind,
        ty: Type,
        call: bool,
        range: SourceRange,
        height: usize,
    ) -> Result<usize, Diagnostic> {
        if height > MAX_DEPTH || self.expressions.len() >= MAX_CODE {
            return Err(Diagnostic::limit(
                range,
                "expression depth or node limit reached",
            ));
        }
        let index = self.expressions.len();
        self.expressions.push(Expr {
            kind,
            ty,
            call,
            range,
            height,
            parenthesized: false,
            readonly: false,
            macro_use: None,
            effect: false,
        });
        Ok(index)
    }

    /// The value of the character constant `token`: the char its single
    /// character or escape sequence makes, converted to int (C99
    /// §6.4.4.4p10). Plain char is signed on x86-64, so `'\xff'` is -1.
    pub(super) fn character_constant(&self, token: Token<'_>) -> Result<i128, Diagnostic> {
        match *lexer::literal_bytes(self.source, token)? {
            | [byte] => Ok(i128::from(i8::from_ne_bytes([byte]))),
            | [] => Err(Diagnostic::new(
                "syntax",
                token.range,
                "a character constant holds one character",
            )),
            | _ => Err(Diagnostic::new(
                "unsupported",
                token.range,
                "a character constant with several characters has an implementation-defined value \
                 (C99 §6.4.4.4p10), outside this subset",
            )),
        }
    }

    /// A call to a function `<stdio.h>` declared, after its `(`. Its
    /// arguments are checked against the function's prototype: `printf`
    /// takes a format string and then any scalars through `...`, `puts` a
    /// string and `putchar` an int.
    ///
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84.
    fn library_call(&mut self, library: Library, range: SourceRange) -> Result<usize, Diagnostic> {
        let mut arguments = Vec::new();
        let mut height = 1;
        if !self.take(")") {
            loop {
                let argument = self.expression(0)?;
                let expression = &self.expressions[argument];
                height = height.max(expression.height + 1);
                arguments.push(argument);
                if arguments.len() > MAX_ARGUMENTS {
                    return Err(Diagnostic::limit(
                        expression.range,
                        "a library call takes at most 16 arguments here",
                    ));
                }
                if self.take(")") {
                    break;
                }
                self.expect(",")?;
            }
        }
        let closing = self.tokens[self.position - 1].range;
        let whole = combined(range, closing);
        let name = library.name();
        // Each takes one fixed argument; printf takes more through `...`.
        if arguments.is_empty() || (library != Library::Printf && arguments.len() > 1) {
            return Err(Diagnostic::new(
                "syntax",
                whole,
                &match library {
                    | Library::Printf => String::from(
                        "printf needs at least its format string: int printf(const char *format, \
                         ...)",
                    ),
                    | Library::Puts =>
                        String::from("puts takes one string: int puts(const char *s)"),
                    | Library::Putchar =>
                        String::from("putchar takes one character: int putchar(int c)"),
                },
            ));
        }
        for (index, &argument) in arguments.iter().enumerate() {
            let expression = &self.expressions[argument];
            let ty = expression.ty;
            let acceptable = match (library, index) {
                | (Library::Printf | Library::Puts, 0) =>
                    matches!(ty, Type::Pointer(Arithmetic::CHAR, _)),
                | (Library::Putchar, _) => matches!(ty, Type::Integer(_)),
                | _ => !matches!(ty, Type::Struct(_)),
            };
            if !acceptable {
                let message = match (library, index) {
                    | (Library::Printf, 0) => String::from(
                        "printf's first argument is its format, a string: a char pointer such as \
                         a string literal",
                    ),
                    | (Library::Puts, _) => String::from(
                        "puts's argument is a string: a char pointer such as a string literal",
                    ),
                    | (Library::Putchar, _) =>
                        String::from("putchar's argument is a character, an integer"),
                    | _ => format!(
                        "a structure passed to {name} matches none of its conversions; pass its \
                         members"
                    ),
                };
                return Err(Diagnostic::new("unsupported", expression.range, &message));
            }
        }
        let effect = self.any_effect(&arguments);
        let node = self.node(
            ExprKind::Library(library, arguments),
            Type::INT,
            true,
            whole,
            height,
        )?;
        self.expressions[node].effect = effect;
        Ok(node)
    }

    /// Whether any of `arguments` contains a `++` or `--`.
    fn any_effect(&self, arguments: &[usize]) -> bool {
        arguments
            .iter()
            .any(|&argument| self.expressions[argument].effect)
    }

    /// Emit `operand`, the first operand of `&&`, `||` or `?:` or the left
    /// operand of a comma operator, which a sequence point ends (C99
    /// §6.5.13p4, §6.5.14p4, §6.5.15p4, §6.5.17p2), and return its node.
    fn emit_first(&mut self, operand: usize, operator: &'static str) -> Result<usize, Diagnostic> {
        let after = self.next_point;
        self.next_point = SequencePoint::Operand {
            operator,
            first: self.expressions[operand].range,
        };
        let node = self.emit_expression(operand, false);
        self.next_point = after;
        node
    }

    /// Emit `expression` (as an lvalue when `address` is set) and return its
    /// expression-tree node.
    pub(super) fn emit_expression(
        &mut self,
        expression: usize,
        address: bool,
    ) -> Result<usize, Diagnostic> {
        let node = self.expressions[expression].clone();
        if address {
            self.check_lvalue(expression)?;
        }
        let range = node.range;
        let tree = match node.kind {
            | ExprKind::Number(value, integer, spelling) => {
                let tree = self.tree(&spelling, range, Vec::new())?;
                self.emit_node(Op::Push(value, integer), range, tree)?;
                self.mark_macro(node.macro_use);
                tree
            },
            | ExprKind::Real(rounded, spelling) => {
                let tree = self.tree(&spelling, range, Vec::new())?;
                self.emit_node(
                    Op::PushReal {
                        value: rounded.value,
                        exact: rounded.exact,
                    },
                    range,
                    tree,
                )?;
                self.mark_macro(node.macro_use);
                tree
            },
            | ExprKind::Character(value, spelling) => {
                let tree = self.tree(&spelling, range, Vec::new())?;
                self.emit_node(Op::Push(value, Integer::Int), range, tree)?;
                self.mark_macro(node.macro_use);
                tree
            },
            | ExprKind::Sizeof(size, ty) => {
                let text = self.source.get(range.start..range.end).unwrap_or("sizeof");
                let tree = self.tree(text, range, Vec::new())?;
                self.emit_node(Op::Sizeof(size, ty), range, tree)?;
                tree
            },
            | ExprKind::Update {
                target,
                delta,
                prefix,
                clash,
            } => {
                let nested = self.nested(clash);
                self.emit_increment(target, delta, prefix, range, Some(nested), false)?
            },
            | ExprKind::Assign { clash, .. } => {
                let nested = self.nested(clash);
                self.emit_assignment(expression, Some(nested), false)?
            },
            | ExprKind::Name(name, declared, place) if declared.is_array() => {
                let tree = self.tree(&name, range, Vec::new())?;
                self.emit_node(Op::Decay(place), range, tree)?;
                self.mark_macro(node.macro_use);
                tree
            },
            | ExprKind::Cast(to, child) => {
                let from = self.expressions[child].ty;
                let operand = self.emit_expression(child, false)?;
                let spelling = to.spelling(&self.structures);
                let tree = self.tree(&format!("({spelling})"), range, vec![operand])?;
                self.emit_node(Op::Cast { from, to }, range, tree)?;
                tree
            },
            | ExprKind::Name(name, declared, place) => {
                let tree = self.tree(&name, range, Vec::new())?;
                self.emit_node(Op::Address(place), range, tree)?;
                if !address {
                    self.load(declared, range, tree)?;
                }
                tree
            },
            | ExprKind::Member {
                base,
                name,
                offset,
                arrow,
                declared,
            } => {
                let operand = self.emit_expression(base, !arrow)?;
                let label = if arrow {
                    format!("->{name}")
                } else {
                    format!(".{name}")
                };
                let tree = self.tree(&label, range, vec![operand])?;
                let array = declared.length();
                self.emit_node(
                    Op::Member {
                        name,
                        offset,
                        arrow,
                        structure: structure_of(declared),
                        array,
                    },
                    range,
                    tree,
                )?;
                if !address && array.is_none() {
                    self.load(declared, range, tree)?;
                }
                tree
            },
            | ExprKind::Unary("&", child) => {
                // C99: §6.5.3.2 paragraph 3, pp. 78-79; PDF pp. 90-91.
                // &*p and &a[n] cancel indirection.
                match self.expressions[child].kind.clone() {
                    | ExprKind::Unary("*", pointer) => {
                        let operand = self.emit_expression(pointer, false)?;
                        self.tree("&*", range, vec![operand])?
                    },
                    | ExprKind::Index(base, index) => {
                        let base = self.emit_expression(base, false)?;
                        let index = self.emit_expression(index, false)?;
                        let tree = self.tree("&[]", range, vec![base, index])?;
                        self.emit_node(Op::ElementAddress, range, tree)?;
                        tree
                    },
                    | _ => {
                        let operand = self.emit_expression(child, true)?;
                        let tree = self.tree("&", range, vec![operand])?;
                        self.emit_node(Op::AddressOf, range, tree)?;
                        tree
                    },
                }
            },
            | ExprKind::Unary("*", child) => {
                let operand = self.emit_expression(child, false)?;
                let tree = self.tree("*", range, vec![operand])?;
                self.emit_node(Op::Dereference, range, tree)?;
                if !address {
                    self.load(node.ty, range, tree)?;
                }
                tree
            },
            | ExprKind::Unary(operator, child) => {
                let operand = self.emit_expression(child, false)?;
                if operator != "!" {
                    self.promoted(child, operand)?;
                }
                let integer = match (operator, node.ty) {
                    | ("!", _) => None,
                    | (_, ty) => ty.arithmetic(),
                };
                let tree = self.tree(operator, range, vec![operand])?;
                self.emit_node(Op::Unary(operator, integer), range, tree)?;
                tree
            },
            // The first operand, a test that a sequence point ends, and the
            // second only when the first leaves the result open (C99
            // §6.5.13p4, §6.5.14p4).
            | ExprKind::Binary(operator @ ("&&" | "||"), lhs, rhs) => {
                let left = self.emit_first(lhs, operator)?;
                let test = self.code.len();
                self.emit(
                    Op::Test {
                        operator,
                        target: 0,
                    },
                    range,
                )?;
                let right = self.emit_expression(rhs, false)?;
                let tree = self.tree(operator, range, vec![left, right])?;
                self.emit_node(
                    Op::Yield {
                        operator,
                        to: None,
                        target: None,
                    },
                    range,
                    tree,
                )?;
                self.close_test(test, tree);
                tree
            },
            // The condition, a test that a sequence point ends, then one
            // operand, converted to the result's type (C99 §6.5.15p4-5).
            | ExprKind::Conditional(condition, then, otherwise) => {
                let first = self.emit_first(condition, "?:")?;
                let test = self.code.len();
                self.emit(
                    Op::Test {
                        operator: "?:",
                        target:   0,
                    },
                    range,
                )?;
                let second = self.emit_expression(then, false)?;
                let skip = self.code.len();
                let to = |parser: &Self, operand: usize| {
                    (parser.expressions[operand].ty != node.ty).then_some(node.ty)
                };
                self.emit(
                    Op::Yield {
                        operator: "?:",
                        to:       to(self, then),
                        target:   Some(0),
                    },
                    range,
                )?;
                self.close_test(test, usize::MAX);
                let third = self.emit_expression(otherwise, false)?;
                let tree = self.tree("?:", range, vec![first, second, third])?;
                self.emit_node(
                    Op::Yield {
                        operator: "?:",
                        to:       to(self, otherwise),
                        target:   None,
                    },
                    range,
                    tree,
                )?;
                self.close_test(skip, tree);
                self.code[test].node = Some(tree);
                tree
            },
            // The left operand, discarded at a sequence point, then the
            // right one, which gives the result (C99 §6.5.17p2).
            | ExprKind::Comma(lhs, rhs) => {
                let left = self.emit_first(lhs, ",")?;
                let comma = self.code.len();
                self.emit(Op::Comma { discard: true }, range)?;
                let right = self.emit_expression(rhs, false)?;
                let tree = self.tree(",", range, vec![left, right])?;
                self.emit_node(
                    Op::Yield {
                        operator: ",",
                        to:       None,
                        target:   None,
                    },
                    range,
                    tree,
                )?;
                self.code[comma].node = Some(tree);
                tree
            },
            | ExprKind::Binary(operator, lhs, rhs) => {
                // The usual arithmetic conversions apply only when both
                // operands are arithmetic; pointer arithmetic promotes nothing.
                // With a floating operand the other converts straight to its
                // type, without the integer promotions (C99 §6.3.1.8p1).
                let types = (self.expressions[lhs].ty, self.expressions[rhs].ty);
                let integers = matches!(types, (Type::Integer(_), Type::Integer(_)));
                let arithmetic =
                    types
                        .0
                        .arithmetic()
                        .zip(types.1.arithmetic())
                        .map(|(left, right)| {
                            if integers {
                                (left.promoted(), right.promoted())
                            } else {
                                (left, right)
                            }
                        });
                let left = self.emit_expression(lhs, false)?;
                if integers {
                    self.promoted(lhs, left)?;
                }
                let right = self.emit_expression(rhs, false)?;
                if integers {
                    self.promoted(rhs, right)?;
                }
                let computation = match arithmetic {
                    // A shift promotes each operand on its own; nothing
                    // brings them to a common type (C99 §6.5.7p3).
                    | Some((left, _)) if matches!(operator, "<<" | ">>") => Some(left),
                    | Some((promoted_left, promoted_right)) => {
                        let common = promoted_left.common(promoted_right);
                        if promoted_left != common {
                            let range = self.expressions[lhs].range;
                            self.emit_node(
                                Op::Convert {
                                    from:  promoted_left,
                                    to:    common,
                                    other: promoted_right,
                                    depth: 1,
                                },
                                range,
                                left,
                            )?;
                        }
                        if promoted_right != common {
                            let range = self.expressions[rhs].range;
                            self.emit_node(
                                Op::Convert {
                                    from:  promoted_right,
                                    to:    common,
                                    other: promoted_left,
                                    depth: 0,
                                },
                                range,
                                right,
                            )?;
                        }
                        Some(common)
                    },
                    | None => None,
                };
                let tree = self.tree(operator, range, vec![left, right])?;
                self.emit_node(Op::Binary(operator, computation), range, tree)?;
                tree
            },
            | ExprKind::Index(base, index) => {
                let base = self.emit_expression(base, false)?;
                let index = self.emit_expression(index, false)?;
                let tree = self.tree("[]", range, vec![base, index])?;
                self.emit_node(Op::Index, range, tree)?;
                if !address {
                    self.emit_node(Op::Load, range, tree)?;
                }
                tree
            },
            | ExprKind::Call(name, arguments) => {
                let mut children = Vec::new();
                let mut types = Vec::new();
                let after = self.next_point;
                self.next_point = SequencePoint::Call(range);
                for &argument in &arguments {
                    children.push(self.emit_expression(argument, false)?);
                    types.push(self.expressions[argument].ty);
                }
                self.next_point = after;
                let tree = self.tree(&format!("{name}()"), range, children)?;
                self.emit_node(Op::Call(name, types), range, tree)?;
                tree
            },
            | ExprKind::Library(library, arguments) => {
                let mut children = Vec::new();
                let mut types = Vec::new();
                let after = self.next_point;
                self.next_point = SequencePoint::Call(range);
                for (index, &argument) in arguments.iter().enumerate() {
                    let child = self.emit_expression(argument, false)?;
                    let ty = self.expressions[argument].ty;
                    // printf's arguments after the format pass through
                    // `...`, so the default argument promotions apply: a
                    // char arrives as an int (C99 §6.5.2.2p6-7). The
                    // prototype's parameters convert as if by assignment.
                    let passed = match (library, index, ty) {
                        | (Library::Printf, 1.., Type::Integer(integer))
                            if integer.promoted() != integer =>
                        {
                            let range = self.expressions[argument].range;
                            self.emit_node(
                                Op::DefaultPromote(Arithmetic::Integer(integer)),
                                range,
                                child,
                            )?;
                            Type::Integer(integer.promoted())
                        },
                        // A float passed through `...` arrives as a double
                        // (C99 §6.5.2.2p6).
                        | (Library::Printf, 1.., Type::Floating(Floating::Float)) => {
                            let range = self.expressions[argument].range;
                            self.emit_node(
                                Op::DefaultPromote(Arithmetic::Floating(Floating::Float)),
                                range,
                                child,
                            )?;
                            Type::Floating(Floating::Double)
                        },
                        | (Library::Printf, 1.., _) => ty,
                        | (Library::Putchar, ..) => Type::INT,
                        | _ => Type::Pointer(Arithmetic::CHAR, true),
                    };
                    children.push(child);
                    types.push(passed);
                }
                self.next_point = after;
                let tree = self.tree(&format!("{}()", library.name()), range, children)?;
                self.emit_node(Op::Library(library, types), range, tree)?;
                tree
            },
        };
        Ok(tree)
    }

    /// Where an update emitted now sits: the sequence point that next
    /// follows it, and whether its full expression is a condition.
    const fn nested(&self, clash: Clash) -> Nested {
        Nested {
            next: self.next_point,
            clash,
            condition: self.in_condition,
        }
    }

    /// Emit `++` or `--` of `target` at `range`: its lvalue, then one step
    /// that reads, adds or subtracts 1, and stores. `nested` when a larger
    /// expression uses its value; `full_expression` when it ends a
    /// statement.
    pub(super) fn emit_increment(
        &mut self,
        target: usize,
        delta: i32,
        prefix: bool,
        range: SourceRange,
        nested: Option<Nested>,
        full_expression: bool,
    ) -> Result<usize, Diagnostic> {
        let operand = self.emit_expression(target, true)?;
        let label = if delta > 0 { "++" } else { "--" };
        let tree = self.tree(label, range, vec![operand])?;
        self.emit_node(
            Op::Increment {
                delta,
                prefix,
                nested,
                full_expression,
            },
            range,
            tree,
        )?;
        Ok(tree)
    }

    /// Emit the assignment `expression`: its left operand's lvalue, its
    /// right operand, and one step that stores (for `=`) or reads, combines
    /// and stores (for a compound assignment, which evaluates its left
    /// operand once). `nested` when a larger expression uses its value;
    /// `full_expression` when it ends a statement.
    ///
    /// C99: §6.5.16.1 paragraph 2, p. 92; PDF p. 104.
    /// C99: §6.5.16.2 paragraph 3, p. 93; PDF p. 105.
    pub(super) fn emit_assignment(
        &mut self,
        expression: usize,
        nested: Option<Nested>,
        full_expression: bool,
    ) -> Result<usize, Diagnostic> {
        let ExprKind::Assign {
            target,
            value,
            operator,
            common,
            right_type,
            ..
        } = self.expressions[expression].kind
        else {
            return Err(self.error("expected an assignment"));
        };
        let range = self.expressions[expression].range;
        let left = self.expressions[target].ty;
        let right = self.expressions[value].ty;
        let target_node = self.emit_expression(target, true)?;
        let operand = self.emit_expression(value, false)?;
        if operator == "=" {
            let tree = self.tree("=", range, vec![target_node, operand])?;
            self.emit_node(
                Op::Store {
                    initializer: false,
                    full_expression,
                    from: right,
                    structure: structure_of(left),
                    nested,
                },
                range,
                tree,
            )?;
            return Ok(tree);
        }
        if let Type::Integer(from) = right
            && from.promoted() != from
            && matches!(left, Type::Integer(_))
        {
            let value_range = self.expressions[value].range;
            self.emit_node(Op::Promote(from), value_range, operand)?;
        }
        let tree = self.tree(operator, range, vec![target_node, operand])?;
        let computed = compound_operator(operator).unwrap_or("+");
        self.emit_node(
            Op::Compound(computed, common, right_type, full_expression, nested),
            range,
            tree,
        )?;
        Ok(tree)
    }

    /// Point the `Test` or `Yield` at `at` to the next operation emitted,
    /// and, unless `tree` is `usize::MAX`, tie it to the expression-tree
    /// node `tree` it is part of, which exists only once all its operands
    /// do.
    fn close_test(&mut self, at: usize, tree: usize) {
        let end = self.code.len();
        if tree != usize::MAX {
            self.code[at].node = Some(tree);
        }
        match &mut self.code[at].op {
            | Op::Test { target, .. } => *target = end,
            | Op::Yield { target, .. } => *target = Some(end),
            | _ => {},
        }
    }

    /// Read the value of the lvalue just emitted, of type `ty`: one scalar,
    /// or a copy of a whole structure.
    fn load(&mut self, ty: Type, range: SourceRange, tree: usize) -> Result<(), Diagnostic> {
        match ty {
            | Type::Struct(id) => self.emit_node(Op::LoadStruct(id), range, tree),
            | _ => self.emit_node(Op::Load, range, tree),
        }
    }

    /// Promote the value of `expression`, whose tree node is `tree`, to int
    /// when its type is narrower (char, short and their signed and unsigned
    /// versions). Only the operands of unary `+` and `-`, of the shifts and
    /// of the usual arithmetic conversions are promoted here: a char that is
    /// stored, returned, passed to an int parameter, tested, or discarded
    /// keeps its value without promotion. (Every call has a prototype, so the
    /// default argument promotions apply only to printf's `...`.)
    /// C99: §6.3.1.1 paragraph 2 and footnote 48, pp. 42-43; PDF pp. 54-55.
    fn promoted(&mut self, expression: usize, tree: usize) -> Result<(), Diagnostic> {
        if let Type::Integer(integer) = self.expressions[expression].ty
            && integer.promoted() != integer
        {
            let range = self.expressions[expression].range;
            self.emit_node(Op::Promote(integer), range, tree)?;
        }
        Ok(())
    }

    pub(super) fn check_lvalue(&self, expression: usize) -> Result<(), Diagnostic> {
        match self.expressions[expression].kind {
            | ExprKind::Name(_, declared, _) if !declared.is_array() => Ok(()),
            | ExprKind::Unary("*", _) | ExprKind::Index(_, _) => Ok(()),
            | ExprKind::Member { declared, .. } if !declared.is_array() => Ok(()),
            | _ => Err(Diagnostic::new(
                "unsupported",
                self.expressions[expression].range,
                "a modifiable scalar lvalue is required; whole-array writes and pointer-to-array \
                 types are outside this subset",
            )),
        }
    }
}

#[derive(Debug, Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent facts the type checker records about one expression."
)]
pub(super) struct Expr {
    pub kind:          ExprKind,
    pub ty:            Type,
    pub call:          bool,
    pub range:         SourceRange,
    pub height:        usize,
    pub parenthesized: bool,
    /// A `const`-qualified lvalue, which no store may modify: an object
    /// defined `const`, or one reached through a pointer to `const`.
    pub readonly:      bool,
    /// The macro whose replacement this constant or literal is.
    pub macro_use:     Option<usize>,
    /// Whether the expression contains a `++` or `--`, which only a
    /// condition may (see `check_update`).
    pub effect:        bool,
}

/// What part of an expression can reach, for `check_update`.
#[derive(Debug, Default)]
struct Reach {
    /// The variables its `++`, `--` and assignments update by name, with
    /// the update's node.
    updates: Vec<(String, usize)>,
    /// The variables it names, other than as an update's operand.
    names:   Vec<String>,
    /// Its first `*`, `[]`, `->` or call, which might reach any object.
    opaque:  Option<SourceRange>,
    /// The updates whose sequencing the run must check: an update through
    /// `*`, `[]`, `->` or a member, and one that such an access or a call
    /// in another unsequenced operand might reach.
    checked: Vec<usize>,
    /// The updates a sequence point inside the part completes: in the
    /// first operand of `&&`, `||`, `?:` or a comma, or in a call's
    /// arguments. They come before an assignment around the part stores,
    /// though not before the part's unsequenced siblings.
    sealed:  Vec<usize>,
}

impl Reach {
    fn opaque(range: SourceRange) -> Self {
        Self {
            opaque: Some(range),
            ..Self::default()
        }
    }

    /// An update of an object only the run decides: it might reach
    /// anything, and its sequencing is the run's to check.
    fn checked(range: SourceRange, node: usize) -> Self {
        Self {
            opaque: Some(range),
            checked: vec![node],
            ..Self::default()
        }
    }

    fn join(&mut self, other: Self) {
        self.updates.extend(other.updates);
        self.names.extend(other.names);
        self.opaque = self.opaque.or(other.opaque);
        self.checked.extend(other.checked);
        self.sealed.extend(other.sealed);
    }

    /// Mark every update so far as completed by a sequence point that
    /// follows the part.
    fn seal(&mut self) {
        self.sealed.extend(
            self.updates
                .iter()
                .map(|&(_, node)| node)
                .chain(self.checked.iter().copied()),
        );
    }
}

#[derive(Debug, Clone)]
pub(super) enum ExprKind {
    /// An integer constant's value, type and spelling.
    Number(i128, Integer, String),
    /// A floating constant's value (with whether it is exact) and spelling.
    Real(Rounded, String),
    /// A character constant's value and its spelling, `'a'`.
    Character(i128, String),
    /// `sizeof`: the size in bytes of the operand's type, and the type.
    Sizeof(usize, Type),
    /// `++`/`--` of `target`; `clash` says what else in the full
    /// expression could reach the object (see `check_update`).
    Update {
        target: usize,
        delta:  i32,
        prefix: bool,
        clash:  Clash,
    },
    /// `target = value`, or `target op= value` for the compound
    /// `operator` (`+=`), which computes in `common` with the right
    /// operand's (promoted) type `right_type`; `clash` as for `Update`.
    Assign {
        target:     usize,
        value:      usize,
        operator:   &'static str,
        common:     Option<Arithmetic>,
        right_type: Option<Arithmetic>,
        clash:      Clash,
    },
    Name(String, Type, Place),
    /// `base.name` or `base->name`; `offset` is the member's first leaf and
    /// `declared` its type.
    Member {
        base:     usize,
        name:     String,
        offset:   i32,
        arrow:    bool,
        declared: Type,
    },
    Unary(&'static str, usize),
    Binary(&'static str, usize, usize),
    Index(usize, usize),
    /// `(type) operand`, converting to the type.
    Cast(Type, usize),
    Call(String, Vec<usize>),
    /// `condition ? then : otherwise`.
    Conditional(usize, usize, usize),
    /// `left, right`: the comma operator, in parentheses.
    Comma(usize, usize),
    /// A call to a function `<stdio.h>` declares.
    Library(Library, Vec<usize>),
}

/// The binary operator `text` spells and its precedence, lowest first
/// (C99 §6.5.5-§6.5.14, whose order of subclauses is the precedence).
fn binary(text: &str) -> Option<(&'static str, u8)> {
    Some(match text {
        | "||" => ("||", 1),
        | "&&" => ("&&", 2),
        | "|" => ("|", 3),
        | "^" => ("^", 4),
        | "&" => ("&", 5),
        | "==" => ("==", 6),
        | "!=" => ("!=", 6),
        | "<" => ("<", 7),
        | "<=" => ("<=", 7),
        | ">" => (">", 7),
        | ">=" => (">=", 7),
        | "<<" => ("<<", 8),
        | ">>" => (">>", 8),
        | "+" => ("+", 9),
        | "-" => ("-", 9),
        | "*" => ("*", 10),
        | "/" => ("/", 10),
        | "%" => ("%", 10),
        | _ => return None,
    })
}

/// The assignment operator `text` spells: `=` or a compound one such as
/// `+=` (C99 §6.5.16p1).
fn assignment_operator(text: &str) -> &'static str {
    match text {
        | "+=" => "+=",
        | "-=" => "-=",
        | "*=" => "*=",
        | "/=" => "/=",
        | "%=" => "%=",
        | "&=" => "&=",
        | "^=" => "^=",
        | "|=" => "|=",
        | "<<=" => "<<=",
        | ">>=" => ">>=",
        | _ => "=",
    }
}

/// The precedence of a unary operator's operand: above every binary
/// operator (C99 §6.5.3).
const UNARY: u8 = 11;

fn unary(text: &str) -> Option<&'static str> {
    match text {
        | "+" => Some("+"),
        | "-" => Some("-"),
        | "!" => Some("!"),
        | "~" => Some("~"),
        | "*" => Some("*"),
        | "&" => Some("&"),
        | _ => None,
    }
}

/// Whether pointers of types `left` and `right` point to the same type,
/// up to the `const` on what a single `int *` or `char *` points to: what
/// subtracting or ordering them needs (C99 §6.5.6p3, §6.5.8p2).
fn same_pointee(left: Type, right: Type) -> bool {
    match (left, right) {
        | (Type::Pointer(a, _), Type::Pointer(b, _)) => a == b,
        | _ => left == right,
    }
}

/// A numeric constant's value: an integer constant's value and type, or a
/// floating constant's rounded value.
#[derive(Debug, Clone, Copy)]
pub(super) enum Number {
    Integer(i128, Integer),
    Real(Rounded),
}

pub(super) fn combined(left: SourceRange, right: SourceRange) -> SourceRange {
    SourceRange {
        end: right.end,
        end_line: right.end_line,
        end_column: right.end_column,
        ..left
    }
}

/// Whether `word` starts a type name, as after the `(` of a cast.
fn is_type_word(word: &str) -> bool {
    matches!(
        word,
        "int"
            | "char"
            | "const"
            | "unsigned"
            | "signed"
            | "short"
            | "long"
            | "float"
            | "double"
            | "void"
            | "struct"
            | "union"
            | "enum"
            | "volatile"
            | "_Bool"
    )
}
