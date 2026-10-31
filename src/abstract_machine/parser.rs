//! Bounded subset compilation. Blocks retain lexical scopes; loops emit jumps.
//! C99: subset of §6.8-§6.9, pp. 131-144; PDF pp. 143-156.
//! C99: §6.5.1 paragraph 2, p. 69; PDF p. 81 (declaration before use).
//! C99: §6.5.2.4 paragraph 2, p. 75; PDF p. 87 and §6.5.3.1 paragraph 2,
//! p. 78; PDF p. 90 (standalone increments; expression results are discarded).

use super::{
    expressions::{
        Expr,
        ExprKind,
        combined,
    },
    lexer::{
        self,
        Token,
    },
    model::{
        Diagnostic,
        ExpressionNode,
        Function,
        Instruction,
        MAX_CODE,
        MAX_DEPTH,
        Op,
        Program,
        SourceRange,
        Type,
    },
};

pub(super) fn compile(source: &str) -> Result<Program, Diagnostic> {
    let mut parser = Parser {
        tokens:         lexer::lex(source)?,
        position:       0,
        code:           Vec::new(),
        expressions:    Vec::new(),
        symbols:        Vec::new(),
        scope:          0,
        nesting:        0,
        calls:          Vec::new(),
        function_index: 0,
        nodes:          Vec::new(),
    };
    let mut functions: Vec<Function> = Vec::new();
    let mut total_code = 0;
    while !parser.at("") {
        parser.function_index = functions.len();
        parser.expect("int")?;
        let name = parser.identifier()?;
        if functions.iter().any(|function| function.name == name.text) {
            return Err(parser.error("duplicate function definition"));
        }
        parser.expect("(")?;
        let mut parameters = Vec::new();
        if parser.take("void") {
            parser.expect(")")?;
        } else if !parser.take(")") {
            loop {
                parser.expect("int")?;
                let ty = if parser.take("*") {
                    Type::Pointer
                } else {
                    Type::Int
                };
                let parameter = parser.identifier()?;
                parser.add_symbol(parameter, ty, 1)?;
                parameters.push((String::from(parameter.text), ty));
                if parser.take(")") {
                    break;
                }
                parser.expect(",")?;
            }
        }
        if !parser.at("{") {
            return Err(parser.error("only function definitions are supported at file scope"));
        }
        parser.block()?;
        parser.emit(Op::Fallthrough, name.range)?;
        total_code += parser.code.len();
        if total_code > MAX_CODE {
            return Err(Diagnostic::limit(
                name.range,
                "compiled operation limit reached",
            ));
        }
        functions.push(Function {
            name: String::from(name.text),
            parameters,
            code: std::mem::take(&mut parser.code),
            nodes: roots(std::mem::take(&mut parser.nodes)),
        });
        parser.symbols.clear();
    }
    let Some(entry) = functions
        .iter()
        .position(|function| function.name == "main")
    else {
        return Err(parser.error("an int main(void) definition is required"));
    };
    if !functions[entry].parameters.is_empty() {
        return Err(parser.error("main parameters are outside this subset"));
    }
    for CallSite {
        name,
        arguments,
        range,
        caller,
    } in &parser.calls
    {
        let Some(index) = functions.iter().position(|function| function.name == *name) else {
            return Err(Diagnostic::new(
                "unsupported",
                *range,
                "call to an undefined function; library functions are not supported",
            ));
        };
        if index > *caller {
            return Err(Diagnostic::new(
                "unsupported",
                *range,
                "function must be declared before its call in C99; forward declarations are \
                 outside this subset",
            ));
        }
        let function = &functions[index];
        if function.parameters.len() != arguments.len() {
            return Err(Diagnostic::new(
                "syntax",
                *range,
                "argument count does not match the function definition",
            ));
        }
        for ((_, expected), &(actual, null)) in function.parameters.iter().zip(arguments) {
            if !compatible(*expected, actual, null) {
                return Err(Diagnostic::new(
                    "unsupported",
                    *range,
                    "argument type does not match the parameter",
                ));
            }
        }
    }
    Ok(Program {
        source: String::from(source),
        functions,
        entry,
    })
}

pub(super) struct Parser<'a> {
    pub tokens:         Vec<Token<'a>>,
    pub position:       usize,
    pub code:           Vec<Instruction>,
    pub expressions:    Vec<Expr>,
    pub symbols:        Vec<Symbol<'a>>,
    pub scope:          usize,
    pub nesting:        usize,
    pub calls:          Vec<CallSite>,
    pub function_index: usize,
    /// The current function's expression-tree nodes.
    pub nodes:          Vec<ExpressionNode>,
}

