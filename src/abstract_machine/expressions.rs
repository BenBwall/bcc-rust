//! Pratt parsing and typed lowering of side-effect-free operands and whole
//! calls. Assignments/increments belong to statements, never nested C
//! expressions. C99: bounded subset of §6.5, pp. 67-94; PDF pp. 79-106.

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
        Diagnostic,
        Library,
        MAX_ARGUMENTS,
        MAX_CODE,
        MAX_DEPTH,
        Op,
        Place,
        SourceRange,
        Type,
    },
    parser::{
        CallSite,
        Parser,
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
            if lhs.call || rhs.call {
                return Err(self.error(
                    "calls must be whole expressions; operand sequencing is outside this subset",
                ));
            }
            if lhs.effect && rhs.effect {
                return Err(self.error(
                    "two ++ or -- in one condition are outside this subset: without a sequence \
                     point between them their order is open",
                ));
            }
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
                | ("==" | "!=", Type::Pointer(a, _), Type::Pointer(b, _)) if a == b => Type::INT,
                | ("==" | "!=", Type::StructPointer(a), Type::StructPointer(b)) if a == b =>
                    Type::INT,
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
                | ("+" | "-", pointer @ Type::Pointer(..), Type::Integer(_))
                | ("+", Type::Integer(_), pointer @ Type::Pointer(..)) => pointer,
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
        Ok(left)
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
        if self.updates && (self.at("++") || self.at("--")) {
            self.position += 1;
            let child = self.expression(6)?;
            return self.update(child, token, true);
        }
        if let Some(operator) = unary(&text) {
            self.position += 1;
            let child = self.expression(6)?;
            let expression = &self.expressions[child];
            if expression.call {
                return Err(self.error("nested calls are outside this subset"));
            }
            let readonly = expression.readonly;
            let ty = match (operator, expression.ty) {
                | ("+" | "-", Type::Integer(integer)) => Type::Integer(integer.promoted()),
                | ("+" | "-", floating @ Type::Floating(_)) => floating,
                | (
                    "!",
                    Type::Integer(_)
                    | Type::Floating(_)
                    | Type::Pointer(..)
                    | Type::StructPointer(_),
                ) => Type::INT,
                | ("*", Type::Pointer(element, _)) => element.ty(),
                | ("*", Type::StructPointer(id)) => self.complete_type(id)?,
                | ("&", ty @ (Type::Integer(_) | Type::Floating(_))) => {
                    self.check_lvalue(child)?;
                    Type::Pointer(ty.arithmetic().unwrap_or(Arithmetic::INT), readonly)
                },
                | ("&", Type::Struct(id)) => {
                    self.check_lvalue(child)?;
                    Type::StructPointer(id)
                },
                | _ =>
                    return Err(self.error(
                        "unsupported unary operand type; pointers to numbers and to structures \
                         are modeled",
                    )),
            };
            if operator == "&" && expression.effect {
                return Err(self.error("the address of an increment's result is no lvalue"));
            }
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
            let value = self.expression(0)?;
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
                        if expression.call {
                            return Err(self.error("nested function calls are outside this subset"));
                        }
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
                self.node(
                    ExprKind::Call(text, arguments),
                    returns,
                    true,
                    combined(range, closing),
                    height,
                )?
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
            if self.updates && (self.at("++") || self.at("--")) {
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
            let element = match base.ty {
                | Type::Pointer(element, _) => Some(element.ty()),
                | _ => None,
            };
            let Some(element) = element.filter(|_| {
                matches!(subscript.ty, Type::Integer(_)) && !base.call && !subscript.call
            }) else {
                return Err(self.error(
                    "subscript requires a pointer to an arithmetic type and a side-effect-free \
                     integer index",
                ));
            };
            let readonly = base.ty.points_to_const();
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

    /// `++operand`, `--operand`, `operand++` or `operand--` inside a
    /// condition, which then uses its value: the new one for a prefix
    /// operator, the old one for a postfix one (C99 §6.5.2.4p2,
    /// §6.5.3.1p2). `condition` checks that nothing else in the condition
    /// could touch the object.
    fn update(
        &mut self,
        operand: usize,
        operator: Token<'_>,
        prefix: bool,
    ) -> Result<usize, Diagnostic> {
        let expression = &self.expressions[operand];
        if !matches!(
            expression.ty,
            Type::Integer(_) | Type::Floating(_) | Type::Pointer(..)
        ) {
            return Err(self.error("++ and -- update a number or a pointer to one in this subset"));
        }
        if expression.readonly {
            return Err(self.readonly_error(operand, true));
        }
        if expression.effect || expression.call {
            return Err(self.error(
                "a ++ or -- of an expression with side effects of its own is outside this subset",
            ));
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
            },
            ty,
            false,
            range,
            height,
        )?;
        self.expressions[node].effect = true;
        Ok(node)
    }

    /// Check a condition that contains a `++` or `--`: it may contain only
    /// that one, of a variable named directly, and nothing else that could
    /// read or write the variable (no other use of its name, no `*`, `[]`,
    /// `->` or call), so that no other access can be unsequenced with the
    /// update (C99 §6.5p2).
    pub(super) fn check_update(&self, expression: usize) -> Result<(), Diagnostic> {
        let mut names = Vec::new();
        let mut updated = None;
        let mut indirect = false;
        self.walk(expression, &mut names, &mut updated, &mut indirect);
        let Some((name, range)) = updated else {
            return Ok(());
        };
        let message = if name.is_empty() {
            "in a condition, ++ and -- may update only a variable named directly, such as `i--`"
        } else if indirect || names.iter().filter(|other| **other == name).count() > 1 {
            "a condition with ++ or -- may not otherwise use its variable, or use *, [], -> or a \
             call, in this subset: without a sequence point between them, an access to the same \
             object would be undefined (C99 §6.5p2)"
        } else {
            return Ok(());
        };
        Err(Diagnostic::new("unsupported", range, message))
    }

    /// Collect the names `expression` mentions, the variable an update in it
    /// updates (empty when it is not a name), and whether it reaches an
    /// object any other way.
    fn walk(
        &self,
        expression: usize,
        names: &mut Vec<String>,
        updated: &mut Option<(String, SourceRange)>,
        indirect: &mut bool,
    ) {
        let node = &self.expressions[expression];
        match &node.kind {
            | ExprKind::Name(name, ..) => names.push(name.clone()),
            | ExprKind::Update { target, .. } => {
                let name = match &self.expressions[*target].kind {
                    | ExprKind::Name(name, ..) => name.clone(),
                    | _ => String::new(),
                };
                *updated = Some((name, node.range));
                self.walk(*target, names, updated, indirect);
            },
            | ExprKind::Unary(operator, child) => {
                *indirect |= *operator == "*";
                self.walk(*child, names, updated, indirect);
            },
            | ExprKind::Cast(_, child) => self.walk(*child, names, updated, indirect),
            | ExprKind::Binary(_, left, right) => {
                self.walk(*left, names, updated, indirect);
                self.walk(*right, names, updated, indirect);
            },
            | ExprKind::Index(base, index) => {
                *indirect = true;
                self.walk(*base, names, updated, indirect);
                self.walk(*index, names, updated, indirect);
            },
            | ExprKind::Member { base, arrow, .. } => {
                *indirect |= *arrow;
                self.walk(*base, names, updated, indirect);
            },
            | ExprKind::Call(..) | ExprKind::Library(..) => *indirect = true,
            | ExprKind::Number(..)
            | ExprKind::Real(..)
            | ExprKind::Character(..)
            | ExprKind::Sizeof(..) => {},
        }
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
            let operand = self.expression(6)?;
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

    /// A type name, as in a cast or `sizeof`: an integer type or a
    /// structure, optionally `const`, optionally a pointer to it.
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
        let pointer = self.pointer()?;
        if !self.at(")") {
            return Err(self.error(
                "a type name in this subset is an arithmetic type or a structure, or a pointer to \
                 one (such as `unsigned int *` or `double`); pointers to pointers and other \
                 declarators are outside it",
            ));
        }
        let range = combined(start.range, self.tokens[self.position - 1].range);
        self.declared_type(base, pointer, None, range)
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

    /// The value and type of the integer constant `token`: decimal digits
    /// and an optional suffix. Octal and hexadecimal constants are outside
    /// the subset.
    ///
    /// C99: §6.4.4.1 paragraphs 1 and 5, pp. 54-56; PDF pp. 66-68.
    pub(super) fn integer_constant(token: Token<'_>) -> Result<(i128, Integer), Diagnostic> {
        let text = token.text;
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        let (number, suffix) = text.split_at(digits);
        let error = |kind, message: &str| Err(Diagnostic::new(kind, token.range, message));
        if (number.len() > 1 && number.starts_with('0')) || suffix.starts_with(['x', 'X']) {
            return error(
                "unsupported",
                "octal and hexadecimal integer constants are outside this subset; write the value \
                 in decimal",
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
        let Some(integer) = number
            .parse::<u64>()
            .ok()
            .and_then(|value| Integer::constant(i128::from(value), suffix).map(|ty| (value, ty)))
        else {
            return error(
                "unsupported",
                &format!(
                    "no type in the list for `{text}` can hold its value, and bcc has no wider \
                     extended integer type to give it (C99 §6.4.4.1p5-6); the largest decimal \
                     constant without a u suffix is 9223372036854775807"
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
        let child = self.expression(6)?;
        let operand = &self.expressions[child];
        if operand.call {
            return Err(self.error(
                "a cast of a call's result is outside this subset; store the result first",
            ));
        }
        let from = operand.ty;
        let allowed = match (from, to) {
            | (from, to) if from.arithmetic().is_some() && to.arithmetic().is_some() => true,
            | (Type::Pointer(a, _), Type::Pointer(b, _)) => a == b,
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
        self.node(
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
        )
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
                if expression.call {
                    return Err(self.error("nested function calls are outside this subset"));
                }
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
        self.node(
            ExprKind::Library(library, arguments),
            Type::INT,
            true,
            whole,
            height,
        )
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
            } => {
                let operand = self.emit_expression(target, true)?;
                let label = if delta > 0 { "++" } else { "--" };
                let tree = self.tree(label, range, vec![operand])?;
                self.emit_node(
                    Op::Increment {
                        delta,
                        prefix,
                        nested: true,
                    },
                    range,
                    tree,
                )?;
                tree
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
                let array = match declared {
                    | Type::Array(_, length) => Some(length),
                    | _ => None,
                };
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
                for &argument in &arguments {
                    children.push(self.emit_expression(argument, false)?);
                    types.push(self.expressions[argument].ty);
                }
                let tree = self.tree(&format!("{name}()"), range, children)?;
                self.emit_node(Op::Call(name, types), range, tree)?;
                tree
            },
            | ExprKind::Library(library, arguments) => {
                let mut children = Vec::new();
                let mut types = Vec::new();
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
                let tree = self.tree(&format!("{}()", library.name()), range, children)?;
                self.emit_node(Op::Library(library, types), range, tree)?;
                tree
            },
        };
        Ok(tree)
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
    /// `++`/`--` of `target`, inside a condition.
    Update {
        target: usize,
        delta:  i32,
        prefix: bool,
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
    /// A call to a function `<stdio.h>` declares.
    Library(Library, Vec<usize>),
}

fn binary(text: &str) -> Option<(&'static str, u8)> {
    Some(match text {
        | "==" => ("==", 1),
        | "!=" => ("!=", 1),
        | "<" => ("<", 2),
        | "<=" => ("<=", 2),
        | ">" => (">", 2),
        | ">=" => (">=", 2),
        | "<<" => ("<<", 3),
        | ">>" => (">>", 3),
        | "+" => ("+", 4),
        | "-" => ("-", 4),
        | "*" => ("*", 5),
        | "/" => ("/", 5),
        | "%" => ("%", 5),
        | _ => return None,
    })
}

fn unary(text: &str) -> Option<&'static str> {
    match text {
        | "+" => Some("+"),
        | "-" => Some("-"),
        | "!" => Some("!"),
        | "*" => Some("*"),
        | "&" => Some("&"),
        | _ => None,
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
