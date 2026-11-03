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
    layout,
    lexer::{
        self,
        Token,
    },
    model::{
        Diagnostic,
        ExpressionNode,
        Function,
        Instruction,
        MAX_CELLS,
        MAX_CODE,
        MAX_DEPTH,
        MAX_MEMBERS,
        MAX_OBJECTS,
        MAX_STRUCTURES,
        Op,
        Place,
        Program,
        SourceRange,
        Static,
        Structure,
        Type,
        Value,
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
        structures:     Vec::new(),
        complete:       Vec::new(),
        statics:        Vec::new(),
        signatures:     Vec::new(),
        returns:        Type::Int,
    };
    let mut functions: Vec<Function> = Vec::new();
    let mut total_code = 0;
    while !parser.at("") {
        if parser.at("struct") && parser.peek(2) == "{" {
            parser.structure_definition()?;
            continue;
        }
        let storage = parser.current();
        let is_static = parser.take("static");
        let base = parser.base_type()?;
        let pointer = parser.take("*");
        let name = parser.identifier()?;
        if !parser.at("(") {
            parser.file_scope_object(base, pointer, name)?;
            continue;
        }
        if is_static {
            return Err(Diagnostic::new(
                "unsupported",
                storage.range,
                "static functions are outside this subset; drop the static",
            ));
        }
        let returns = match (base, pointer) {
            | (Base::Int, false) => Type::Int,
            | (Base::Struct(id), false) => parser.complete_structure(id, name.range)?,
            | _ =>
                return Err(Diagnostic::new(
                    "unsupported",
                    name.range,
                    "functions return int or a structure in this subset",
                )),
        };
        parser.function_index = functions.len();
        if functions.iter().any(|function| function.name == name.text) {
            return Err(parser.error("duplicate function definition"));
        }
        if parser.symbols.iter().any(|symbol| symbol.name == name.text) {
            return Err(Diagnostic::new(
                "unsupported",
                name.range,
                "a function and a file-scope object share this name",
            ));
        }
        parser.signatures.push((String::from(name.text), returns));
        parser.returns = returns;
        parser.expect("(")?;
        let mut parameters = Vec::new();
        parser.scope = 1;
        if parser.take("void") {
            parser.expect(")")?;
        } else if !parser.take(")") {
            loop {
                let base = parser.base_type()?;
                let pointer = parser.take("*");
                let parameter = parser.identifier()?;
                let ty = match (base, pointer) {
                    | (Base::Int, false) => Type::Int,
                    | (Base::Int, true) => Type::Pointer,
                    | (Base::Struct(id), false) =>
                        parser.complete_structure(id, parameter.range)?,
                    | (Base::Struct(id), true) => Type::StructPointer(id),
                    | (Base::Char, _) =>
                        return Err(Diagnostic::new(
                            "unsupported",
                            parameter.range,
                            "parameters are int, int *, a structure or a pointer to one in this \
                             subset",
                        )),
                };
                parser.add_symbol(parameter, ty, 1, None)?;
                parameters.push((String::from(parameter.text), ty));
                if parser.take(")") {
                    break;
                }
                parser.expect(",")?;
            }
        }
        parser.scope = 0;
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
            returns,
            parameters,
            code: std::mem::take(&mut parser.code),
            nodes: roots(std::mem::take(&mut parser.nodes)),
        });
        parser.symbols.retain(|symbol| symbol.scope == 0);
    }
    let Some(entry) = functions
        .iter()
        .position(|function| function.name == "main")
    else {
        return Err(parser.error("an int main(void) definition is required"));
    };
    if !functions[entry].parameters.is_empty() || functions[entry].returns != Type::Int {
        return Err(parser.error("main must be int main(void) in this subset"));
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
        structures: parser.structures.into(),
        statics: parser.statics,
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
    /// Structure types by index; an incomplete one (declared by `struct T *`
    /// or still being defined) has no members yet.
    pub structures:     Vec<Structure>,
    pub complete:       Vec<bool>,
    /// Objects with static storage duration, in declaration order.
    pub statics:        Vec<Static>,
    /// Each function defined so far, or being defined, and its return type.
    pub signatures:     Vec<(String, Type)>,
    /// The return type of the function being compiled.
    pub returns:        Type,
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
            if self.at("int") || self.at("char") || self.at("struct") || self.at("static") {
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
                self.require_type(expression, self.returns)?;
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
            | "int" | "char" | "struct" | "static" | "else" | "break" | "continue" | "goto"
            | "do" | "switch" => Err(self.error("unsupported statement in this subset")),
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

    /// A block-scope declaration: one declarator, with an optional
    /// initializer. A `static` one names an object that already exists.
    fn declaration(&mut self) -> Result<(), Diagnostic> {
        let is_static = self.take("static");
        let base = self.base_type()?;
        let pointer = self.take("*");
        let name = self.identifier()?;
        let array = self.array_suffix()?;
        let ty = self.declared_type(base, pointer, array, name.range)?;
        if is_static {
            return self.static_local(name, ty);
        }
        match ty {
            | Type::Array(_) | Type::Struct(_) => self.aggregate_declaration(name, ty),
            | _ => self.scalar_declaration(name, ty),
        }
    }

    /// `int x`, `int *p`, `char c` or `struct point *p`, with an optional
    /// initializer.
    fn scalar_declaration(&mut self, name: Token<'a>, ty: Type) -> Result<(), Diagnostic> {
        self.add_symbol(name, ty, self.scope, None)?;
        self.emit(Op::Declare(String::from(name.text), ty), name.range)?;
        if self.take("=") {
            self.whole_initializer(name, ty)?;
        }
        self.expect(";")
    }

    /// `= expression` for the object `name` just declared: one store that
    /// ends the full expression. A structure copies a whole structure.
    fn whole_initializer(&mut self, name: Token<'a>, ty: Type) -> Result<(), Diagnostic> {
        let target = self.tree(name.text, name.range, Vec::new())?;
        self.emit_node(
            Op::Address(Place::Local(String::from(name.text))),
            name.range,
            target,
        )?;
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
                structure: structure_of(ty),
            },
            range,
            node,
        )
    }

    /// An automatic array or structure, with an optional initializer: a
    /// brace-enclosed list, or for a structure also a whole structure
    /// (C99 §6.7.8p13).
    fn aggregate_declaration(&mut self, name: Token<'a>, ty: Type) -> Result<(), Diagnostic> {
        self.add_symbol(name, ty, self.scope, None)?;
        self.emit(Op::Declare(String::from(name.text), ty), name.range)?;
        if self.take("=") {
            if !self.at("{") {
                if !matches!(ty, Type::Struct(_)) {
                    return Err(self.error("an array is initialized by a brace-enclosed list"));
                }
                self.whole_initializer(name, ty)?;
                return self.expect(";");
            }
            let leaves = ty.leaves(&self.structures);
            let mut mentioned = vec![false; leaves];
            let shape = ty.indeterminate(&self.structures);
            self.initializer_list(name, &shape, ty, 0, &mut mentioned)?;
            // Every scalar the list did not mention is initialized to zero,
            // as a static object would be (C99 §6.7.8p19, p21).
            for (leaf, done) in mentioned.iter().enumerate() {
                if *done {
                    continue;
                }
                let offset = i32::try_from(leaf).map_err(|_| self.error("object too large"))?;
                let label = self.label(name.text, &shape, offset, None);
                let element = self.element(name, &label, offset, None)?;
                let zero = self.tree("0", name.range, Vec::new())?;
                self.emit_node(Op::Zero, name.range, zero)?;
                self.initialize(element, zero, name.range, None)?;
            }
            // The initializer list is one full expression: its last store
            // ends it (C99 §6.8p4).
            if let Some(Op::Store {
                full_expression, ..
            }) = self
                .code
                .iter_mut()
                .rev()
                .find(|instruction| matches!(instruction.op, Op::Store { .. }))
                .map(|instruction| &mut instruction.op)
            {
                *full_expression = true;
            }
        }
        self.expect(";")
    }

    /// A brace-enclosed list initializing the subobject of type `ty` at
    /// leaf `offset` of `name`, marking each scalar it initializes. Without
    /// designators it fills elements or members in order; `.m =` moves to
    /// member `m` (C99 §6.7.8p17). Nested aggregates take braces of their
    /// own: brace elision is outside this subset.
    fn initializer_list(
        &mut self,
        name: Token<'a>,
        shape: &Value,
        ty: Type,
        offset: usize,
        mentioned: &mut [bool],
    ) -> Result<(), Diagnostic> {
        self.enter_nesting()?;
        self.expect("{")?;
        if self.at("}") {
            return Err(self.error("empty initializer lists are outside the C99 subset"));
        }
        let mut index = 0;
        while !self.at("}") {
            let (member_type, leaf) = match ty {
                | Type::Array(length) => {
                    if self.at("[") {
                        return Err(self.error("array designators are outside this subset"));
                    }
                    if index >= length {
                        return Err(self.error("too many array initializer elements"));
                    }
                    (Type::Int, offset + index)
                },
                | Type::Struct(id) => {
                    if self.take(".") {
                        let member = self.identifier()?;
                        index = self.structures[id]
                            .members
                            .iter()
                            .position(|candidate| candidate.name == member.text)
                            .ok_or_else(|| self.member_error(member, id))?;
                        self.expect("=")?;
                    }
                    let Some(member) = self.structures[id].members.get(index) else {
                        return Err(self.error("too many initializers for the structure's members"));
                    };
                    (member.ty, offset + member.first)
                },
                | _ => return Err(self.error("braces initialize only arrays and structures")),
            };
            let at = i32::try_from(leaf).map_err(|_| self.error("object too large"))?;
            match member_type {
                | Type::Array(_) | Type::Struct(_) if self.at("{") =>
                    self.initializer_list(name, shape, member_type, leaf, mentioned)?,
                | Type::Array(_) =>
                    return Err(self.error(
                        "brace the initializer of each nested array; brace elision is outside \
                         this subset",
                    )),
                | _ => {
                    let label = self.label(name.text, shape, at, structure_of(member_type));
                    let element = self.element(name, &label, at, structure_of(member_type))?;
                    let expression = self.expression(0)?;
                    if matches!(member_type, Type::Struct(_))
                        && !matches!(self.expressions[expression].ty, Type::Struct(_))
                    {
                        return Err(self.error(
                            "brace the initializer of each nested structure; brace elision is \
                             outside this subset",
                        ));
                    }
                    self.require_type(expression, member_type)?;
                    if self.expressions[expression].call {
                        return Err(
                            self.error("calls in initializer lists are outside this subset")
                        );
                    }
                    let value = self.emit_expression(expression, false)?;
                    let range = self.expressions[expression].range;
                    self.initialize(element, value, range, structure_of(member_type))?;
                    for done in mentioned
                        .iter_mut()
                        .skip(leaf)
                        .take(member_type.leaves(&self.structures))
                    {
                        *done = true;
                    }
                },
            }
            index += 1;
            if !self.take(",") {
                break;
            }
        }
        self.expect("}")?;
        self.nesting -= 1;
        Ok(())
    }

    /// The lvalue for the subobject `label` at leaf `offset` of the object
    /// `name` being initialized.
    fn element(
        &mut self,
        name: Token<'_>,
        label: &str,
        offset: i32,
        structure: Option<usize>,
    ) -> Result<usize, Diagnostic> {
        let node = self.tree(label, name.range, Vec::new())?;
        self.emit_node(
            Op::Element {
                name: String::from(name.text),
                offset,
                structure,
            },
            name.range,
            node,
        )?;
        Ok(node)
    }

    /// Store an initializer element. Only the list's last store ends the
    /// full expression; the declaration marks it.
    fn initialize(
        &mut self,
        element: usize,
        value: usize,
        range: SourceRange,
        structure: Option<usize>,
    ) -> Result<(), Diagnostic> {
        let node = self.tree("=", range, vec![element, value])?;
        self.emit_node(
            Op::Store {
                initializer: true,
                full_expression: false,
                char_value: false,
                structure,
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
                    structure: structure_of(self.expressions[expression].ty),
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
            if !matches!(operand.ty, Type::Int | Type::Char | Type::Pointer) {
                return Err(Diagnostic::new(
                    "unsupported",
                    range,
                    "++ and -- update an int, a char or an int pointer in this subset",
                ));
            }
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
            Type::Int | Type::Char | Type::Pointer | Type::StructPointer(_)
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

    /// The text of the token `ahead` places past the current one.
    fn peek(&self, ahead: usize) -> &'a str {
        self.tokens
            .get(self.position + ahead)
            .map_or("", |token| token.text)
    }

    fn add_symbol(
        &mut self,
        token: Token<'a>,
        ty: Type,
        scope: usize,
        place: Option<usize>,
    ) -> Result<(), Diagnostic> {
        if self
            .symbols
            .iter()
            .any(|symbol| symbol.name == token.text && symbol.scope == scope)
        {
            return Err(Diagnostic::new(
                "unsupported",
                token.range,
                if scope == 0 {
                    "duplicate file-scope declaration"
                } else {
                    "duplicate local or parameter declaration"
                },
            ));
        }
        self.symbols.push(Symbol {
            name: token.text,
            ty,
            scope,
            place,
        });
        Ok(())
    }

    /// A type specifier: `int`, `char` or `struct tag`. A tag not seen
    /// before declares an incomplete structure type, which only a pointer
    /// may use until its definition.
    fn base_type(&mut self) -> Result<Base, Diagnostic> {
        if self.take("int") {
            return Ok(Base::Int);
        }
        if self.take("char") {
            return Ok(Base::Char);
        }
        if !self.take("struct") {
            return Err(self.error(
                "expected int, char or struct; other types (unsigned, long, float, void, ...) are \
                 outside this subset",
            ));
        }
        let tag = self.identifier()?;
        if self.at("{") {
            return Err(
                self.error("define structures at file scope, on their own (struct tag { ... };)")
            );
        }
        Ok(Base::Struct(self.tag(tag)?))
    }

    /// The structure type tagged `tag`, declared incomplete if new.
    fn tag(&mut self, tag: Token<'_>) -> Result<usize, Diagnostic> {
        if let Some(id) = self
            .structures
            .iter()
            .position(|structure| structure.name == tag.text)
        {
            return Ok(id);
        }
        if self.structures.len() >= MAX_STRUCTURES {
            return Err(Diagnostic::limit(tag.range, "structure type limit reached"));
        }
        self.structures.push(layout::structure(tag.text, &[], &[]));
        self.complete.push(false);
        Ok(self.structures.len() - 1)
    }

    /// `Struct(id)`, provided its definition is complete.
    fn complete_structure(&self, id: usize, range: SourceRange) -> Result<Type, Diagnostic> {
        if self.complete.get(id) == Some(&true) {
            Ok(Type::Struct(id))
        } else {
            Err(Diagnostic::new(
                "unsupported",
                range,
                "this structure type is incomplete here: define it before declaring an object of \
                 it (a pointer to it is fine)",
            ))
        }
    }

    /// An optional `[N]` with a positive decimal length.
    fn array_suffix(&mut self) -> Result<Option<usize>, Diagnostic> {
        if !self.take("[") {
            return Ok(None);
        }
        let count = self.current();
        let Ok(length) = count.text.parse::<usize>() else {
            return Err(self.error("array size must be a positive decimal literal"));
        };
        if length == 0 || length > MAX_CELLS {
            return Err(Diagnostic::limit(
                count.range,
                "array size must be between 1 and 512",
            ));
        }
        self.position += 1;
        self.expect("]")?;
        Ok(Some(length))
    }

    /// The type a declarator gives its base type: `int`, `int *`,
    /// `int[N]`, `char`, `struct T` or `struct T *`.
    fn declared_type(
        &self,
        base: Base,
        pointer: bool,
        array: Option<usize>,
        range: SourceRange,
    ) -> Result<Type, Diagnostic> {
        let unsupported = |message| Err(Diagnostic::new("unsupported", range, message));
        match (base, pointer, array) {
            | (Base::Int, false, None) => Ok(Type::Int),
            | (Base::Int, true, None) => Ok(Type::Pointer),
            | (Base::Int, false, Some(length)) => Ok(Type::Array(length)),
            | (_, true, Some(_)) => unsupported("arrays of pointers are outside this subset"),
            | (Base::Char, false, None) => Ok(Type::Char),
            | (Base::Char, ..) => unsupported("only scalar char objects are supported"),
            | (Base::Struct(id), false, None) => self.complete_structure(id, range),
            | (Base::Struct(id), true, None) => Ok(Type::StructPointer(id)),
            | (Base::Struct(_), false, Some(_)) =>
                unsupported("arrays of structures are outside this subset"),
        }
    }

    /// `struct tag { members };` at file scope. Members are int, char,
    /// int *, int arrays, complete structures and pointers to structures
    /// (including this one); several may share a line (`int x, y;`).
    fn structure_definition(&mut self) -> Result<(), Diagnostic> {
        self.expect("struct")?;
        let tag = self.identifier()?;
        let id = self.tag(tag)?;
        if self.complete[id] {
            return Err(Diagnostic::new(
                "unsupported",
                tag.range,
                "this structure is already defined",
            ));
        }
        self.expect("{")?;
        let mut members: Vec<(String, Type)> = Vec::new();
        while !self.take("}") {
            if self.at("") {
                return Err(self.error("expected closing brace"));
            }
            let base = self.base_type()?;
            loop {
                let pointer = self.take("*");
                let name = self.identifier()?;
                let array = self.array_suffix()?;
                if base == Base::Struct(id) && !pointer {
                    return Err(Diagnostic::new(
                        "unsupported",
                        name.range,
                        "a structure cannot contain itself; a pointer to it is fine",
                    ));
                }
                let ty = self.declared_type(base, pointer, array, name.range)?;
                if members.iter().any(|(existing, _)| existing == name.text) {
                    return Err(Diagnostic::new(
                        "unsupported",
                        name.range,
                        "duplicate member",
                    ));
                }
                if members.len() >= MAX_MEMBERS {
                    return Err(Diagnostic::limit(
                        name.range,
                        "structure member limit reached",
                    ));
                }
                members.push((String::from(name.text), ty));
                if !self.take(",") {
                    break;
                }
            }
            self.expect(";")?;
        }
        if members.is_empty() {
            return Err(Diagnostic::new(
                "unsupported",
                tag.range,
                "a structure needs at least one member",
            ));
        }
        let laid = layout::structure(tag.text, &members, &self.structures);
        if laid.size > MAX_CELLS * 8 {
            return Err(Diagnostic::limit(tag.range, "structure size limit reached"));
        }
        self.structures[id] = laid;
        self.complete[id] = true;
        if !self.at(";") {
            return Err(self.error(
                "end the structure definition with `;` and declare its objects separately",
            ));
        }
        self.expect(";")
    }

    /// The error for a member `name` that structure `id` does not have.
    fn member_error(&self, name: Token<'_>, id: usize) -> Diagnostic {
        Diagnostic::new(
            "unsupported",
            name.range,
            &format!(
                "struct {} has no member named `{}`",
                self.structures[id].name, name.text
            ),
        )
    }

    /// A file-scope object: static storage duration, initialized before
    /// `main` by its constant initializer or to zero.
    fn file_scope_object(
        &mut self,
        base: Base,
        pointer: bool,
        name: Token<'a>,
    ) -> Result<(), Diagnostic> {
        let array = self.array_suffix()?;
        let ty = self.declared_type(base, pointer, array, name.range)?;
        if self
            .signatures
            .iter()
            .any(|(function, _)| function == name.text)
        {
            return Err(Diagnostic::new(
                "unsupported",
                name.range,
                "a function and a file-scope object share this name",
            ));
        }
        let index = self.static_object(name, ty, None)?;
        self.add_symbol(name, ty, 0, Some(index))?;
        self.expect(";")
    }

    /// `static T name [= constant];` in a block: the object exists before
    /// `main` and keeps its value between calls; the declaration names it
    /// and does nothing when execution reaches it.
    fn static_local(&mut self, name: Token<'a>, ty: Type) -> Result<(), Diagnostic> {
        let function = self.signatures.last().map(|(function, _)| function.clone());
        let index = self.static_object(name, ty, function)?;
        self.add_symbol(name, ty, self.scope, Some(index))?;
        self.emit(Op::StaticLocal(index), name.range)?;
        self.expect(";")
    }

    /// Record an object with static storage duration and its initial value,
    /// from an optional constant initializer (C99 §6.7.8p4, p10).
    fn static_object(
        &mut self,
        name: Token<'a>,
        ty: Type,
        function: Option<String>,
    ) -> Result<usize, Diagnostic> {
        if self.statics.len() >= MAX_OBJECTS {
            return Err(Diagnostic::limit(name.range, "object limit reached"));
        }
        let mut leaves = Vec::new();
        layout::flatten(&ty.zero(&self.structures), &mut leaves);
        let explicit = self.take("=");
        let mut range = name.range;
        if explicit {
            self.constant_initializer(ty, 0, &mut leaves)?;
            range = combined(name.range, self.tokens[self.position - 1].range);
        }
        self.statics.push(Static {
            name: String::from(name.text),
            ty,
            function,
            value: ty.shaped(&self.structures, &leaves),
            explicit,
            range,
        });
        Ok(self.statics.len() - 1)
    }

    /// A static object's initializer for the subobject of type `ty` at leaf
    /// `offset`: an integer constant, `0` for a pointer, or a braced list
    /// of them. Its value is known before the program runs.
    fn constant_initializer(
        &mut self,
        ty: Type,
        offset: usize,
        leaves: &mut [Value],
    ) -> Result<(), Diagnostic> {
        self.enter_nesting()?;
        let result = self.constant_initializer_inner(ty, offset, leaves);
        self.nesting -= 1;
        result
    }

    fn constant_initializer_inner(
        &mut self,
        ty: Type,
        offset: usize,
        leaves: &mut [Value],
    ) -> Result<(), Diagnostic> {
        match ty {
            | Type::Array(_) | Type::Struct(_) => {
                self.expect("{")?;
                if self.at("}") {
                    return Err(self.error("empty initializer lists are outside the C99 subset"));
                }
                let mut index = 0;
                while !self.at("}") {
                    let (member, leaf) = match ty {
                        | Type::Array(length) => {
                            if index >= length {
                                return Err(self.error("too many array initializer elements"));
                            }
                            (Type::Int, offset + index)
                        },
                        | Type::Struct(id) => {
                            if self.take(".") {
                                let member = self.identifier()?;
                                index = self.structures[id]
                                    .members
                                    .iter()
                                    .position(|candidate| candidate.name == member.text)
                                    .ok_or_else(|| self.member_error(member, id))?;
                                self.expect("=")?;
                            }
                            let Some(member) = self.structures[id].members.get(index) else {
                                return Err(
                                    self.error("too many initializers for the structure's members")
                                );
                            };
                            (member.ty, offset + member.first)
                        },
                        | _ =>
                            return Err(self.error("braces initialize only arrays and structures")),
                    };
                    if matches!(member, Type::Array(_) | Type::Struct(_)) && !self.at("{") {
                        return Err(self.error(
                            "brace the initializer of each nested array or structure; brace \
                             elision is outside this subset",
                        ));
                    }
                    self.constant_initializer(member, leaf, leaves)?;
                    index += 1;
                    if !self.take(",") {
                        break;
                    }
                }
                self.expect("}")
            },
            | Type::Int | Type::Char => {
                let negative = self.take("-");
                let token = self.current();
                let digits = token
                    .text
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_digit)
                    && (token.text.len() == 1 || !token.text.starts_with('0'));
                let parsed = if negative {
                    format!("-{}", token.text).parse::<i32>()
                } else {
                    token.text.parse::<i32>()
                };
                let Ok(mut integer) = parsed
                    .map_err(|_| ())
                    .and_then(|value| if digits { Ok(value) } else { Err(()) })
                else {
                    return Err(self.error(
                        "a static object's initializer must be a constant; this subset takes a \
                         decimal int constant, optionally negated",
                    ));
                };
                self.position += 1;
                if ty == Type::Char {
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "Keeping the low 8 bits is the modeled conversion."
                    )]
                    let low = integer as i8;
                    integer = i32::from(low);
                }
                leaves[offset] = Value::Integer(integer);
                Ok(())
            },
            | Type::Pointer | Type::StructPointer(_) => {
                if !self.take("0") {
                    return Err(self.error(
                        "a static pointer's initializer must be a constant; this subset takes \
                         only 0, the null pointer constant",
                    ));
                }
                leaves[offset] = Value::null();
                Ok(())
            },
        }
    }

    /// How the source spells the scalar (or the whole structure
    /// `structure`) at leaf `offset` of the object `name` with value `shape`.
    fn label(&self, name: &str, shape: &Value, offset: i32, structure: Option<usize>) -> String {
        let mut label = String::from(name);
        label.push_str(&layout::path(shape, offset, structure, &self.structures));
        label
    }
}

/// A declaration's type specifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Base {
    Int,
    Char,
    Struct(usize),
}

/// The structure type `ty` is, if it is one.
pub(super) const fn structure_of(ty: Type) -> Option<usize> {
    match ty {
        | Type::Struct(id) => Some(id),
        | _ => None,
    }
}

pub(super) struct Symbol<'a> {
    pub name:  &'a str,
    pub ty:    Type,
    pub scope: usize,
    /// The static object it names: a file-scope object or a block-scope
    /// `static` one.
    pub place: Option<usize>,
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
        || (matches!(expected, Type::Pointer | Type::StructPointer(_))
            && actual == Type::Int
            && null)
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