impl<'a> Parser<'a> {
    fn block(&mut self) -> Result<(), Diagnostic> {
        let opening = self.current();
        self.expect("{")?;
        self.scope += 1;
        self.emit(Op::Enter, opening.range)?;
        while !self.at("}") {
            if self.at("") {
                return Err(self.error("expected closing brace"));
            }
            if self.at("int") || self.at("char") {
                self.declaration()?;
            } else {
                self.statement()?;
            }
        }
        let closing = self.current();
        self.expect("}")?;
        self.emit(Op::Leave, closing.range)?;
        self.symbols.retain(|symbol| symbol.scope < self.scope);
        self.scope -= 1;
        Ok(())
    }

    fn statement(&mut self) -> Result<(), Diagnostic> {
        self.enter_nesting()?;
        let result = self.statement_inner();
        self.nesting -= 1;
        result
    }

    fn statement_inner(&mut self) -> Result<(), Diagnostic> {
        let token = self.current();
        match token.text {
            | "{" => self.block(),
            | "return" => {
                self.position += 1;
                let expression = self.expression(0)?;
                self.require_type(expression, Type::Int)?;
                let value = self.emit_expression(expression, false)?;
                let range = combined(token.range, self.expressions[expression].range);
                self.expect(";")?;
                let node = self.tree("return", range, vec![value])?;
                self.emit_node(Op::Return, range, node)?;
                Ok(())
            },
            | "if" => {
                self.position += 1;
                let (range, node) = self.condition("if")?;
                let branch = self.code.len();
                self.emit_node(Op::Branch(0, "if"), range, node)?;
                self.control_body()?;
                if self.take("else") {
                    let jump = self.code.len();
                    self.emit(Op::Jump(0), token.range)?;
                    self.patch(branch, self.code.len());
                    self.control_body()?;
                    self.patch(jump, self.code.len());
                } else {
                    self.patch(branch, self.code.len());
                }
                Ok(())
            },
            | "while" => {
                self.position += 1;
                let head = self.code.len();
                let (range, node) = self.condition("while")?;
                let branch = self.code.len();
                self.emit_node(Op::Branch(0, "while"), range, node)?;
                self.control_body()?;
                self.emit(Op::Jump(head), token.range)?;
                self.patch(branch, self.code.len());
                Ok(())
            },
            | "for" => self.for_statement(),
            | ";" => {
                self.position += 1;
                Ok(())
            },
            | "int" | "char" | "else" | "break" | "continue" | "goto" | "do" | "switch" =>
                Err(self.error("unsupported statement in this subset")),
            | _ => {
                self.expression_statement()?;
                self.expect(";")
            },
        }
    }

    // Selection/iteration substatements get their C99 implicit block scope.
    // A compound body is a block of its own that ends at the same point, so
    // its scope stands in for the implicit one.
    fn control_body(&mut self) -> Result<(), Diagnostic> {
        if self.at("{") {
            return self.statement();
        }
        let range = self.current().range;
        self.scope += 1;
        self.emit(Op::Enter, range)?;
        self.statement()?;
        self.emit(Op::Leave, range)?;
        self.symbols.retain(|symbol| symbol.scope < self.scope);
        self.scope -= 1;
        Ok(())
    }

    fn for_statement(&mut self) -> Result<(), Diagnostic> {
        let token = self.current();
        self.position += 1;
        self.expect("(")?;
        self.scope += 1;
        self.emit(Op::Enter, token.range)?;
        if self.at("int") || self.at("char") {
            self.declaration()?;
        } else {
            if !self.at(";") {
                self.expression_statement()?;
            }
            self.expect(";")?;
        }
        let head = self.code.len();
        let branch = if self.at(";") {
            self.emit(Op::Push(1), token.range)?;
            let branch = self.code.len();
            self.emit(Op::Branch(0, "for"), token.range)?;
            branch
        } else {
            let condition = self.expression(0)?;
            self.require_scalar(condition)?;
            let value = self.emit_expression(condition, false)?;
            let range = self.expressions[condition].range;
            let node = self.tree("for", range, vec![value])?;
            let branch = self.code.len();
            self.emit_node(Op::Branch(0, "for"), range, node)?;
            branch
        };
        self.expect(";")?;
        let iteration_start = self.code.len();
        if !self.at(")") {
            self.expression_statement()?;
        }
        self.expect(")")?;
        let iteration = self.code.split_off(iteration_start);
        self.control_body()?;
        self.code.extend(iteration);
        self.emit(Op::Jump(head), token.range)?;
        self.patch(branch, self.code.len());
        self.emit(Op::Leave, token.range)?;
        self.symbols.retain(|symbol| symbol.scope < self.scope);
        self.scope -= 1;
        Ok(())
    }

