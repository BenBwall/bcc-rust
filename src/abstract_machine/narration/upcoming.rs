//! What the next operation will do, said before it runs: "read `total`",
//! "add `i` and `1`", "call `bump`". The machine evaluates one operation per
//! step (C99 §5.1.2.3p2), so the page can say where the next step goes. A
//! description names only what is known before the operation runs: the
//! source it evaluates, its operands' spellings and the objects they
//! designate, never the value it will produce or whether it will stop.
//!
//! C99: §5.1.2.3 paragraph 2, p. 13; PDF p. 25 (evaluation and side
//! effects).

use std::fmt::Write;

use super::snippet;
use crate::abstract_machine::{
    clauses::{
        self,
        Clause,
    },
    machine::Machine,
    model::{
        Arithmetic,
        DirectiveKind,
        EventKind,
        Instruction,
        Library,
        Op,
        Operand,
        Place,
        Program,
        Upcoming,
        Value,
    },
};

impl Machine {
    /// The next operation, `next`, of the innermost call, described before
    /// it runs.
    pub(in crate::abstract_machine) fn upcoming(
        &self,
        program: &Program,
        next: &Instruction,
    ) -> Upcoming {
        let source = &program.source;
        let code = || code(&snippet(source, next.range));
        // The spellings of the operands of the node `next` evaluates, in
        // source order: `i` and `1` for `i + 1`.
        let operands = || self.operand_spellings(program, next);
        let (kind, text, clause) = match &next.op {
            | Op::Enter { objects, .. } if objects.is_empty() => (
                EventKind::EnterBlock,
                String::from("enter a block"),
                clauses::COMPOUND_STATEMENT,
            ),
            | Op::Enter { objects, .. } => (
                EventKind::EnterBlock,
                format!(
                    "begin the lifetime of {}",
                    listed(objects.iter().map(|object| object.name.as_str()))
                ),
                clauses::AUTOMATIC_LIFETIME,
            ),
            | Op::Leave => {
                let ended = self.ending();
                if ended.is_empty() {
                    (
                        EventKind::LeaveBlock,
                        String::from("leave the block"),
                        self.block_clause(program, next),
                    )
                } else {
                    (
                        EventKind::LeaveBlock,
                        format!("end the lifetime of {}", listed(ended.into_iter())),
                        clauses::AUTOMATIC_LIFETIME,
                    )
                }
            },
            | Op::Declare { name, .. } => (
                EventKind::Declare,
                format!("reach the declaration of `{name}`"),
                clauses::SCOPE_START,
            ),
            | &Op::StaticLocal(index) => (
                EventKind::StaticLocal,
                format!(
                    "reach the declaration of the static `{}`",
                    program.statics[index].name
                ),
                clauses::STATIC_STORAGE,
            ),
            | Op::Push(..) | Op::PushReal { .. } => {
                let spelled = code();
                let clause = match next.macro_use {
                    | Some(used) if program.macros[used].kind == "character constant" =>
                        clauses::CHARACTER_CONSTANT,
                    | None if spelled.starts_with("`'") => clauses::CHARACTER_CONSTANT,
                    | _ if matches!(next.op, Op::PushReal { .. }) => clauses::FLOATING_CONSTANT,
                    | _ => clauses::INTEGER_CONSTANT,
                };
                let text = next.macro_use.map_or_else(
                    || format!("evaluate the constant {spelled}"),
                    |used| {
                        format!(
                            "evaluate {spelled}, which stands for {}",
                            self::code(&program.macros[used].replacement)
                        )
                    },
                );
                (EventKind::Constant, text, clause)
            },
            | Op::Sizeof(..) => (
                EventKind::Sizeof,
                format!("compute {}", code()),
                clauses::SIZEOF,
            ),
            | Op::Zero => (
                EventKind::Constant,
                String::from("supply an implicit 0"),
                clauses::PARTIAL_INITIALIZER,
            ),
            | Op::Address(_) => (
                EventKind::Designate,
                format!("find the object {}", code()),
                clauses::IDENTIFIER,
            ),
            | Op::Element {
                name,
                offset,
                structure,
            } => {
                let place = self.named(&Place::Local(name.clone())).map_or_else(
                    |_| format!("`{name}`"),
                    |id| self.quoted_location(id, *offset, *structure),
                );
                (
                    EventKind::Designate,
                    format!("select {place} to initialize"),
                    clauses::INITIALIZER_ORDER,
                )
            },
            | Op::Decay(place) => {
                let literal =
                    matches!(place, &Place::Static(index) if program.statics[index].literal);
                let text = if literal {
                    format!(
                        "turn the string literal {} into a pointer to its first char",
                        code()
                    )
                } else {
                    format!(
                        "turn the array {} into a pointer to its first element",
                        code()
                    )
                };
                (EventKind::ArrayDecay, text, clauses::ARRAY_DECAY)
            },
            | Op::Load => (
                EventKind::LvalueConversion,
                format!("read {}", code()),
                clauses::LVALUE_CONVERSION,
            ),
            | Op::LoadStruct(_) => (
                EventKind::LvalueConversion,
                format!("read {}, every member", code()),
                clauses::LVALUE_CONVERSION,
            ),
            | Op::Member { name, arrow, .. } => {
                let base = operands()
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| String::from("the structure"));
                if *arrow {
                    (
                        EventKind::Member,
                        format!("select the member `{name}` of the structure {base} points to"),
                        clauses::MEMBER_POINTER,
                    )
                } else {
                    (
                        EventKind::Member,
                        format!("select the member `{name}` of {base}"),
                        clauses::MEMBER,
                    )
                }
            },
            | &Op::Promote(from) => (
                EventKind::IntegerPromotion,
                format!(
                    "promote {} from `{}` to `{}`",
                    code(),
                    from.spelling(),
                    from.promoted().spelling()
                ),
                clauses::INTEGER_PROMOTIONS,
            ),
            | &Op::Convert { from, to, .. } => (
                EventKind::ArithmeticConversion,
                format!(
                    "convert {} from `{}` to `{}`",
                    code(),
                    from.spelling(),
                    to.spelling()
                ),
                clauses::USUAL_ARITHMETIC,
            ),
            | &Op::Cast { from, to } => {
                let operand = operands().pop().unwrap_or_else(code);
                (
                    EventKind::Cast,
                    format!(
                        "cast {operand} from `{}` to `{}`",
                        from.spelling(&self.structures, &self.signatures),
                        to.spelling(&self.structures, &self.signatures)
                    ),
                    clauses::CAST,
                )
            },
            | &Op::DefaultPromote(from) => {
                // A float becomes a double, a conversion between floating
                // types; a narrower integer takes the integer promotions.
                let (kind, to) = match from {
                    | Arithmetic::Floating(_) => (EventKind::ArithmeticConversion, "double"),
                    | Arithmetic::Integer(integer) =>
                        (EventKind::IntegerPromotion, integer.promoted().spelling()),
                };
                (
                    kind,
                    format!(
                        "promote {} from `{}` to `{to}` to pass it through `...`",
                        code(),
                        from.spelling()
                    ),
                    clauses::DEFAULT_PROMOTIONS,
                )
            },
            | Op::AddressOf => {
                let place = self.top_location(0).unwrap_or_else(code);
                (
                    EventKind::AddressOf,
                    format!("take the address of {place}"),
                    clauses::ADDRESS_OPERATOR,
                )
            },
            | Op::Dereference => {
                let pointer = operands()
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| String::from("the pointer"));
                (
                    EventKind::Dereference,
                    format!("find the object {pointer} points to"),
                    clauses::INDIRECTION,
                )
            },
            | Op::Index => (
                EventKind::Subscript,
                format!("select {}, checking it is inside its array", code()),
                clauses::SUBSCRIPT,
            ),
            | &Op::Unary(operator, _) => {
                let operand = operands().pop().unwrap_or_else(code);
                let (text, clause) = match operator {
                    | "-" => (format!("negate {operand}"), clauses::UNARY_MINUS),
                    | "~" => (format!("flip the bits of {operand}"), clauses::COMPLEMENT),
                    | "!" => (
                        format!("apply `!` to {operand}: 1 if it is 0, else 0"),
                        clauses::LOGICAL_NOT,
                    ),
                    | _ => (format!("apply unary `+` to {operand}"), clauses::UNARY_PLUS),
                };
                (EventKind::Unary, text, clause)
            },
            | &Op::Binary(operator, _) => self.binary_upcoming(operator, &code(), &operands()),
            | Op::ElementAddress => (
                EventKind::PointerArithmetic,
                format!("compute the address {}", code()),
                clauses::ADDRESS_OPERATOR,
            ),
            | &Op::Store {
                initializer,
                structure,
                ..
            } => {
                let place = self
                    .top_location(1)
                    .unwrap_or_else(|| String::from("the object"));
                if initializer {
                    (
                        EventKind::Initialize,
                        format!("initialize {place}"),
                        if structure.is_some() {
                            clauses::STRUCTURE_INITIALIZER
                        } else {
                            clauses::SCALAR_INITIALIZER
                        },
                    )
                } else if structure.is_some() {
                    (
                        EventKind::Assign,
                        format!("copy every member into {place}"),
                        clauses::STRUCTURE_ASSIGNMENT,
                    )
                } else {
                    (
                        EventKind::Assign,
                        format!("store into {place}"),
                        clauses::SIMPLE_ASSIGNMENT,
                    )
                }
            },
            | &Op::Increment { delta, prefix, .. } => {
                let place = self
                    .top_location(0)
                    .unwrap_or_else(|| String::from("the object"));
                let (text, clause) = match (delta > 0, prefix) {
                    | (true, false) => (format!("add 1 to {place}"), clauses::POSTFIX_INCREMENT),
                    | (true, true) => (format!("add 1 to {place}"), clauses::PREFIX_INCREMENT),
                    | (false, false) => (
                        format!("subtract 1 from {place}"),
                        clauses::POSTFIX_DECREMENT,
                    ),
                    | (false, true) => (
                        format!("subtract 1 from {place}"),
                        clauses::PREFIX_DECREMENT,
                    ),
                };
                (EventKind::Increment, text, clause)
            },
            | &Op::Compound(operator, ..) => {
                let place = self
                    .top_location(1)
                    .unwrap_or_else(|| String::from("the object"));
                let value = operands()
                    .pop()
                    .unwrap_or_else(|| String::from("the value"));
                let text = match operator {
                    | "+" => format!("add {value} to {place}"),
                    | "-" => format!("subtract {value} from {place}"),
                    | "*" => format!("multiply {place} by {value}"),
                    | "/" => format!("divide {place} by {value}"),
                    | _ => format!("update {place} with `{operator}=` {value}"),
                };
                (
                    EventKind::CompoundAssign,
                    text,
                    clauses::COMPOUND_ASSIGNMENT,
                )
            },
            | &Op::Test { operator, .. } => {
                let operands = operands();
                let first = operands.first().map_or_else(&code, Clone::clone);
                match operator {
                    | "?:" => (
                        EventKind::Conditional,
                        format!("test {first} to choose an operand of `?:`"),
                        clauses::CONDITIONAL,
                    ),
                    | "&&" => (
                        EventKind::Logical,
                        format!("test {first}, the left operand of `&&`"),
                        clauses::LOGICAL_AND,
                    ),
                    | _ => (
                        EventKind::Logical,
                        format!("test {first}, the left operand of `||`"),
                        clauses::LOGICAL_OR,
                    ),
                }
            },
            | &Op::Yield {
                operator, target, ..
            } => {
                let operands = operands();
                match operator {
                    | "&&" | "||" => (
                        EventKind::Logical,
                        format!(
                            "turn {} into `{operator}`'s 0 or 1",
                            operands.last().map_or_else(&code, Clone::clone)
                        ),
                        if operator == "&&" {
                            clauses::LOGICAL_AND_VALUE
                        } else {
                            clauses::LOGICAL_OR_VALUE
                        },
                    ),
                    | "?:" => {
                        let chosen = if target.is_some() {
                            operands.get(1)
                        } else {
                            operands.last()
                        };
                        (
                            EventKind::Conditional,
                            format!(
                                "give `?:` the value of {}",
                                chosen.map_or_else(&code, Clone::clone)
                            ),
                            clauses::CONDITIONAL,
                        )
                    },
                    | _ => (
                        EventKind::Comma,
                        format!(
                            "give the comma operator the value of {}",
                            operands.last().map_or_else(&code, Clone::clone)
                        ),
                        clauses::COMMA,
                    ),
                }
            },
            | Op::Comma { .. } => (
                EventKind::Comma,
                format!(
                    "finish {}, the left operand of a comma operator",
                    operands().first().map_or_else(&code, Clone::clone)
                ),
                clauses::COMMA,
            ),
            | Op::Pop => (
                EventKind::Discard,
                format!("discard the value of {}", code()),
                clauses::EXPRESSION_STATEMENT,
            ),
            | &Op::Jump(target) => {
                // The jump's own index is the program counter now; running
                // it moves the counter past it first.
                let backward = self
                    .activations
                    .last()
                    .is_some_and(|frame| target <= frame.pc);
                if backward {
                    (
                        EventKind::Jump,
                        String::from("jump back to the loop condition"),
                        clauses::ITERATION,
                    )
                } else {
                    (
                        EventKind::Jump,
                        String::from("skip the else branch"),
                        clauses::IF_STATEMENT,
                    )
                }
            },
            | &Op::Branch(_, statement) => (
                EventKind::Branch,
                format!("test the `{statement}` condition {}", code()),
                if statement == "if" {
                    clauses::IF_STATEMENT
                } else {
                    clauses::ITERATION
                },
            ),
            | Op::Call(name, _) => (
                EventKind::Call,
                format!("call `{name}`"),
                clauses::CALL_ARGUMENTS,
            ),
            | Op::CallPointer { .. } | Op::Function { .. } | Op::FunctionIndirection =>
                function_upcoming(&next.op, &operands()),
            | &Op::Library(library, _) => (
                EventKind::LibraryCall,
                format!("call `{}`, which writes to standard output", library.name()),
                match library {
                    | Library::Printf => clauses::PRINTF,
                    | Library::Puts => clauses::PUTS,
                    | Library::Putchar => clauses::PUTCHAR,
                },
            ),
            | Op::InitializeString { .. } => {
                let place = match self
                    .activations
                    .last()
                    .and_then(|frame| frame.operands.last())
                {
                    | Some(&Operand::Address { target, offset, .. }) =>
                        self.array_name(target, offset),
                    | _ => String::from("the array"),
                };
                (
                    EventKind::Initialize,
                    format!("store the string's characters into {place}"),
                    clauses::STRING_INITIALIZER,
                )
            },
            | Op::Return(_) => {
                let function = self.current_function(program);
                if self.in_initial_main(program) {
                    (
                        EventKind::Return,
                        String::from("return from `main`, which ends the program"),
                        clauses::MAIN_RETURN,
                    )
                } else {
                    // A return from a recursive call of `main` goes back to
                    // its caller like any other: only the initial call ends
                    // the program (C99 §5.1.2.2.3p1).
                    (
                        EventKind::Return,
                        format!("return from `{function}`"),
                        clauses::RETURN,
                    )
                }
            },
            | Op::Fallthrough => {
                let function = self.current_function(program);
                if self.in_initial_main(program) {
                    (
                        EventKind::Return,
                        String::from(
                            "reach the end of `main`, which returns 0 and ends the program",
                        ),
                        clauses::MAIN_RETURN,
                    )
                } else if self.in_main(program) {
                    (
                        EventKind::Return,
                        String::from("reach the end of `main`, which returns 0 to its caller"),
                        clauses::MAIN_RETURN,
                    )
                } else {
                    // Only `main` returns 0 from its closing brace; another
                    // function's caller must not use the value (C99
                    // §6.9.1p12), and the machine stops there.
                    (
                        EventKind::Return,
                        format!("reach the end of `{function}` without a return statement"),
                        clauses::FUNCTION_END,
                    )
                }
            },
        };
        Upcoming { kind, text, clause }
    }

    /// Before `main`: the next directive to run or static object to
    /// initialize, or else `main`'s first operation.
    pub(in crate::abstract_machine) fn upcoming_before_main(&self, program: &Program) -> Upcoming {
        if let Some(directive) = program.directives.get(self.preprocessed) {
            let (text, clause) = match directive.kind {
                | DirectiveKind::Include => (
                    String::from("include the header `<stdio.h>`"),
                    clauses::SOURCE_INCLUSION,
                ),
                | DirectiveKind::Define { index, .. } => (
                    format!("define the macro `{}`", program.macros[index].name),
                    clauses::OBJECT_LIKE_MACRO,
                ),
            };
            return Upcoming {
                kind: EventKind::Directive,
                text,
                clause,
            };
        }
        if let Some(declared) = program.statics.get(self.initialized) {
            let text = if declared.literal {
                format!(
                    "create the string literal `{}`, before `main` runs",
                    declared.name
                )
            } else {
                format!(
                    "give `{}` its initial value, before `main` runs",
                    declared.name
                )
            };
            return Upcoming {
                kind: EventKind::StaticStorage,
                text,
                clause: clauses::STARTUP,
            };
        }
        let entry = &program.functions[program.entry].code[0];
        self.upcoming(program, entry)
    }

    /// A binary operator's description: arithmetic names its operands,
    /// a comparison and pointer arithmetic quote the whole expression.
    fn binary_upcoming(
        &self,
        operator: &str,
        whole: &str,
        operands: &[String],
    ) -> (EventKind, String, Clause) {
        let pointers = self
            .top_values()
            .map(|values| values.map(|value| matches!(value, Value::Pointer { .. })));
        let both = pointers == Some([true, true]);
        if matches!(operator, "<" | "<=" | ">" | ">=" | "==" | "!=") {
            let clause = match operator {
                | "==" | "!=" => clauses::EQUALITY,
                | _ if both => clauses::POINTER_ORDER,
                | _ => clauses::RELATIONAL,
            };
            return (EventKind::Comparison, format!("compare {whole}"), clause);
        }
        if both && operator == "-" {
            return (
                EventKind::PointerArithmetic,
                format!("count the elements between the pointers: {whole}"),
                clauses::POINTER_SUBTRACTION,
            );
        }
        if let Some([left, right]) = pointers
            && left != right
        {
            return (
                EventKind::PointerArithmetic,
                format!("move a pointer: {whole}"),
                clauses::POINTER_ARITHMETIC,
            );
        }
        let (a, b) = match operands {
            | [a, b] => (a.as_str(), b.as_str()),
            | _ =>
                return (
                    EventKind::Arithmetic,
                    format!("compute {whole}"),
                    clauses::ADDITION,
                ),
        };
        let (text, clause) = match operator {
            | "+" => (format!("add {a} and {b}"), clauses::ADDITION),
            | "-" => (format!("subtract {b} from {a}"), clauses::ADDITION),
            | "*" => (format!("multiply {a} by {b}"), clauses::MULTIPLICATION),
            | "/" => (format!("divide {a} by {b}"), clauses::DIVISION),
            | "%" => (
                format!("take the remainder of {a} divided by {b}"),
                clauses::DIVISION,
            ),
            | "<<" => (
                format!("shift the bits of {a} left by {b}"),
                clauses::LEFT_SHIFT,
            ),
            | ">>" => (
                format!("shift the bits of {a} right by {b}"),
                clauses::RIGHT_SHIFT,
            ),
            | "&" => (
                format!("combine the bits of {a} and {b} with `&`"),
                clauses::BITWISE_AND,
            ),
            | "^" => (
                format!("combine the bits of {a} and {b} with `^`"),
                clauses::BITWISE_XOR,
            ),
            | "|" => (
                format!("combine the bits of {a} and {b} with `|`"),
                clauses::BITWISE_OR,
            ),
            | _ => (format!("compute {whole}"), clauses::ADDITION),
        };
        (EventKind::Arithmetic, text, clause)
    }

    /// The two values on top of the innermost call's operand stack, left
    /// then right, when both are values.
    fn top_values(&self) -> Option<[&Value; 2]> {
        let operands = &self.activations.last()?.operands;
        match operands.len().checked_sub(2).map(|at| &operands[at..])? {
            | [Operand::Value(left), Operand::Value(right)] => Some([left, right]),
            | _ => None,
        }
    }

    /// Where the address `depth` places below the top of the innermost
    /// call's operand stack points, in backticks: `` `total` ``,
    /// `` `values[1]` (in main) ``.
    fn top_location(&self, depth: usize) -> Option<String> {
        let operands = &self.activations.last()?.operands;
        let at = operands.len().checked_sub(depth + 1)?;
        match operands[at] {
            | Operand::Address {
                target,
                offset,
                structure,
            } => Some(self.quoted_location(target, offset, structure)),
            | Operand::Value(_) => None,
        }
    }

    /// [`Machine::location_of`] with the place itself always in backticks.
    fn quoted_location(&self, target: usize, offset: i32, structure: Option<usize>) -> String {
        let location = self.location_of(target, offset, structure);
        if location.contains('`') {
            return location;
        }
        match location.split_once(" (in ") {
            | Some((place, rest)) => format!("`{place}` (in {rest}"),
            | None => format!("`{location}`"),
        }
    }

    /// The paragraph that makes the block `leave` ends a block: a compound
    /// statement's, or for a for statement or a substatement without braces,
    /// the loop body's or the selection statement's, as the step that
    /// entered it said. A substatement's block begins and ends at its first
    /// token, so the entry with the same range says whether it is a loop
    /// body.
    ///
    /// C99: §6.8.2 paragraph 2, p. 132; PDF p. 144.
    /// C99: §6.8.4 paragraph 3, p. 133; PDF p. 145.
    /// C99: §6.8.5 paragraph 5, p. 135; PDF p. 147.
    fn block_clause(&self, program: &Program, leave: &Instruction) -> Clause {
        match program
            .source
            .get(leave.range.start..leave.range.end)
            .unwrap_or("")
        {
            | "}" => return clauses::COMPOUND_STATEMENT,
            | "for" => return clauses::LOOP_BODY,
            | _ => {},
        }
        let iteration = self.activations.last().is_some_and(|frame| {
            program.functions[frame.function]
                .code
                .iter()
                .any(|instruction| {
                    instruction.range == leave.range
                        && matches!(
                            instruction.op,
                            Op::Enter {
                                iteration: true,
                                ..
                            }
                        )
                })
        });
        if iteration {
            clauses::LOOP_BODY
        } else {
            clauses::SELECTION_BLOCK
        }
    }

    /// The objects leaving the innermost block ends, by name.
    fn ending(&self) -> Vec<&str> {
        let Some(frame) = self.activations.last() else {
            return Vec::new();
        };
        self.state
            .objects
            .iter()
            .filter(|object| {
                object.frame_id == frame.id
                    && object.lifetime == "alive"
                    && self.metadata[object.id - 1].scope == frame.scope
            })
            .map(|object| object.name.as_str())
            .collect()
    }

    /// The spellings of the operands of the node `instruction` evaluates,
    /// each in backticks.
    fn operand_spellings(&self, program: &Program, instruction: &Instruction) -> Vec<String> {
        let Some(frame) = self.activations.last() else {
            return Vec::new();
        };
        let nodes = &program.functions[frame.function].nodes;
        instruction
            .node
            .and_then(|node| nodes.get(node))
            .map_or_else(Vec::new, |node| {
                node.children
                    .iter()
                    .filter_map(|&child| nodes.get(child))
                    .map(|child| code(&snippet(&program.source, child.range)))
                    .collect()
            })
    }

    fn current_function<'a>(&self, program: &'a Program) -> &'a str {
        self.activations.last().map_or("main", |frame| {
            program.functions[frame.function].name.as_str()
        })
    }

    fn in_main(&self, program: &Program) -> bool {
        self.activations
            .last()
            .is_some_and(|frame| frame.function == program.entry)
    }

    /// Whether the running call is the initial call of `main`, whose return
    /// ends the program (C99 §5.1.2.2.3p1), rather than a recursive one.
    fn in_initial_main(&self, program: &Program) -> bool {
        self.activations.len() == 1 && self.in_main(program)
    }
}

