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
    integers::Integer,
    layout,
    lexer::{
        self,
        Token,
    },
    model::{
        Diagnostic,
        Directive,
        DirectiveKind,
        ExpressionNode,
        Function,
        Instruction,
        Library,
        MAX_CELLS,
        MAX_CODE,
        MAX_DEPTH,
        MAX_MEMBERS,
        MAX_OBJECTS,
        MAX_STRUCTURES,
        Macro,
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
        tokens: lexer::lex(source)?,
        source,
        position: 0,
        code: Vec::new(),
        expressions: Vec::new(),
        symbols: Vec::new(),
        scope: 0,
        nesting: 0,
        calls: Vec::new(),
        function_index: 0,
        function_name: None,
        nodes: Vec::new(),
        structures: Vec::new(),
        complete: Vec::new(),
        statics: Vec::new(),
        signatures: Vec::new(),
        prototypes: Vec::new(),
        directives: Vec::new(),
        macros: Vec::new(),
        declaration_macros: Vec::new(),
        returns: Type::INT,
        updates: false,
    };
    let mut functions: Vec<Function> = Vec::new();
    let mut total_code = 0;
    while !parser.at("") {
        if parser.current().text.starts_with('#') {
            parser.directive()?;
            continue;
        }
        if parser.at("struct") && parser.peek(2) == "{" {
            parser.structure_definition()?;
            continue;
        }
        let storage = parser.current();
        parser.declaration_macros.clear();
        let is_static = parser.take("static");
        let base = parser.base_type()?;
        let pointer = parser.pointer()?;
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
            | (Base::Integer(integer, false), false) => Type::Integer(integer),
            | (Base::Struct(id), false) => parser.complete_structure(id, name.range)?,
            | _ =>
                return Err(Diagnostic::new(
                    "unsupported",
                    name.range,
                    "functions return an integer type or a structure in this subset",
                )),
        };
        if parser.symbols.iter().any(|symbol| symbol.name == name.text) {
            return Err(Diagnostic::new(
                "unsupported",
                name.range,
                "a function and a file-scope object share this name",
            ));
        }
        parser.library_conflict(name)?;
        let header = parser.function_header(name)?;
        if parser.take(";") {
            parser.prototype(name, returns, &header)?;
            parser.symbols.retain(|symbol| symbol.scope == 0);
            continue;
        }
        if !parser.at("{") {
            return Err(parser
                .error("only function definitions and prototypes are supported at file scope"));
        }
        if functions.iter().any(|function| function.name == name.text) {
            return Err(Diagnostic::new(
                "unsupported",
                name.range,
                "duplicate function definition",
            ));
        }
        let mut parameters = Vec::new();
        for (parameter, ty) in header {
            let Some(parameter) = parameter else {
                return Err(Diagnostic::new(
                    "unsupported",
                    name.range,
                    "a function definition names each of its parameters",
                ));
            };
            parameters.push((String::from(parameter.text), ty));
        }
        parser.agrees_with_prototypes(name, returns, &parameters)?;
        parser.function_index = functions.len();
        parser.function_name = Some(String::from(name.text));
        parser.signatures.push((String::from(name.text), returns));
        parser.returns = returns;
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
        parser.function_name = None;
        parser.symbols.retain(|symbol| symbol.scope == 0);
    }
    let Some(entry) = functions
        .iter()
        .position(|function| function.name == "main")
    else {
        return Err(parser.error("an int main(void) definition is required"));
    };
    if !functions[entry].parameters.is_empty() || functions[entry].returns != Type::INT {
        return Err(parser.error("main must be int main(void) in this subset"));
    }
    for CallSite {
        name,
        arguments,
        range,
        declared,
    } in &parser.calls
    {
        let Some(index) = functions.iter().position(|function| function.name == *name) else {
            let message = if *declared {
                format_message("`", name, "` is declared but never defined")
            } else if Library::named(name).is_some() {
                format_message(
                    "`",
                    name,
                    "` is a library function declared in <stdio.h>; write #include <stdio.h> \
                     before its first use",
                )
            } else {
                String::from(
                    "call to an undefined function; the only library functions are printf, puts \
                     and putchar, from <stdio.h>",
                )
            };
            return Err(Diagnostic::new("unsupported", *range, &message));
        };
        if !*declared {
            return Err(Diagnostic::new(
                "unsupported",
                *range,
                "in C99 a function must be declared before its call: define it earlier, or \
                 declare it first with a prototype such as `int f(int);` (C99 §6.5.1p2)",
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
        directives: parser.directives,
        macros: parser.macros,
    })
}

pub(super) struct Parser<'a> {
    pub tokens:             Vec<Token<'a>>,
    pub source:             &'a str,
    pub position:           usize,
    pub code:               Vec<Instruction>,
    pub expressions:        Vec<Expr>,
    pub symbols:            Vec<Symbol<'a>>,
    pub scope:              usize,
    pub nesting:            usize,
    pub calls:              Vec<CallSite>,
    pub function_index:     usize,
    /// The function whose body is being compiled.
    pub function_name:      Option<String>,
    /// The current function's expression-tree nodes.
    pub nodes:              Vec<ExpressionNode>,
    /// Structure types by index; an incomplete one (declared by `struct T *`
    /// or still being defined) has no members yet.
    pub structures:         Vec<Structure>,
    pub complete:           Vec<bool>,
    /// Objects with static storage duration, in declaration order.
    pub statics:            Vec<Static>,
    /// Each function defined so far, or being defined, and its return type.
    pub signatures:         Vec<(String, Type)>,
    /// Each function declared by a prototype so far.
    pub prototypes:         Vec<Prototype>,
    /// Each `#include <stdio.h>` and object-like `#define` so far.
    pub directives:         Vec<Directive>,
    /// The object-like macros defined so far.
    pub macros:             Vec<Macro>,
    /// The macros whose replacements the declaration being parsed uses
    /// outside expressions: in an array size or a static initializer.
    pub declaration_macros: Vec<usize>,
    /// The return type of the function being compiled.
    pub returns:            Type,
    /// Whether the expression being parsed is a condition, which may
    /// contain one `++` or `--` (see `check_update`).
    pub updates:            bool,
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
            if self.current().text.starts_with('#') {
                return Err(self.error(
                    "a directive inside a function is outside this subset; the machine runs \
                     #include <stdio.h> at file scope",
                ));
            }
            if self.at_declaration() {
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
                let from = self.expressions[expression].ty;
                self.emit_node(Op::Return(from), range, node)?;
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
            | "int" | "char" | "short" | "long" | "signed" | "unsigned" | "struct" | "static"
            | "const" | "else" | "break" | "continue" | "goto" | "do" | "switch" =>
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
        if self.at_type() || self.at("const") {
            self.declaration()?;
        } else {
            if !self.at(";") {
                self.expression_statement()?;
            }
            self.expect(";")?;
        }
        let head = self.code.len();
        let branch = if self.at(";") {
            self.emit(Op::Push(1, Integer::Int), token.range)?;
            let branch = self.code.len();
            self.emit(Op::Branch(0, "for"), token.range)?;
            branch
        } else {
            let condition = self.controlling_expression()?;
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
        let expression = self.controlling_expression()?;
        let value = self.emit_expression(expression, false)?;
        self.expect(")")?;
        let range = self.expressions[expression].range;
        Ok((range, self.tree(statement, range, vec![value])?))
    }

    /// The controlling expression of an `if`, `while` or `for`: a scalar,
    /// which may update one variable with `++` or `--` (`i-- > 0`).
    fn controlling_expression(&mut self) -> Result<usize, Diagnostic> {
        self.updates = true;
        let expression = self.expression(0);
        self.updates = false;
        let expression = expression?;
        self.require_scalar(expression)?;
        self.check_update(expression)?;
        Ok(expression)
    }

    /// A block-scope declaration: one declarator, with an optional
    /// initializer. A `static` one names an object that already exists.
    fn declaration(&mut self) -> Result<(), Diagnostic> {
        self.declaration_macros.clear();
        let is_static = self.take("static");
        let base = self.base_type()?;
        let pointer = self.pointer()?;
        let name = self.identifier()?;
        let array = self.declarator_array()?;
        let ty = self.declared_type(base, pointer, array, name.range)?;
        let constant = base.is_const() && !pointer;
        if is_static {
            return self.static_local(name, ty, constant);
        }
        match ty {
            | Type::Array(..) | Type::Struct(_) => self.aggregate_declaration(name, ty, constant),
            | _ => self.scalar_declaration(name, ty, constant),
        }
    }

    /// A declarator's optional `*`. A `const` after it would make the
    /// pointer itself `const`, which this subset leaves out.
    pub fn pointer(&mut self) -> Result<bool, Diagnostic> {
        if !self.take("*") {
            return Ok(false);
        }
        if self.at("const") {
            return Err(self.error(
                "a const pointer (`* const`) is outside this subset; `const int *p` and `const \
                 char *p`, pointers to const, are in it",
            ));
        }
        Ok(true)
    }

    /// Emit the creation of the automatic object `name`, noting a macro
    /// in its array size.
    fn declare(&mut self, name: Token<'a>, ty: Type, constant: bool) -> Result<(), Diagnostic> {
        self.add_symbol(name, ty, self.scope, None, constant)?;
        self.emit(
            Op::Declare {
                name: String::from(name.text),
                ty,
                constant,
            },
            name.range,
        )?;
        self.mark_macro(self.declaration_macros.first().copied());
        Ok(())
    }

    /// Whether a block-scope declaration starts here.
    fn at_declaration(&self) -> bool {
        self.at_type()
            || ["struct", "static", "const"]
                .iter()
                .any(|keyword| self.at(keyword))
    }

    /// Whether an integer type's specifiers start here: a keyword such as
    /// `unsigned`, or `size_t` once `<stdio.h>` has declared it.
    pub fn at_type(&self) -> bool {
        is_integer_specifier(self.current().text) || (self.at("size_t") && self.includes_stdio())
    }

    /// A declarator's optional `[N]`, or `[]` when a string literal
    /// initializer follows, whose length it takes: the literal's
    /// characters and its null (C99 §6.7.8p22).
    fn declarator_array(&mut self) -> Result<Option<usize>, Diagnostic> {
        if !(self.at("[") && self.peek(1) == "]") {
            return self.array_suffix();
        }
        let open = self.current();
        self.position += 2;
        let literal = self.tokens.get(self.position + 1).copied();
        match literal {
            | Some(literal) if self.at("=") && literal.text.starts_with('"') => {
                let joined = self.joined_literal(self.position + 1)?;
                let length = joined.bytes.len() + 1;
                if length > MAX_CELLS {
                    return Err(Diagnostic::limit(
                        joined.range,
                        "array size must be between 1 and 512",
                    ));
                }
                Ok(Some(length))
            },
            | _ => Err(Diagnostic::new(
                "unsupported",
                open.range,
                "an array of unknown size needs a string literal initializer in this subset; give \
                 its size",
            )),
        }
    }

    /// `int x`, `int *p`, `char c` or `struct point *p`, with an optional
    /// initializer.
    fn scalar_declaration(
        &mut self,
        name: Token<'a>,
        ty: Type,
        constant: bool,
    ) -> Result<(), Diagnostic> {
        self.declare(name, ty, constant)?;
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
        let from = self.expressions[expression].ty;
        self.emit_node(
            Op::Store {
                initializer: true,
                full_expression: true,
                from,
                structure: structure_of(ty),
            },
            range,
            node,
        )
    }

    /// An automatic array or structure, with an optional initializer: a
    /// brace-enclosed list, or for a structure also a whole structure
    /// (C99 §6.7.8p13).
    fn aggregate_declaration(
        &mut self,
        name: Token<'a>,
        ty: Type,
        constant: bool,
    ) -> Result<(), Diagnostic> {
        self.declare(name, ty, constant)?;
        if self.take("=") {
            if let Type::Array(Integer::Char, length) = ty
                && self.current().text.starts_with('"')
            {
                let element = self.tree(name.text, name.range, Vec::new())?;
                self.emit_node(
                    Op::Address(Place::Local(String::from(name.text))),
                    name.range,
                    element,
                )?;
                self.string_initializer(element, name.range, length, true)?;
                return self.expect(";");
            }
            if !self.at("{") {
                if !matches!(ty, Type::Struct(_)) {
                    return Err(self.error(
                        "an array is initialized by a brace-enclosed list, or a char array by a \
                         string literal",
                    ));
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
                let from = ty.leaf_type(&self.structures, leaf).unwrap_or(Type::INT);
                self.initialize(element, zero, name.range, from, None)?;
            }
            // The initializer list is one full expression: its last store
            // ends it (C99 §6.8p4).
            if let Some(
                Op::Store {
                    full_expression, ..
                }
                | Op::InitializeString {
                    full_expression, ..
                },
            ) = self
                .code
                .iter_mut()
                .rev()
                .find(|instruction| {
                    matches!(
                        instruction.op,
                        Op::Store { .. } | Op::InitializeString { .. }
                    )
                })
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
                | Type::Array(element, length) => {
                    if self.at("[") {
                        return Err(self.error("array designators are outside this subset"));
                    }
                    if index >= length {
                        return Err(self.error("too many array initializer elements"));
                    }
                    (Type::Integer(element), offset + index)
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
                | Type::Array(Integer::Char, length) if self.current().text.starts_with('"') => {
                    let label = self.label(name.text, shape, at, None);
                    let element = self.element(name, &label, at, None)?;
                    self.string_initializer(element, name.range, length, false)?;
                    mentioned[leaf..leaf + length].fill(true);
                },
                | Type::Array(..) | Type::Struct(_) if self.at("{") => {
                    // A later initializer of the same subobject replaces it
                    // whole, so a scalar the new list leaves out is zero
                    // again, not what an earlier list stored (C99 §6.7.8p19,
                    // p21).
                    mentioned[leaf..leaf + member_type.leaves(&self.structures)].fill(false);
                    self.initializer_list(name, shape, member_type, leaf, mentioned)?;
                },
                | Type::Array(..) =>
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
                    let from = self.expressions[expression].ty;
                    self.initialize(element, value, range, from, structure_of(member_type))?;
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

    /// `= "..."` for the char array of `length` elements designated by
    /// tree node `element`: the literal's characters, then its null if
    /// there is room, initialize successive elements, and the rest are
    /// zero. The literal names no object of its own here: only its
    /// characters are used (C99 §6.7.8p14, p21).
    fn string_initializer(
        &mut self,
        element: usize,
        target: SourceRange,
        length: usize,
        full_expression: bool,
    ) -> Result<(), Diagnostic> {
        let literal = self.take_literal()?;
        if literal.bytes.len() > length {
            return Err(Diagnostic::new(
                "unsupported",
                literal.range,
                &format!(
                    "the string literal has {} characters, more than the {length} elements of the \
                     array it initializes (C99 §6.7.8p2)",
                    literal.bytes.len()
                ),
            ));
        }
        let value = self.tree(&literal.spelling, literal.range, Vec::new())?;
        let range = combined(target, literal.range);
        let root = self.tree("=", range, vec![element, value])?;
        self.emit_node(
            Op::InitializeString {
                bytes: literal.bytes,
                length,
                full_expression,
                parts: literal.parts,
            },
            range,
            root,
        )?;
        self.mark_macro(literal.macro_use);
        Ok(())
    }

    /// The string literal whose first token is token `at`, joined with
    /// the literals right after it: translation phase 6 concatenates
    /// adjacent string literals into one, after phase 5 has replaced their
    /// escape sequences, so `"\x4" "1"` is two characters, not `"A"`.
    ///
    /// C99: §5.1.1.2 paragraph 1 item 6, p. 10; PDF p. 22.
    /// C99: §6.4.5 paragraph 4, p. 62; PDF p. 74.
    pub fn joined_literal(&self, at: usize) -> Result<Joined, Diagnostic> {
        let first = self.tokens[at];
        let mut joined = Joined {
            bytes:     Vec::new(),
            range:     first.range,
            spelling:  String::new(),
            parts:     0,
            macro_use: None,
        };
        while let Some(&part) = self
            .tokens
            .get(at + joined.parts)
            .filter(|token| token.text.starts_with('"'))
        {
            joined
                .bytes
                .extend(lexer::literal_bytes(self.source, part)?);
            if !joined.spelling.is_empty() {
                joined.spelling.push(' ');
            }
            joined.spelling.push_str(part.text);
            joined.range = combined(first.range, part.range);
            joined.macro_use = joined.macro_use.or(part.macro_use);
            joined.parts += 1;
            if joined.bytes.len() >= MAX_CELLS {
                return Err(Diagnostic::limit(
                    joined.range,
                    "a string literal may hold at most 511 characters here",
                ));
            }
        }
        Ok(joined)
    }

    /// The string literal at the current token, joined with any adjacent
    /// ones, moving past them all.
    pub fn take_literal(&mut self) -> Result<Joined, Diagnostic> {
        let joined = self.joined_literal(self.position)?;
        self.position += joined.parts;
        Ok(joined)
    }

    /// Note on the operation just emitted that it evaluates the
    /// replacement of macro `index`.
    pub fn mark_macro(&mut self, index: Option<usize>) {
        if let (Some(index), Some(last)) = (index, self.code.last_mut()) {
            last.macro_use = Some(index);
        }
    }

    /// Store an initializer element. Only the list's last store ends the
    /// full expression; the declaration marks it.
    fn initialize(
        &mut self,
        element: usize,
        value: usize,
        range: SourceRange,
        from: Type,
        structure: Option<usize>,
    ) -> Result<(), Diagnostic> {
        let node = self.tree("=", range, vec![element, value])?;
        self.emit_node(
            Op::Store {
                initializer: true,
                full_expression: false,
                from,
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
        let compound = compound_operator(self.current().text);
        let modifies = self.at("=")
            || compound.is_some()
            || prefix.is_some()
            || self.at("++")
            || self.at("--");
        if modifies && self.expressions[expression].readonly {
            return Err(self.readonly_error(
                expression,
                prefix.is_some() || compound.is_none() && !self.at("="),
            ));
        }
        if let Some(operator) = compound {
            if prefix.is_some() {
                return Err(self.error("nested increments are outside this subset"));
            }
            return self.compound_assignment(expression, operator);
        }
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
            let from = self.expressions[value].ty;
            self.emit_node(
                Op::Store {
                    initializer: false,
                    full_expression: true,
                    from,
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
            if !matches!(operand.ty, Type::Integer(_) | Type::Pointer(..)) {
                return Err(Diagnostic::new(
                    "unsupported",
                    range,
                    "++ and -- update an integer or a pointer to one in this subset",
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
                    nested: false,
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

    /// The constraint an assignment, compound assignment, `++` or `--`
    /// breaks when its operand `expression` is `const`: an object defined
    /// `const`, or one reached through a pointer to `const`. `increment`
    /// for `++` and `--`.
    ///
    /// C99: §6.5.16 paragraph 2, p. 91; PDF p. 103.
    /// C99: §6.5.2.4 paragraph 1, p. 75; PDF p. 87.
    /// C99: §6.3.2.1 paragraph 1, p. 46; PDF p. 58 (modifiable lvalue).
    pub fn readonly_error(&self, expression: usize, increment: bool) -> Diagnostic {
        let operand = &self.expressions[expression];
        let what = match &operand.kind {
            | ExprKind::Name(name, ..) => format!(
                "`{name}` is defined const, so it is not a modifiable lvalue: the program \
                 promised never to change it after its initialization"
            ),
            | _ => String::from(
                "this object is reached through a pointer to const, so it is not a modifiable \
                 lvalue here: the program promised not to change it through this pointer",
            ),
        };
        let (operation, clause) = if increment {
            (
                "++ and -- need a modifiable lvalue (C99 §6.5.2.4p1, §6.5.3.1p1)",
                &super::clauses::INCREMENT_CONSTRAINT,
            )
        } else {
            (
                "an assignment's left operand must be a modifiable lvalue (C99 §6.5.16p2)",
                &super::clauses::ASSIGNMENT_CONSTRAINT,
            )
        };
        Diagnostic::constraint(operand.range, &format!("{what}; {operation}"), clause)
    }

    /// `E1 op= E2` as a statement, after `E1`: one step designates `E1`
    /// (it is evaluated once), `E2` is evaluated, and one step reads `E1`,
    /// applies `op` and stores the result, as `E1 = E1 op (E2)` would.
    ///
    /// C99: §6.5.16.2 paragraphs 1-3, p. 93; PDF p. 105.
    fn compound_assignment(
        &mut self,
        expression: usize,
        operator: &'static str,
    ) -> Result<(), Diagnostic> {
        let token = self.current();
        self.position += 1;
        let value = self.expression(0)?;
        let left = self.expressions[expression].ty;
        let right = self.expressions[value].ty;
        let range = combined(
            self.expressions[expression].range,
            self.expressions[value].range,
        );
        let shift = matches!(operator, "<<" | ">>");
        let common = match (left, right) {
            | (Type::Integer(left), Type::Integer(_)) if shift => Some(left.promoted()),
            | (Type::Integer(left), Type::Integer(right)) =>
                Some(left.promoted().common(right.promoted()).0),
            | _ => None,
        };
        match (left, operator) {
            | _ if common.is_some() => {},
            | (Type::Pointer(..), "+" | "-") if matches!(right, Type::Integer(_)) => {},
            | (Type::StructPointer(_), "+" | "-") if matches!(right, Type::Integer(_)) =>
                return Err(Diagnostic::new(
                    "unsupported",
                    range,
                    "arithmetic on structure pointers needs an array of structures, which is \
                     outside this subset",
                )),
            | (Type::Struct(_), _) => {
                return Err(Diagnostic::constraint(
                    range,
                    &format!(
                        "`{}` needs an arithmetic left operand, or a pointer for += and -=; a \
                         structure is neither (C99 §6.5.16.2p1, p2)",
                        token.text
                    ),
                    &super::clauses::COMPOUND_CONSTRAINT,
                ));
            },
            | _ => {
                return Err(Diagnostic::constraint(
                    range,
                    &format!(
                        "`{}` takes an arithmetic left operand and an arithmetic right one \
                         (integer ones for <<= and >>=), or for += and -= a pointer on the left \
                         and an integer on the right (C99 §6.5.16.2p1, p2)",
                        token.text
                    ),
                    &super::clauses::COMPOUND_CONSTRAINT,
                ));
            },
        }
        if self.expressions[value].call
            && !matches!(
                self.expressions[value].kind,
                ExprKind::Call(..) | ExprKind::Library(..)
            )
        {
            return Err(self.error(
                "calls must be whole expressions; operand sequencing is outside this subset",
            ));
        }
        let target = self.emit_expression(expression, true)?;
        let operand = self.emit_expression(value, false)?;
        if let Type::Integer(from) = right
            && from.promoted() != from
        {
            let value_range = self.expressions[value].range;
            self.emit_node(Op::Promote(from), value_range, operand)?;
        }
        let root = self.tree(token.text, range, vec![target, operand])?;
        self.emit_node(Op::Compound(operator, common), range, root)
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
        self.code.push(Instruction {
            op,
            range,
            node,
            macro_use: None,
        });
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
            matches!(expression.kind, ExprKind::Number(0, ..)),
        ) {
            Ok(())
        } else if let Some(name) = match (expected, expression.ty) {
            | (Type::Pointer(element, false), Type::Pointer(other, true)) if element == other =>
                Some(element.spelling()),
            | _ => None,
        } {
            Err(Diagnostic::constraint(
                expression.range,
                &format!(
                    "a const {name} * does not convert to a {name} *: that would let a store \
                     change what the program promised not to change (C99 §6.5.16.1p1)"
                ),
                &super::clauses::POINTER_ASSIGNMENT,
            ))
        } else {
            Err(Diagnostic::new(
                "unsupported",
                expression.range,
                "incompatible types in the bounded subset",
            ))
        }
    }

    pub fn require_scalar(&self, expression: usize) -> Result<(), Diagnostic> {
        let ty = self.expressions[expression].ty;
        if matches!(ty, Type::Integer(_)) || ty.is_pointer() {
            Ok(())
        } else {
            Err(self.error("expected a scalar expression"))
        }
    }
}

impl<'a> Parser<'a> {
    /// A preprocessing directive at file scope. The machine runs
    /// `#include <stdio.h>` itself, as translation phase 4 would: it
    /// declares `printf`, `puts` and `putchar` for the rest of the file.
    /// It runs an object-like `#define` too (see `define`). Every other
    /// directive is outside the subset.
    ///
    /// C99: §6.10.2 paragraph 2, p. 149; PDF p. 161.
    /// C99: §7.1.2 paragraph 1, p. 165; PDF p. 177.
    fn directive(&mut self) -> Result<(), Diagnostic> {
        let token = self.current();
        let words: String = token.text[1..]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if words == "define" || words.starts_with("define ") {
            return self.define(token);
        }
        let header = words
            .strip_prefix("include")
            .map(str::trim_start)
            .filter(|header| header.starts_with('<') || header.starts_with('"'));
        match header {
            | Some("<stdio.h>") => {},
            | Some(other) =>
                return Err(Diagnostic::new(
                    "unsupported",
                    token.range,
                    &format_message(
                        "the machine provides only <stdio.h>, not ",
                        other,
                        "; the compiler pipeline view runs the real preprocessor",
                    ),
                )),
            | None =>
                return Err(Diagnostic::new(
                    "unsupported",
                    token.range,
                    "the machine runs only #include <stdio.h> and object-like #define lines \
                     itself; other directives are executed by the preprocessor, which the \
                     compiler pipeline view shows",
                )),
        }
        for name in ["printf", "puts", "putchar"] {
            let defined = self.signatures.iter().any(|(function, _)| function == name)
                || self
                    .prototypes
                    .iter()
                    .any(|prototype| prototype.name == name)
                || self.symbols.iter().any(|symbol| symbol.name == name);
            if defined {
                return Err(Diagnostic::new(
                    "unsupported",
                    token.range,
                    &format_message(
                        "<stdio.h> declares `",
                        name,
                        "`, which this program already declares as something else",
                    ),
                ));
            }
        }
        self.directives.push(Directive {
            range: token.range,
            kind:  DirectiveKind::Include,
        });
        self.position += 1;
        Ok(())
    }

    /// `#define NAME replacement`, an object-like macro whose replacement
    /// list is one integer constant, character constant or string literal.
    /// Translation phase 4 replaces each later use of the name with the
    /// replacement (C99 §6.10.3p9); the machine does it here, before the
    /// program is compiled, so every later token stands for what the
    /// preprocessor leaves. A function-like macro, and any other
    /// replacement, is the preprocessor's own.
    ///
    /// C99: §6.10.3 paragraph 9, p. 152; PDF p. 164.
    /// C99: §6.10.3 paragraph 2, p. 151; PDF p. 163 (redefinition).
    fn define(&mut self, token: Token<'a>) -> Result<(), Diagnostic> {
        let unsupported = |message: &str| Diagnostic::new("unsupported", token.range, message);
        let text = token.text;
        // `#`, optional white space, `define`, then the name.
        let after_hash = text[1..].trim_start();
        let rest = &after_hash["define".len()..];
        if !rest.starts_with([' ', '\t']) {
            return Err(unsupported("#define needs a macro name"));
        }
        let rest = rest.trim_start();
        let name_length = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            .count();
        let name = &rest[..name_length];
        if !is_identifier(name) {
            return Err(unsupported(
                "#define needs a macro name, an identifier that is not a keyword, in this subset",
            ));
        }
        let after_name = &rest[name_length..];
        if after_name.starts_with('(') {
            return Err(unsupported(
                "function-like macros are expanded by the preprocessor, which the compiler \
                 pipeline view shows; the machine runs #define only for a name that stands for \
                 one constant or string literal",
            ));
        }
        let replacement = after_name.trim();
        let start = token.range.start
            + (text.len() - after_name.len())
            + after_name.find(replacement).unwrap_or(0);
        let literal = Token {
            text:      &self.source[start..start + replacement.len()],
            range:     lexer::range(self.source, start, start + replacement.len()),
            macro_use: None,
        };
        let kind = self.replacement_kind(literal).ok_or_else(|| {
            unsupported(
                "the machine runs #define only when the replacement is one integer constant, \
                 character constant or string literal, such as `#define N 10`; other macros are \
                 expanded by the preprocessor, which the compiler pipeline view shows",
            )
        })??;
        if let Some(index) = self.macros.iter().position(|defined| defined.name == name) {
            if self.macros[index].replacement != replacement {
                return Err(Diagnostic::constraint(
                    token.range,
                    &format!(
                        "`{name}` is already defined as `{}`; a macro may be defined again only \
                         with the same replacement list (C99 §6.10.3p2)",
                        self.macros[index].replacement
                    ),
                    &super::clauses::MACRO_REDEFINITION,
                ));
            }
            self.directives.push(Directive {
                range: token.range,
                kind:  DirectiveKind::Define { index, again: true },
            });
            self.position += 1;
            return Ok(());
        }
        let index = self.macros.len();
        self.macros.push(Macro {
            name: String::from(name),
            replacement: String::from(replacement),
            kind,
            range: token.range,
        });
        self.directives.push(Directive {
            range: token.range,
            kind:  DirectiveKind::Define {
                index,
                again: false,
            },
        });
        // Each later instance of the name is replaced (C99 §6.10.3p9);
        // directives are lines of their own and keep their text.
        for later in &mut self.tokens[self.position + 1..] {
            if later.text == name {
                later.text = literal.text;
                later.macro_use = Some(index);
            }
        }
        self.position += 1;
        Ok(())
    }

    /// What a macro's replacement `literal` is, when the machine runs it:
    /// `Some(Err)` for a constant or literal that is itself malformed.
    fn replacement_kind(&self, literal: Token<'_>) -> Option<Result<&'static str, Diagnostic>> {
        let text = literal.text;
        let bytes = text.as_bytes();
        let first = *bytes.first()?;
        if first.is_ascii_digit() {
            return Self::integer_constant(literal)
                .is_ok()
                .then_some(Ok("integer constant"));
        }
        if first != b'"' && first != b'\'' {
            return None;
        }
        let end = match lexer::literal_end(self.source, literal.range.start) {
            | Ok(end) => end,
            | Err(diagnostic) => return Some(Err(diagnostic)),
        };
        if end != literal.range.end {
            return None;
        }
        if first == b'\'' {
            return Some(
                self.character_constant(literal)
                    .map(|_| "character constant"),
            );
        }
        Some(match lexer::literal_bytes(self.source, literal) {
            | Ok(bytes) if bytes.len() >= MAX_CELLS => Err(Diagnostic::limit(
                literal.range,
                "a string literal may hold at most 511 characters here",
            )),
            | Ok(_) => Ok("string literal"),
            | Err(diagnostic) => Err(diagnostic),
        })
    }

    /// Whether the program has included `<stdio.h>` so far.
    pub fn includes_stdio(&self) -> bool {
        self.directives
            .iter()
            .any(|directive| directive.kind == DirectiveKind::Include)
    }

    /// The library function `name` names, when `<stdio.h>` has been
    /// included.
    pub fn library(&self, name: &str) -> Option<Library> {
        Library::named(name).filter(|_| self.includes_stdio())
    }

    /// Reject a declaration of `name` at file scope that `<stdio.h>` already
    /// declares.
    fn library_conflict(&self, name: Token<'_>) -> Result<(), Diagnostic> {
        if self.library(name.text).is_some() {
            return Err(Diagnostic::new(
                "unsupported",
                name.range,
                &format_message(
                    "`",
                    name.text,
                    "` is already declared by <stdio.h> as a library function; give yours another \
                     name",
                ),
            ));
        }
        Ok(())
    }

    /// A function declarator's parenthesized parameter list: each
    /// parameter's name (absent in a prototype that omits it) and type.
    /// Named parameters enter the function's scope.
    fn function_header(
        &mut self,
        name: Token<'a>,
    ) -> Result<Vec<(Option<Token<'a>>, Type)>, Diagnostic> {
        self.expect("(")?;
        let mut parameters = Vec::new();
        self.scope = 1;
        if self.take("void") {
            self.expect(")")?;
        } else if self.at(")") {
            if self.peek(1) == ";" {
                return Err(Diagnostic::new(
                    "unsupported",
                    name.range,
                    "empty parentheses declare a function without a prototype; write (void) for \
                     no parameters",
                ));
            }
            self.position += 1;
        } else {
            loop {
                let base = self.base_type()?;
                let pointer = self.pointer()?;
                let parameter = if self.at(",") || self.at(")") {
                    None
                } else {
                    Some(self.identifier()?)
                };
                let range = parameter.map_or(self.current().range, |token| token.range);
                let ty = match (base, pointer) {
                    | (Base::Integer(integer, false), false) => Type::Integer(integer),
                    | (Base::Integer(integer, constant), true) => Type::Pointer(integer, constant),
                    | (Base::Struct(id), false) => self.complete_structure(id, range)?,
                    | (Base::Struct(id), true) => Type::StructPointer(id),
                    | (Base::Integer(_, true), false) =>
                        return Err(Diagnostic::new(
                            "unsupported",
                            range,
                            "const parameters are outside this subset; `const int *` and `const \
                             char *` parameters are in it",
                        )),
                };
                if self.at("[") {
                    return Err(self.error(
                        "array parameters are outside this subset; write the pointer they adjust \
                         to, such as `char *s`",
                    ));
                }
                if let Some(parameter) = parameter {
                    self.add_symbol(parameter, ty, 1, None, false)?;
                }
                parameters.push((parameter, ty));
                if self.take(")") {
                    break;
                }
                self.expect(",")?;
            }
        }
        self.scope = 0;
        Ok(parameters)
    }

    /// `T name(parameters);`: a prototype that lets calls before the
    /// definition check their arguments (C99 §6.9.1p7, §6.7.5.3p15).
    fn prototype(
        &mut self,
        name: Token<'a>,
        returns: Type,
        header: &[(Option<Token<'a>>, Type)],
    ) -> Result<(), Diagnostic> {
        let parameters: Vec<Type> = header.iter().map(|&(_, ty)| ty).collect();
        let named: Vec<(String, Type)> = parameters.iter().map(|&ty| (String::new(), ty)).collect();
        self.agrees_with_prototypes(name, returns, &named)?;
        if let Some((_, defined)) = self
            .signatures
            .iter()
            .find(|(function, _)| function == name.text)
            && *defined != returns
        {
            return Err(Diagnostic::new(
                "unsupported",
                name.range,
                "this prototype's return type differs from the function's definition",
            ));
        }
        self.prototypes.push(Prototype {
            name: String::from(name.text),
            returns,
            parameters,
        });
        Ok(())
    }

    /// Two declarations of one function must give it compatible types:
    /// the same return type and the same parameter types (C99 §6.7.5.3p15).
    fn agrees_with_prototypes(
        &self,
        name: Token<'_>,
        returns: Type,
        parameters: &[(String, Type)],
    ) -> Result<(), Diagnostic> {
        for prototype in self
            .prototypes
            .iter()
            .filter(|prototype| prototype.name == name.text)
        {
            let same = prototype.returns == returns
                && prototype.parameters.len() == parameters.len()
                && prototype
                    .parameters
                    .iter()
                    .zip(parameters)
                    .all(|(declared, (_, ty))| declared == ty);
            if !same {
                return Err(Diagnostic::new(
                    "unsupported",
                    name.range,
                    "this declaration's type differs from the function's prototype; both must \
                     give the same return and parameter types (C99 §6.7.5.3p15)",
                ));
            }
        }
        Ok(())
    }

    fn identifier(&mut self) -> Result<Token<'a>, Diagnostic> {
        let token = self.tokens[self.position];
        if !is_identifier(token.text) {
            return Err(self.error("expected an identifier; keyword or unsupported type found"));
        }
        self.position += 1;
        Ok(token)
    }

    /// The text of the token `ahead` places past the current one.
    pub fn peek(&self, ahead: usize) -> &'a str {
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
        constant: bool,
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
            constant,
        });
        Ok(())
    }

    /// A type specifier: an integer type, optionally `const`, or `struct
    /// tag`. A tag not seen before declares an incomplete structure type,
    /// which only a pointer may use until its definition.
    ///
    /// C99: §6.7.3 paragraph 3, p. 108; PDF p. 120 (`const` qualifies
    /// the type; the object is then not modifiable).
    pub fn base_type(&mut self) -> Result<Base, Diagnostic> {
        let constant = self.take("const");
        if let Some(integer) = self.integer_type()? {
            if self.at("const") {
                return Err(self.error(
                    "write const first, as in `const int`; other placements of const are outside \
                     this subset",
                ));
            }
            return Ok(Base::Integer(integer, constant));
        }
        if constant {
            return Err(self.error(
                "const is supported before an integer type in this subset (`const int limit = \
                 3;`, `const char *s`)",
            ));
        }
        if !self.take("struct") {
            return Err(self.error(
                "expected an integer type (char, short, int, long, long long, signed or unsigned) \
                 or struct; other types (float, double, void, ...) are outside this subset",
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

    /// The integer type the type specifiers here name, if they start here:
    /// `char`, `signed char`, `unsigned char`, `short`, `int`, `long`, `long
    /// long`, each with `signed` or `unsigned`, the words in any order and
    /// `int` optional after `short` and `long`; or `size_t`, which
    /// `<stdio.h>` defines as `unsigned long` on x86-64.
    ///
    /// C99: §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112.
    /// C99: §7.19.1 paragraph 2, p. 262; PDF p. 274 (`size_t`).
    pub fn integer_type(&mut self) -> Result<Option<Integer>, Diagnostic> {
        if self.at("size_t") && self.includes_stdio() {
            self.position += 1;
            return Ok(Some(Integer::UnsignedLong));
        }
        let first = self.current();
        let mut counts = [0_u8; 6];
        let words = ["signed", "unsigned", "char", "short", "int", "long"];
        while let Some(word) = words.iter().position(|word| self.at(word)) {
            counts[word] += 1;
            self.position += 1;
        }
        if counts == [0; 6] {
            return Ok(None);
        }
        let range = combined(first.range, self.tokens[self.position - 1].range);
        if matches!(
            self.current().text,
            "float" | "double" | "_Bool" | "_Complex"
        ) {
            return Err(self.error(
                "floating and boolean types are outside this subset, which models the integer \
                 types",
            ));
        }
        let [signed, unsigned, char, short, int, long] = counts;
        let valid = signed + unsigned <= 1
            && char <= 1
            && short <= 1
            && int <= 1
            && long <= 2
            && (char == 0 || short + int + long == 0)
            && (short == 0 || long == 0);
        if !valid {
            return Err(Diagnostic::constraint(
                range,
                "these type specifiers name no type: C allows each integer type only as one of \
                 its listed spellings, such as `unsigned long int` (C99 §6.7.2p2)",
                &super::clauses::TYPE_SPECIFIERS,
            ));
        }
        let unsigned = unsigned == 1;
        Ok(Some(match (char, short, long) {
            | (1, ..) if unsigned => Integer::UnsignedChar,
            | (1, ..) if signed == 1 => Integer::SignedChar,
            | (1, ..) => Integer::Char,
            | (_, 1, _) if unsigned => Integer::UnsignedShort,
            | (_, 1, _) => Integer::Short,
            | (.., 1) if unsigned => Integer::UnsignedLong,
            | (.., 1) => Integer::Long,
            | (.., 2) if unsigned => Integer::UnsignedLongLong,
            | (.., 2) => Integer::LongLong,
            | _ if unsigned => Integer::UnsignedInt,
            | _ => Integer::Int,
        }))
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
        self.declaration_macros.extend(count.macro_use);
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

    /// The type a declarator gives its base type: an integer type `T`,
    /// `T *`, `const T *`, `T[N]`, `struct S` or `struct S *`.
    pub fn declared_type(
        &self,
        base: Base,
        pointer: bool,
        array: Option<usize>,
        range: SourceRange,
    ) -> Result<Type, Diagnostic> {
        let unsupported = |message| Err(Diagnostic::new("unsupported", range, message));
        match (base, pointer, array) {
            | (Base::Integer(integer, _), false, None) => Ok(Type::Integer(integer)),
            | (Base::Integer(integer, constant), true, None) =>
                Ok(Type::Pointer(integer, constant)),
            | (Base::Integer(integer, _), false, Some(length)) => Ok(Type::Array(integer, length)),
            | (_, true, Some(_)) => unsupported("arrays of pointers are outside this subset"),
            | (Base::Struct(id), false, None) => self.complete_structure(id, range),
            | (Base::Struct(id), true, None) => Ok(Type::StructPointer(id)),
            | (Base::Struct(_), false, Some(_)) =>
                unsupported("arrays of structures are outside this subset"),
        }
    }

    /// `struct tag { members };` at file scope. Members are integers,
    /// pointers to them, arrays of them, complete structures
    /// and pointers to structures (including this one); several may share
    /// a line (`int x, y;`).
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
                let pointer = self.pointer()?;
                let name = self.identifier()?;
                let array = self.array_suffix()?;
                if base == Base::Struct(id) && !pointer {
                    return Err(Diagnostic::new(
                        "unsupported",
                        name.range,
                        "a structure cannot contain itself; a pointer to it is fine",
                    ));
                }
                if base.is_const() && !pointer {
                    return Err(Diagnostic::new(
                        "unsupported",
                        name.range,
                        "const members are outside this subset",
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
        let array = self.declarator_array()?;
        let ty = self.declared_type(base, pointer, array, name.range)?;
        self.library_conflict(name)?;
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
        let constant = base.is_const() && !pointer;
        let index = self.static_object(name, ty, None, constant)?;
        self.add_symbol(name, ty, 0, Some(index), constant)?;
        self.expect(";")
    }

    /// `static T name [= constant];` in a block: the object exists before
    /// `main` and keeps its value between calls; the declaration names it
    /// and does nothing when execution reaches it.
    fn static_local(
        &mut self,
        name: Token<'a>,
        ty: Type,
        constant: bool,
    ) -> Result<(), Diagnostic> {
        let function = self.function_name.clone();
        let index = self.static_object(name, ty, function, constant)?;
        self.add_symbol(name, ty, self.scope, Some(index), constant)?;
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
        constant: bool,
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
            literal: false,
            constant,
            parts: 0,
            macros: std::mem::take(&mut self.declaration_macros),
            range,
        });
        Ok(self.statics.len() - 1)
    }

    /// The array of static storage duration the string literal `token`
    /// denotes: its characters and a terminating null (C99 §6.4.5p5).
    /// Each literal in the source is its own array; it exists before
    /// `main` like every static object.
    pub fn string_literal(&mut self, literal: Joined) -> Result<usize, Diagnostic> {
        if self.statics.len() >= MAX_OBJECTS {
            return Err(Diagnostic::limit(literal.range, "object limit reached"));
        }
        let mut bytes = literal.bytes;
        bytes.push(0);
        let ty = Type::Array(Integer::Char, bytes.len());
        self.statics.push(Static {
            name: literal.spelling,
            ty,
            function: None,
            value: Value::Array(bytes.into_iter().map(char_value).collect()),
            explicit: true,
            literal: true,
            constant: false,
            parts: literal.parts,
            macros: literal.macro_use.into_iter().collect(),
            range: literal.range,
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
            | Type::Array(Integer::Char, length) if self.current().text.starts_with('"') => {
                let literal = self.joined_literal(self.position)?;
                if literal.bytes.len() > length {
                    return Err(self.error(
                        "the string literal has more characters than the array has elements (C99 \
                         §6.7.8p2)",
                    ));
                }
                for (at, &byte) in literal.bytes.iter().enumerate() {
                    leaves[offset + at] = char_value(byte);
                }
                self.declaration_macros.extend(literal.macro_use);
                self.position += literal.parts;
                Ok(())
            },
            | Type::Array(..) | Type::Struct(_) => {
                self.expect("{")?;
                if self.at("}") {
                    return Err(self.error("empty initializer lists are outside the C99 subset"));
                }
                let mut index = 0;
                while !self.at("}") {
                    let (member, leaf) = match ty {
                        | Type::Array(element, length) => {
                            if index >= length {
                                return Err(self.error("too many array initializer elements"));
                            }
                            (Type::Integer(element), offset + index)
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
                    let string = matches!(member, Type::Array(Integer::Char, _))
                        && self.current().text.starts_with('"');
                    if matches!(member, Type::Array(..) | Type::Struct(_))
                        && !self.at("{")
                        && !string
                    {
                        return Err(self.error(
                            "brace the initializer of each nested array or structure; brace \
                             elision is outside this subset",
                        ));
                    }
                    // A later initializer of the same subobject replaces it
                    // whole (C99 §6.7.8p19, p21).
                    let mut zeros = Vec::new();
                    layout::flatten(&member.zero(&self.structures), &mut zeros);
                    leaves[leaf..leaf + zeros.len()].clone_from_slice(&zeros);
                    self.constant_initializer(member, leaf, leaves)?;
                    index += 1;
                    if !self.take(",") {
                        break;
                    }
                }
                self.expect("}")
            },
            | Type::Integer(integer) if self.current().text.starts_with('\'') => {
                let token = self.current();
                self.declaration_macros.extend(token.macro_use);
                let value = self.character_constant(token)?;
                self.position += 1;
                leaves[offset] = Value::Integer(integer.convert(value));
                Ok(())
            },
            | Type::Integer(integer) => {
                let negative = self.take("-");
                let token = self.current();
                self.declaration_macros.extend(token.macro_use);
                let not_constant = |parser: &Self| {
                    parser.error(
                        "a static object's initializer must be a constant; this subset takes a \
                         decimal integer constant, optionally negated",
                    )
                };
                if !token
                    .text
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_digit)
                {
                    return Err(not_constant(self));
                }
                let (value, _) = Self::integer_constant(token)?;
                self.position += 1;
                if !(self.at(",") || self.at(";") || self.at("}")) {
                    return Err(self.error(
                        "a static object's initializer must be a constant; this subset takes a \
                         decimal integer constant, optionally negated, and no arithmetic on it",
                    ));
                }
                // The initializer converts as by assignment: modulo 2^N
                // into an unsigned type, wrapped into a signed one (C99
                // §6.7.8p11, §6.3.1.3).
                let value = if negative { -value } else { value };
                leaves[offset] = Value::Integer(integer.convert(value));
                Ok(())
            },
            | Type::Pointer(Integer::Char, _) if self.current().text.starts_with('"') => {
                // The literal's array exists before main, so its address
                // is a constant (C99 §6.6p9).
                let literal = self.take_literal()?;
                self.declaration_macros.extend(literal.macro_use);
                let index = self.string_literal(literal)?;
                let length = match self.statics[index].ty {
                    | Type::Array(_, length) => i32::try_from(length).unwrap_or(i32::MAX),
                    | _ => 1,
                };
                leaves[offset] = Value::pointer(index + 1, 0, 0, length);
                Ok(())
            },
            | Type::Pointer(..) | Type::StructPointer(_) => {
                if !self.take("0") {
                    return Err(self.error(
                        "a static pointer's initializer must be a constant; this subset takes 0, \
                         the null pointer constant, or a string literal for a char pointer",
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
#[expect(
    variant_size_differences,
    reason = "A short-lived parser value; a structure's index is the larger variant."
)]
pub(super) enum Base {
    /// An integer type, `const`-qualified when set.
    Integer(Integer, bool),
    Struct(usize),
}

impl Base {
    /// Whether the specifier is `const`-qualified.
    pub(super) const fn is_const(self) -> bool {
        matches!(self, Self::Integer(_, true))
    }
}

/// The char value a byte holds: plain char is signed with 8 bits on x86-64
/// (C99 §6.2.5p15).
pub(super) fn char_value(byte: u8) -> Value {
    Value::Integer(i128::from(i8::from_ne_bytes([byte])))
}

/// Whether `word` is one of the keywords that specify an integer type.
pub(super) fn is_integer_specifier(word: &str) -> bool {
    matches!(
        word,
        "signed" | "unsigned" | "char" | "short" | "int" | "long"
    )
}

/// The structure type `ty` is, if it is one.
pub(super) const fn structure_of(ty: Type) -> Option<usize> {
    match ty {
        | Type::Struct(id) => Some(id),
        | _ => None,
    }
}

pub(super) struct Symbol<'a> {
    pub name:     &'a str,
    pub ty:       Type,
    pub scope:    usize,
    /// The static object it names: a file-scope object or a block-scope
    /// `static` one.
    pub place:    Option<usize>,
    /// Whether the object is defined `const`.
    pub constant: bool,
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

/// Whether a value of type `actual` may be stored in an object of type
/// `expected` (as assignment converts it), `null` when it is the constant 0:
/// any integer converts to any integer type, and a `T *` to a `const T *`,
/// never the reverse (C99 §6.5.16.1p1).
pub(super) fn compatible(expected: Type, actual: Type, null: bool) -> bool {
    expected == actual
        || matches!((expected, actual), (Type::Integer(_), Type::Integer(_)))
        || matches!(
            (expected, actual),
            (Type::Pointer(element, true), Type::Pointer(other, false)) if element == other
        )
        || (expected.is_pointer() && matches!(actual, Type::Integer(_)) && null)
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
    /// Whether a definition or prototype declared the function before the
    /// call.
    pub declared:  bool,
}

/// A function declared by a prototype: `int f(int);`.
pub(super) struct Prototype {
    pub name:       String,
    pub returns:    Type,
    pub parameters: Vec<Type>,
}

/// One string literal after translation phase 6: one or more adjacent
/// literal tokens joined.
pub(super) struct Joined {
    /// The characters, without the null phase 7 appends.
    pub bytes:     Vec<u8>,
    /// From the first token to the last.
    pub range:     SourceRange,
    /// The literals as written (after phase 4), separated by one space.
    pub spelling:  String,
    /// How many literal tokens were joined.
    pub parts:     usize,
    /// The first macro among them whose replacement is a literal.
    pub macro_use: Option<usize>,
}

/// The binary operator of a compound assignment operator: `+` for `+=`.
fn compound_operator(text: &str) -> Option<&'static str> {
    match text {
        | "+=" => Some("+"),
        | "-=" => Some("-"),
        | "*=" => Some("*"),
        | "/=" => Some("/"),
        | "%=" => Some("%"),
        | "<<=" => Some("<<"),
        | ">>=" => Some(">>"),
        | _ => None,
    }
}
