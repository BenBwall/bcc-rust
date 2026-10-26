//! Pratt parsing and typed lowering of side-effect-free operands and whole
//! calls. Assignments/increments belong to statements, never nested C
//! expressions. C99: bounded subset of §6.5, pp. 67-94; PDF pp. 79-106.

use super::{
    model::{
        Diagnostic,
        MAX_CODE,
        MAX_DEPTH,
        Op,
        SourceRange,
        Type,
    },
    parser::{
        CallSite,
        Parser,
        is_identifier,
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
                | ("==" | "!=", Type::Pointer, Type::Pointer) => Type::Int,
                | ("+" | "-", Type::Pointer, Type::Int | Type::Char)
                | ("+", Type::Int | Type::Char, Type::Pointer) => Type::Pointer,
                | ("==" | "!=", Type::Pointer, Type::Int)
                    if matches!(rhs.kind, ExprKind::Number(0)) =>
                    Type::Int,
                | ("==" | "!=", Type::Int, Type::Pointer)
                    if matches!(lhs.kind, ExprKind::Number(0)) =>
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
        if let Some(operator) = unary(&text) {
            self.position += 1;
            let child = self.expression(6)?;
            let expression = &self.expressions[child];
            if expression.call {
                return Err(self.error("nested calls are outside this subset"));
            }
            if operator == "&" && matches!(expression.kind, ExprKind::Name(_, Type::Char)) {
                return Err(self.error("pointers to char are outside this subset"));
            }
            let ty = match (operator, expression.ty) {
                | ("+" | "-" | "!", Type::Int | Type::Char) | ("!" | "*", Type::Pointer) =>
                    Type::Int,
                | ("&", Type::Int) => {
                    self.check_lvalue(child)?;
                    Type::Pointer
                },
                | _ =>
                    return Err(
                        self.error("unsupported unary operand type; only int pointers are modeled")
                    ),
            };
            return self.node(
                ExprKind::Unary(operator, child),
                ty,
                false,
                combined(range, expression.range),
                expression.height + 1,
            );
        }
        let mut value = if self.take("(") {
            let value = self.expression(0)?;
            let closing = self.current().range;
            self.expect(")")?;
            self.expressions[value].parenthesized = true;
            self.expressions[value].range = combined(range, closing);
            value
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
            self.node(ExprKind::Number(integer), Type::Int, false, range, 1)?
        } else if is_identifier(&text) {
            self.position += 1;
            let binding = self
                .symbols
                .iter()
                .rev()
                .find(|symbol| symbol.name == text)
                .map(|symbol| symbol.ty);
            if self.take("(") {
                if binding.is_some() {
                    return Err(self.error("calling a local object is not supported"));
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
                self.calls.push(CallSite {
                    name: text.clone(),
                    arguments: types,
                    range,
                    caller: self.function_index,
                });
                let closing = self.tokens[self.position - 1].range;
                self.node(
                    ExprKind::Call(text, arguments),
                    Type::Int,
                    true,
                    combined(range, closing),
                    height,
                )?
            } else {
                let Some(declared) = binding else {
                    return Err(self.error("use of an undeclared identifier"));
                };
                // Arrays decay to pointers. A char's value keeps type char:
                // only the operators that need it promote it (see `promoted`).
                // C99: §6.3.2.1 paragraph 2, p. 46; PDF p. 58.
                let ty = match declared {
                    | Type::Array(_) => Type::Pointer,
                    | other => other,
                };
                self.node(ExprKind::Name(text, declared), ty, false, range, 1)?
            }
        } else {
            return Err(self.error(
                "expected a subset expression; nested assignments/increments and unsupported \
                 syntax are rejected",
            ));
        };
        while self.take("[") {
            let index = self.expression(0)?;
            self.expect("]")?;
            let base = &self.expressions[value];
            let subscript = &self.expressions[index];
            if base.ty != Type::Pointer
                || !matches!(subscript.ty, Type::Int | Type::Char)
                || base.call
                || subscript.call
            {
                return Err(self
                    .error("subscript requires an int pointer and a side-effect-free int index"));
            }
            value = self.node(
                ExprKind::Index(value, index),
                Type::Int,
                false,
                combined(base.range, subscript.range),
                base.height.max(subscript.height) + 1,
            )?;
        }
        Ok(value)
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
        });
        Ok(index)
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
                tree
            },
            | ExprKind::Name(name, Type::Array(_)) => {
                let tree = self.tree(&name, range, Vec::new())?;
                self.emit_node(Op::Decay(name), range, tree)?;
                tree
            },
            | ExprKind::Name(name, _) => {
                let tree = self.tree(&name, range, Vec::new())?;
                self.emit_node(Op::Address(name), range, tree)?;
                if !address {
                    self.emit_node(Op::Load, range, tree)?;
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
                        self.emit_node(Op::Binary("+"), range, tree)?;
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
                    self.emit_node(Op::Load, range, tree)?;
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
                let arithmetic = self.expressions[lhs].ty != Type::Pointer
                    && self.expressions[rhs].ty != Type::Pointer;
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
        };
        Ok(tree)
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

    fn check_lvalue(&self, expression: usize) -> Result<(), Diagnostic> {
        match self.expressions[expression].kind {
            | ExprKind::Name(_, Type::Int | Type::Char | Type::Pointer)
            | ExprKind::Unary("*", _)
            | ExprKind::Index(_, _) => Ok(()),
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
}

#[derive(Debug, Clone)]
pub(super) enum ExprKind {
    Number(i32),
    Name(String, Type),
    Unary(&'static str, usize),
    Binary(&'static str, usize, usize),
    Index(usize, usize),
    Call(String, Vec<usize>),
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
