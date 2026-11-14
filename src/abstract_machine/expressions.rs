//! Pratt parsing and typed lowering of side-effect-free operands and whole
//! calls. Assignments/increments belong to statements, never nested C
//! expressions. C99: bounded subset of §6.5, pp. 67-94; PDF pp. 79-106.

use super::{
    lexer::{
        self,
        Token,
    },
    model::{
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
            let ty = match (operator, lhs.ty, rhs.ty) {
                | (_, Type::Int | Type::Char, Type::Int | Type::Char)
                | ("==" | "!=", Type::Pointer(_), Type::Pointer(_))
                | ("==" | "!=", Type::CharPointer(_), Type::CharPointer(_)) => Type::Int,
                | ("==" | "!=", Type::StructPointer(a), Type::StructPointer(b)) if a == b =>
                    Type::Int,
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
                | ("+" | "-", Type::Pointer(constant), Type::Int | Type::Char)
                | ("+", Type::Int | Type::Char, Type::Pointer(constant)) => Type::Pointer(constant),
                | ("+" | "-", Type::CharPointer(constant), Type::Int | Type::Char)
                | ("+", Type::Int | Type::Char, Type::CharPointer(constant)) =>
                    Type::CharPointer(constant),
                | ("==" | "!=", pointer, Type::Int)
                    if pointer.is_pointer() && matches!(rhs.kind, ExprKind::Number(0)) =>
                    Type::Int,
                | ("==" | "!=", Type::Int, pointer)
                    if pointer.is_pointer() && matches!(lhs.kind, ExprKind::Number(0)) =>
                    Type::Int,
                | _ => return Err(self.error("unsupported operand types for this operator")),
            };
            left = self.node(
                ExprKind::Binary(operator, left, right),
                ty,
                false,
                combined(lhs.range, rhs.range),
                lhs.height.max(rhs.height) + 1,
            )?;
        }
        Ok(left)
    }

    fn prefix(&mut self) -> Result<usize, Diagnostic> {
        let token = self.current();
        let range = token.range;
        let text = String::from(token.text);
        if self.at("(") && is_type_word(self.peek(1)) {
            return self.cast();
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
                | ("+" | "-" | "!", Type::Int | Type::Char)
                | ("!" | "*", Type::Pointer(_))
                | ("!", Type::StructPointer(_) | Type::CharPointer(_)) => Type::Int,
                | ("*", Type::CharPointer(_)) => Type::Char,
                | ("*", Type::StructPointer(id)) => self.complete_type(id)?,
                | ("&", Type::Char) => {
                    self.check_lvalue(child)?;
                    Type::CharPointer(readonly)
                },
                | ("&", Type::Int) => {
                    self.check_lvalue(child)?;
                    Type::Pointer(readonly)
                },
                | ("&", Type::Struct(id)) => {
                    self.check_lvalue(child)?;
                    Type::StructPointer(id)
                },
                | _ =>
                    return Err(
                        self.error("unsupported unary operand type; only int pointers are modeled")
                    ),
            };
            let range = combined(range, self.expressions[child].range);
            let height = self.expressions[child].height + 1;
            let node = self.node(ExprKind::Unary(operator, child), ty, false, range, height)?;
            // `*p` for a `const char *p` or a `const int *p` designates an
            // object the program may not modify through it (C99 §6.3.2.1p1).
            self.expressions[node].readonly =
                operator == "*" && self.expressions[child].ty.points_to_const();
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
            let node = self.node(ExprKind::Character(value, text), Type::Int, false, range, 1)?;
            self.expressions[node].macro_use = token.macro_use;
            node
        } else if text.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            if text.len() > 1 && text.starts_with('0') {
                return Err(
                    self.error("only unsuffixed decimal int literals are supported (no octal/hex)")
                );
            }
            let Ok(integer) = text.parse::<i32>() else {
                return Err(self
                    .error("literal must be an unsuffixed decimal value within signed int range"));
            };
            self.position += 1;
            let node = self.node(ExprKind::Number(integer), Type::Int, false, range, 1)?;
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
                            matches!(expression.kind, ExprKind::Number(0)),
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
                let returns = declared.unwrap_or(Type::Int);
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
            if !self.take("[") {
                break;
            }
            let index = self.expression(0)?;
            let closing = self.current().range;
            self.expect("]")?;
            let base = &self.expressions[value];
            let subscript = &self.expressions[index];
            let element = match base.ty {
                | Type::Pointer(_) => Some(Type::Int),
                | Type::CharPointer(_) => Some(Type::Char),
                | _ => None,
            };
            let Some(element) = element.filter(|_| {
                matches!(subscript.ty, Type::Int | Type::Char) && !base.call && !subscript.call
            }) else {
                return Err(self.error(
                    "subscript requires an int or char pointer and a side-effect-free int index",
                ));
            };
            let readonly = base.ty.points_to_const();
            value = self.node(
                ExprKind::Index(value, index),
                element,
                false,
                combined(base.range, closing),
                base.height.max(subscript.height) + 1,
            )?;
            self.expressions[value].readonly = readonly;
        }
        Ok(value)
    }

    /// `(type-name) operand`: a cast to `int`, `char`, or between pointers
    /// to the same type that differ only in `const`. Its operand binds as
    /// a unary operator's does, so `(char)300 + 1` adds 1 to the cast.
    ///
    /// C99: §6.5.4 paragraphs 2 and 4, p. 81; PDF p. 93.
    fn cast(&mut self) -> Result<usize, Diagnostic> {
        let opening = self.current();
        self.position += 1;
        let start = self.current();
        let to = match self.current().text {
            | "unsigned" =>
                return Err(self.error(
                    "unsigned types are outside this subset, which models int and char; \
                     converting a negative value to an unsigned type wraps it modulo 2^N, so \
                     (unsigned)-1 is 4294967295 (C99 §6.3.1.3p2)",
                )),
            | "void" =>
                return Err(self.error(
                    "a cast to void, which discards a value, is outside this subset; write the \
                     expression as a statement of its own",
                )),
            | "int" | "char" | "const" => {
                let constant = self.take("const");
                let base = if self.take("int") {
                    Type::Int
                } else if self.take("char") {
                    Type::Char
                } else {
                    return Err(self.error(
                        "a cast in this subset names int, char, or a pointer to int or char",
                    ));
                };
                if self.take("*") {
                    if base == Type::Int {
                        Type::Pointer(constant)
                    } else {
                        Type::CharPointer(constant)
                    }
                } else {
                    // A cast yields a value, not an object, so a qualifier
                    // on the type it names changes nothing (C99 §6.5.4,
                    // footnote 89).
                    base
                }
            },
            | _ =>
                return Err(self.error(
                    "a cast in this subset names int, char, or a pointer to int or char; other \
                     types (unsigned, long, float, structures, ...) are outside it",
                )),
        };
        if !self.at(")") {
            return Err(self.error(
                "a cast in this subset names int, char, or a pointer to int or char (such as \
                 `(int *)`); pointers to pointers and other declarators are outside it",
            ));
        }
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
            | (Type::Int | Type::Char, Type::Int | Type::Char)
            | (Type::Pointer(_), Type::Pointer(_))
            | (Type::CharPointer(_), Type::CharPointer(_)) => true,
            | (Type::Struct(_) | Type::Array(_) | Type::CharArray(_), _) =>
                return Err(Diagnostic::constraint(
                    combined(opening.range, operand.range),
                    "a cast converts a scalar, a number or a pointer; a structure has no scalar \
                     value to convert (C99 §6.5.4p2)",
                    &super::clauses::CAST_CONSTRAINT,
                )),
            | _ => false,
        };
        if !allowed {
            return Err(Diagnostic::new(
                "unsupported",
                combined(start.range, closing.range),
                "this subset casts between int and char, and between pointers to the same type \
                 that differ only in const; casts between integers and pointers or between \
                 different pointer types are outside it",
            ));
        }
        let range = combined(opening.range, operand.range);
        let height = operand.height + 1;
        self.node(ExprKind::Cast(to, child), to, false, range, height)
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
        });
        Ok(index)
    }

    /// The value of the character constant `token`: the char its single
    /// character or escape sequence makes, converted to int (C99
    /// §6.4.4.4p10). Plain char is signed on x86-64, so `'\xff'` is -1.
    pub(super) fn character_constant(&self, token: Token<'_>) -> Result<i32, Diagnostic> {
        match *lexer::literal_bytes(self.source, token)? {
            | [byte] => Ok(i32::from(i8::from_ne_bytes([byte]))),
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
                | (Library::Printf | Library::Puts, 0) => matches!(ty, Type::CharPointer(_)),
                | (Library::Putchar, _) => matches!(ty, Type::Int | Type::Char),
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
                        String::from("putchar's argument is a character, an int"),
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
            Type::Int,
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
            | ExprKind::Number(integer) => {
                let tree = self.tree(&format!("{integer}"), range, Vec::new())?;
                self.emit_node(Op::Push(integer), range, tree)?;
                self.mark_macro(node.macro_use);
                tree
            },
            | ExprKind::Character(value, spelling) => {
                let tree = self.tree(&spelling, range, Vec::new())?;
                self.emit_node(Op::Push(value), range, tree)?;
                self.mark_macro(node.macro_use);
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
                let tree = self.tree(&format!("({})", cast_spelling(to)), range, vec![operand])?;
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
                    | Type::Array(length) | Type::CharArray(length) => Some(length),
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
                let tree = self.tree(operator, range, vec![operand])?;
                self.emit_node(Op::Unary(operator), range, tree)?;
                tree
            },
            | ExprKind::Binary(operator, lhs, rhs) => {
                // The usual arithmetic conversions apply only when both
                // operands are arithmetic; pointer arithmetic promotes nothing.
                let arithmetic = !self.expressions[lhs].ty.is_pointer()
                    && !self.expressions[rhs].ty.is_pointer();
                let left = self.emit_expression(lhs, false)?;
                if arithmetic {
                    self.promoted(lhs, left)?;
                }
                let right = self.emit_expression(rhs, false)?;
                if arithmetic {
                    self.promoted(rhs, right)?;
                }
                let tree = self.tree(operator, range, vec![left, right])?;
                self.emit_node(Op::Binary(operator), range, tree)?;
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
                for &argument in &arguments {
                    children.push(self.emit_expression(argument, false)?);
                }
                let tree = self.tree(&format!("{name}()"), range, children)?;
                self.emit_node(Op::Call(name, arguments.len()), range, tree)?;
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
                    let passed = match (library, index) {
                        | (Library::Printf, 1..) if ty == Type::Char => {
                            let range = self.expressions[argument].range;
                            self.emit_node(Op::DefaultPromote, range, child)?;
                            Type::Int
                        },
                        | (Library::Printf, 1..) => ty,
                        | (Library::Putchar, _) => Type::Int,
                        | _ => Type::CharPointer(true),
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

    /// Promote the char value of `expression`, whose tree node is `tree`, to
    /// int. Only the operands of unary `+` and `-` and of the usual arithmetic
    /// conversions are promoted here: a char that is stored, returned, passed
    /// to an int parameter, tested, or discarded keeps its value without
    /// promotion. (Every call has a prototype, so the default argument
    /// promotions never apply.)
    /// C99: §6.3.1.1 paragraph 2 and footnote 48, pp. 42-43; PDF pp. 54-55.
    fn promoted(&mut self, expression: usize, tree: usize) -> Result<(), Diagnostic> {
        if self.expressions[expression].ty == Type::Char {
            let range = self.expressions[expression].range;
            self.emit_node(Op::Promote, range, tree)?;
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
}

#[derive(Debug, Clone)]
pub(super) enum ExprKind {
    Number(i32),
    /// A character constant's value and its spelling, `'a'`.
    Character(i32, String),
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
        | "+" => ("+", 3),
        | "-" => ("-", 3),
        | "*" => ("*", 4),
        | "/" => ("/", 4),
        | "%" => ("%", 4),
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

/// A cast's type as C spells it: `char`, `int *`, `const char *`.
pub(super) const fn cast_spelling(ty: Type) -> &'static str {
    match ty {
        | Type::Char => "char",
        | Type::Pointer(false) => "int *",
        | Type::Pointer(true) => "const int *",
        | Type::CharPointer(false) => "char *",
        | Type::CharPointer(true) => "const char *",
        | _ => "int",
    }
}