/// `text` as code, in backticks, on one line: a line break and the
/// indentation after it become one space. Spaces within a line stay, as a
/// string literal needs them.
fn code(text: &str) -> String {
    let mut output = String::from("`");
    let mut lines = text.lines();
    if let Some(first) = lines.next() {
        output.push_str(first.trim_end());
    }
    for line in lines {
        output.push(' ');
        output.push_str(line.trim());
    }
    output.push('`');
    output
}

/// Names in backticks, joined as a sentence lists them: `` `a`, `b` and
/// `c` ``.
fn listed<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<&str> = names.collect();
    let mut text = String::new();
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            text.push_str(if index + 1 == names.len() {
                " and "
            } else {
                ", "
            });
        }
        let _ = write!(text, "`{name}`");
    }
    text
}

/// What an operation on a pointer to a function will do: convert a
/// function's name, follow `*fp`, or call through a pointer, whose
/// operands' spellings are `operands` (the pointer's first).
fn function_upcoming(op: &Op, operands: &[String]) -> (EventKind, String, Clause) {
    let pointer = operands
        .first()
        .cloned()
        .unwrap_or_else(|| String::from("the pointer"));
    match op {
        | Op::Function {
            name,
            address: false,
        } => (
            EventKind::FunctionDesignator,
            format!("convert `{name}` to a pointer to the function"),
            clauses::FUNCTION_DESIGNATOR,
        ),
        | Op::Function { name, .. } => (
            EventKind::AddressOf,
            format!("take the address of the function `{name}`"),
            clauses::ADDRESS_OPERATOR,
        ),
        | Op::FunctionIndirection => (
            EventKind::Dereference,
            format!("follow {pointer} to the function it points to"),
            clauses::INDIRECTION,
        ),
        | _ => {
            // `(*fp)` designates the function; `fp` points to it.
            let verb =
                if super::functions::unparenthesized(&pointer.replace('`', "")).starts_with('*') {
                    "designates"
                } else {
                    "points to"
                };
            (
                EventKind::Call,
                format!("call the function {pointer} {verb}"),
                clauses::FUNCTION_CALL,
            )
        },
    }
}