    /// A parenthesized controlling expression; returns its range and the
    /// statement's tree root.
    fn condition(&mut self, statement: &'static str) -> Result<(SourceRange, usize), Diagnostic> {
        self.expect("(")?;
        let expression = self.expression(0)?;
        self.require_scalar(expression)?;
        let value = self.emit_expression(expression, false)?;
        self.expect(")")?;
        let range = self.expressions[expression].range;
        Ok((range, self.tree(statement, range, vec![value])?))
    }

    fn declaration(&mut self) -> Result<(), Diagnostic> {
        if self.take("char") {
            let name = self.identifier()?;
            if self.at("*") || self.at("[") {
                return Err(self.error("only scalar char objects are supported"));
            }
            return self.scalar_declaration(name, Type::Char);
        }
        self.expect("int")?;
        let mut ty = if self.take("*") {
            Type::Pointer
        } else {
            Type::Int
        };
        let name = self.identifier()?;
        if self.take("[") {
            if ty != Type::Int {
                return Err(self.error("arrays of pointers are outside this subset"));
            }
            let count = self.current();
            let Ok(length) = count.text.parse::<usize>() else {
                return Err(self.error("array size must be a positive decimal literal"));
            };
            if length == 0 || length > super::model::MAX_CELLS {
                return Err(Diagnostic::limit(
                    count.range,
                    "array size must be between 1 and 512",
                ));
            }
            self.position += 1;
            self.expect("]")?;
            ty = Type::Array(length);
        }
        let Type::Array(length) = ty else {
            return self.scalar_declaration(name, ty);
        };
        self.add_symbol(name, ty, self.scope)?;
        self.emit(Op::Declare(String::from(name.text), ty), name.range)?;
        if self.take("=") {
            self.expect("{")?;
            let mut index = 0;
            while !self.at("}") {
                if index >= length {
                    return Err(self.error("too many array initializer elements"));
                }
                let element = self.array_element(name, index)?;
                let expression = self.expression(0)?;
                self.require_type(expression, Type::Int)?;
                if self.expressions[expression].call {
                    return Err(self.error("calls in array initializers are outside this subset"));
                }
                let value = self.emit_expression(expression, false)?;
                index += 1;
                let range = self.expressions[expression].range;
                self.initialize(element, value, range, index == length)?;
                if !self.take(",") {
                    break;
                }
            }
            self.expect("}")?;
            if index == 0 {
                return Err(self.error("empty initializer lists are outside the C99 subset"));
            }
            while index < length {
                let element = self.array_element(name, index)?;
                let zero = self.tree("0", name.range, Vec::new())?;
                self.emit_node(Op::Zero, name.range, zero)?;
                index += 1;
                self.initialize(element, zero, name.range, index == length)?;
            }
        }
        self.expect(";")
    }

    /// `int x`, `int *p` or `char c`, with an optional initializer.
    fn scalar_declaration(&mut self, name: Token<'a>, ty: Type) -> Result<(), Diagnostic> {
        self.add_symbol(name, ty, self.scope)?;
        self.emit(Op::Declare(String::from(name.text), ty), name.range)?;
        if self.take("=") {
            let target = self.tree(name.text, name.range, Vec::new())?;
            self.emit_node(Op::Address(String::from(name.text)), name.range, target)?;
            let expression = self.expression(0)?;
            self.require_type(expression, ty)?;
            let value = self.emit_expression(expression, false)?;
            let range = combined(name.range, self.expressions[expression].range);
            let node = self.tree("=", range, vec![target, value])?;
            let char_value = self.expressions[expression].ty == Type::Char;
            self.emit_node(
                Op::Store {
                    initializer: true,
                    full_expression: true,
                    char_value,
                },
                range,
                node,
            )?;
        }
        self.expect(";")
    }

    /// The lvalue for element `index` of the array being initialized.
    fn array_element(&mut self, name: Token<'_>, index: usize) -> Result<usize, Diagnostic> {
        let index =
            i32::try_from(index).map_err(|_| self.error("array index exceeds int range"))?;
        let node = self.tree(&format!("{}[{index}]", name.text), name.range, Vec::new())?;
        self.emit_node(
            Op::Element(String::from(name.text), index),
            name.range,
            node,
        )?;
        Ok(node)
    }

