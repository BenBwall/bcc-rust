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
                | (_, Type::Int, Type::Int) | ("==" | "!=", Type::Pointer, Type::Pointer) =>
                    Type::Int,
                | ("+" | "-", Type::Pointer, Type::Int) | ("+", Type::Int, Type::Pointer) =>
                    Type::Pointer,
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
            let ty = match (operator, expression.ty) {
                | ("+" | "-" | "!", Type::Int) | ("!" | "*", Type::Pointer) => Type::Int,
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
            self.expect(")")?;
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
                });
                self.node(
                    ExprKind::Call(text, arguments),
                    Type::Int,
                    true,
                    range,
                    height,
                )?
            } else {
                let Some(declared) = binding else {
                    return Err(self.error("use of an undeclared identifier"));
                };
                let ty = if matches!(declared, Type::Array(_)) {
                    Type::Pointer
                } else {
                    declared
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
            if base.ty != Type::Pointer || subscript.ty != Type::Int || base.call || subscript.call
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
        });
        Ok(index)
    }

    pub(super) fn emit_expression(
        &mut self,
        expression: usize,
        address: bool,
    ) -> Result<(), Diagnostic> {
        let node = self.expressions[expression].clone();
        if address {
            self.check_lvalue(expression)?;
        }
        match node.kind {
            | ExprKind::Number(integer) => {
                self.emit(Op::Push(integer), node.range)?;
            },
            | ExprKind::Name(name, Type::Array(_)) => {
                self.emit(Op::Decay(name), node.range)?;
            },
            | ExprKind::Name(name, _) => {
                self.emit(Op::Address(name), node.range)?;
                if !address {
                    self.emit(Op::Load, node.range)?;
                }
            },
            | ExprKind::Unary("&", child) => {
                // C99: §6.5.3.2 paragraph 3, pp. 78-79; PDF pp. 90-91.
                // &*p and &a[n] cancel indirection.
                match self.expressions[child].kind.clone() {
                    | ExprKind::Unary("*", pointer) => self.emit_expression(pointer, false)?,
                    | ExprKind::Index(base, index) => {
                        self.emit_expression(base, false)?;
                        self.emit_expression(index, false)?;
                        self.emit(Op::Binary("+"), node.range)?;
                    },
                    | _ => {
                        self.emit_expression(child, true)?;
                        self.emit(Op::AddressOf, node.range)?;
                    },
                }
            },
            | ExprKind::Unary("*", child) => {
                self.emit_expression(child, false)?;
                self.emit(Op::Dereference, node.range)?;
                if !address {
                    self.emit(Op::Load, node.range)?;
                }
            },
            | ExprKind::Unary(operator, child) => {
                self.emit_expression(child, false)?;
                self.emit(Op::Unary(operator), node.range)?;
            },
            | ExprKind::Binary(operator, left, right) => {
                self.emit_expression(left, false)?;
                self.emit_expression(right, false)?;
                self.emit(Op::Binary(operator), node.range)?;
            },
            | ExprKind::Index(base, index) => {
                self.emit_expression(base, false)?;
                self.emit_expression(index, false)?;
                self.emit(Op::Index, node.range)?;
                if !address {
                    self.emit(Op::Load, node.range)?;
                }
            },
            | ExprKind::Call(name, arguments) => {
                for &argument in &arguments {
                    self.emit_expression(argument, false)?;
                }
                self.emit(Op::Call(name, arguments.len()), node.range)?;
            },
        }
        Ok(())
    }

    fn check_lvalue(&self, expression: usize) -> Result<(), Diagnostic> {
        match self.expressions[expression].kind {
            | ExprKind::Name(_, Type::Int | Type::Pointer)
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
    pub kind:   ExprKind,
    pub ty:     Type,
    pub call:   bool,
    pub range:  SourceRange,
    pub height: usize,
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

fn combined(left: SourceRange, right: SourceRange) -> SourceRange {
    SourceRange {
        end: right.end,
        end_line: right.end_line,
        end_column: right.end_column,
        ..left
    }
}