    /// Store an initializer element. The initializer list is one full
    /// expression, so only its last store ends it (C99 §6.8p4).
    fn initialize(
        &mut self,
        element: usize,
        value: usize,
        range: SourceRange,
        last: bool,
    ) -> Result<(), Diagnostic> {
        let node = self.tree("=", range, vec![element, value])?;
        self.emit_node(
            Op::Store {
                initializer:     true,
                full_expression: last,
                char_value:      false,
            },
            range,
            node,
        )
    }

    fn expression_statement(&mut self) -> Result<(), Diagnostic> {
        let prefix_token = self.current();
        let prefix = if self.take("++") {
            Some(1)
        } else if self.take("--") {
            Some(-1)
        } else {
            None
        };
        let expression = self.expression(0)?;
        let range = self.expressions[expression].range;
        if self.take("=") {
            if prefix.is_some() {
                return Err(self.error("nested increments are outside this subset"));
            }
            let target = self.emit_expression(expression, true)?;
            let value = self.expression(0)?;
            self.require_type(value, self.expressions[expression].ty)?;
            let node = self.emit_expression(value, false)?;
            let range = combined(range, self.expressions[value].range);
            let root = self.tree("=", range, vec![target, node])?;
            let char_value = self.expressions[value].ty == Type::Char;
            self.emit_node(
                Op::Store {
                    initializer: false,
                    full_expression: true,
                    char_value,
                },
                range,
                root,
            )?;
            return Ok(());
        }
        let postfix_token = self.current();
        let postfix = if prefix.is_some() {
            None
        } else if self.take("++") {
            Some(1)
        } else if self.take("--") {
            Some(-1)
        } else {
            None
        };
        if let Some(delta) = prefix.or(postfix) {
            let operand = &self.expressions[expression];
            if prefix.is_none()
                && matches!(operand.kind, ExprKind::Unary("*", _))
                && !operand.parenthesized
            {
                return Err(Diagnostic::new(
                    "unsupported",
                    range,
                    "unparenthesized *p++ and *p-- contain nested side effects; use (*p)++ to \
                     increment the pointed-to int",
                ));
            }
            let target = self.emit_expression(expression, true)?;
            let (range, label) = if prefix.is_some() {
                (combined(prefix_token.range, range), prefix_token.text)
            } else {
                (combined(range, postfix_token.range), postfix_token.text)
            };
            let root = self.tree(label, range, vec![target])?;
            self.emit_node(
                Op::Increment {
                    delta,
                    prefix: prefix.is_some(),
                },
                range,
                root,
            )?;
        } else {
            let _value = self.emit_expression(expression, false)?;
            self.emit(Op::Pop, range)?;
        }
        Ok(())
    }

    pub fn emit(&mut self, op: Op, range: SourceRange) -> Result<(), Diagnostic> {
        self.push_instruction(op, range, None)
    }

    /// Emit an operation that evaluates expression-tree `node`.
    pub fn emit_node(&mut self, op: Op, range: SourceRange, node: usize) -> Result<(), Diagnostic> {
        self.push_instruction(op, range, Some(node))
    }

    fn push_instruction(
        &mut self,
        op: Op,
        range: SourceRange,
        node: Option<usize>,
    ) -> Result<(), Diagnostic> {
        if self.code.len() >= MAX_CODE {
            return Err(Diagnostic::limit(range, "compiled operation limit reached"));
        }
        self.code.push(Instruction { op, range, node });
        Ok(())
    }

    /// Add an expression-tree node over `children`, which must be the most
    /// recently completed subtrees in source order.
    pub fn tree(
        &mut self,
        label: &str,
        range: SourceRange,
        children: Vec<usize>,
    ) -> Result<usize, Diagnostic> {
        if self.nodes.len() >= MAX_CODE {
            return Err(Diagnostic::limit(range, "expression node limit reached"));
        }
        let index = self.nodes.len();
        let first = children
            .first()
            .map_or(index, |&child| self.nodes[child].first);
        self.nodes.push(ExpressionNode {
            label: String::from(label),
            range,
            children,
            first,
            root: index,
        });
        Ok(index)
    }

    fn patch(&mut self, index: usize, target: usize) {
        match &mut self.code[index].op {
            | Op::Branch(value, _) | Op::Jump(value) => *value = target,
            | _ => {},
        }
    }

    pub fn current(&self) -> Token<'a> {
        self.tokens[self.position]
    }

    pub fn at(&self, text: &str) -> bool {
        self.current().text == text
    }

    pub fn take(&mut self, text: &str) -> bool {
        if self.at(text) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    pub fn expect(&mut self, text: &str) -> Result<(), Diagnostic> {
        if self.take(text) {
            Ok(())
        } else {
            Err(Diagnostic::new(
                "unsupported",
                self.current().range,
                &format_message(
                    "expected ",
                    text,
                    "; unsupported or malformed subset syntax",
                ),
            ))
        }
    }

    pub fn error(&self, message: &str) -> Diagnostic {
        Diagnostic::new("unsupported", self.current().range, message)
    }

    pub fn enter_nesting(&mut self) -> Result<(), Diagnostic> {
        if self.nesting >= MAX_DEPTH {
            return Err(Diagnostic::limit(
                self.current().range,
                "parser nesting limit reached",
            ));
        }
        self.nesting += 1;
        Ok(())
    }

    pub fn require_type(&self, expression: usize, expected: Type) -> Result<(), Diagnostic> {
        let expression = &self.expressions[expression];
        if compatible(
            expected,
            expression.ty,
            matches!(expression.kind, ExprKind::Number(0)),
        ) {
            Ok(())
        } else {
            Err(Diagnostic::new(
                "unsupported",
                expression.range,
                "incompatible types in the bounded subset",
            ))
        }
    }

    pub fn require_scalar(&self, expression: usize) -> Result<(), Diagnostic> {
        if matches!(
            self.expressions[expression].ty,
            Type::Int | Type::Char | Type::Pointer
        ) {
            Ok(())
        } else {
            Err(self.error("expected a scalar expression"))
        }
    }
}

impl<'a> Parser<'a> {
    fn identifier(&mut self) -> Result<Token<'a>, Diagnostic> {
        let token = self.tokens[self.position];
        if !is_identifier(token.text) {
            return Err(self.error("expected an identifier; keyword or unsupported type found"));
        }
        self.position += 1;
        Ok(token)
    }

    fn add_symbol(&mut self, token: Token<'a>, ty: Type, scope: usize) -> Result<(), Diagnostic> {
        if self
            .symbols
            .iter()
            .any(|symbol| symbol.name == token.text && symbol.scope == scope)
        {
            return Err(self.error("duplicate local or parameter declaration"));
        }
        self.symbols.push(Symbol {
            name: token.text,
            ty,
            scope,
        });
        Ok(())
    }
}

pub(super) struct Symbol<'a> {
    pub name:  &'a str,
    pub ty:    Type,
    pub scope: usize,
}

pub(super) fn is_identifier(text: &str) -> bool {
    text.as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && !matches!(
            text,
            "int"
                | "void"
                | "return"
                | "if"
                | "else"
                | "while"
                | "for"
                | "break"
                | "continue"
                | "goto"
                | "do"
                | "switch"
                | "case"
                | "default"
                | "sizeof"
                | "char"
                | "short"
                | "long"
                | "float"
                | "double"
                | "signed"
                | "unsigned"
                | "const"
                | "volatile"
                | "restrict"
                | "static"
                | "extern"
                | "auto"
                | "register"
                | "typedef"
                | "struct"
                | "union"
                | "enum"
                | "inline"
                | "_Bool"
                | "_Complex"
                | "_Imaginary"
        )
}

fn compatible(expected: Type, actual: Type, null: bool) -> bool {
    expected == actual
        || matches!(
            (expected, actual),
            (Type::Char, Type::Int) | (Type::Int, Type::Char)
        )
        || (expected == Type::Pointer && actual == Type::Int && null)
}

/// Set each node's full-expression root. A parent follows its children, so a
/// reverse walk meets every parent before its children.
fn roots(mut nodes: Vec<ExpressionNode>) -> Vec<ExpressionNode> {
    for index in (0..nodes.len()).rev() {
        let root = nodes[index].root;
        for child in nodes[index].children.clone() {
            nodes[child].root = root;
        }
    }
    nodes
}

fn format_message(prefix: &str, value: &str, suffix: &str) -> String {
    let mut message = String::from(prefix);
    message.push_str(value);
    message.push_str(suffix);
    message
}

pub(super) struct CallSite {
    pub name:      String,
    pub arguments: Vec<(Type, bool)>,
    pub range:     SourceRange,
    pub caller:    usize,
}
