//! The three output functions `<stdio.h>` declares that the machine runs
//! itself: `printf`, `puts` and `putchar`. Each call is one step that reads
//! its arguments, writes to standard output (`State::output`) and returns.
//! Reading a string follows its pointer element by element to the null
//! character, with the machine's ordinary lifetime, bounds and
//! indeterminate checks; a format is read the same way, then interpreted
//! one conversion specification at a time.
//!
//! The subset of `printf` is the conversions `%d %i %u %x %c %s %%`, with
//! the `-` and `0` flags and a field width. Other valid specifications
//! (precision, `*`, length modifiers, other conversions and flags) stop
//! with an explicit diagnostic; invalid ones, a missing argument and an
//! argument of the wrong type are undefined behavior.
//!
//! C99: §7.19.6.1 paragraphs 2, 4, 6, 8 and 9, pp. 274-280; PDF pp.
//! 286-292 (fprintf).
//! C99: §7.19.6.3 paragraphs 2-3, p. 290; PDF p. 302 (printf).
//! C99: §7.19.7.3 paragraphs 2-3, p. 297; PDF p. 309 (fputc).
//! C99: §7.19.7.9 paragraphs 2-3, p. 299; PDF p. 311 (putchar).
//! C99: §7.19.7.10 paragraphs 2-3, p. 299; PDF p. 311 (puts).
//! C99: §7.1.4 paragraph 1, pp. 166-167; PDF pp. 178-179 (invalid
//! arguments).
//! C99: §7.1.1 paragraph 1, p. 164; PDF p. 176 (strings).

use std::fmt::Write;

use super::{
    clauses::{
        self,
        Clause,
    },
    machine::Machine,
    model::{
        Diagnostic,
        Event,
        EventKind,
        Instruction,
        Library,
        MAX_OUTPUT,
        Operand,
        Program,
        Type,
        Undefined,
        Value,
    },
    narration::{
        self,
        Topic,
    },
};

/// What one piece of a format did, for the narration.
enum Piece {
    /// Ordinary characters, copied as they are.
    Copied(Vec<u8>),
    /// A conversion specification, the argument it took (1-based, after
    /// the format), what that argument was, and the text it wrote.
    Converted {
        specification: String,
        argument:      Option<usize>,
        /// The argument as the call spells it, or empty.
        source:        String,
        took:          String,
        wrote:         Vec<u8>,
    },
}

/// A parsed conversion specification.
struct Specification {
    left:       bool,
    zero:       bool,
    width:      usize,
    conversion: u8,
    text:       String,
}

impl Machine {
    /// Run the library call `instruction` evaluates, with its arguments on
    /// the operand stack, passed as `types`.
    pub(super) fn library_call(
        &mut self,
        program: &Program,
        instruction: &Instruction,
        library: Library,
        types: &[Type],
    ) -> Result<Event, Diagnostic> {
        let mut arguments = Vec::new();
        for _ in types {
            arguments.push(self.pop_value()?);
        }
        arguments.reverse();
        let texts = self.argument_texts(program, instruction);
        let text = |index: usize| texts.get(index).map_or("", String::as_str);
        let range = instruction.range;
        match library {
            | Library::Putchar => {
                let Some(&Value::Integer(character)) = arguments.first() else {
                    return Err(self.error("putchar needs an int"));
                };
                // fputc converts its int to unsigned char (C99 §7.19.7.3p2).
                let byte = character.to_le_bytes()[0];
                self.write(&[byte])?;
                let returned = i32::from(byte);
                self.push(Operand::Value(Value::Integer(returned)))?;
                Ok(Self::putchar_event(range, text(0), character, byte))
            },
            | Library::Puts => {
                let pointer = arguments.first().cloned().unwrap_or(Value::Indeterminate);
                let mut string = self.read_string(&pointer, "puts", text(0))?;
                string.push(b'\n');
                self.write(&string)?;
                // C promises only a nonnegative value; glibc returns the
                // number of characters written.
                let returned = i32::try_from(string.len()).unwrap_or(i32::MAX);
                self.push(Operand::Value(Value::Integer(returned)))?;
                Ok(self.puts_event(range, text(0), &pointer, &string, returned))
            },
            | Library::Printf => {
                let pointer = arguments.first().cloned().unwrap_or(Value::Indeterminate);
                let format = self.read_string(&pointer, "printf", text(0))?;
                let (pieces, written) = self.format(&format, &arguments, types, &texts)?;
                self.write(&written)?;
                let returned = i32::try_from(written.len()).unwrap_or(i32::MAX);
                self.push(Operand::Value(Value::Integer(returned)))?;
                Ok(Self::printf_event(
                    range, &format, &pointer, &arguments, &pieces, &written,
                ))
            },
        }
    }

    /// The source text of each argument of the call `instruction`
    /// evaluates.
    fn argument_texts(&self, program: &Program, instruction: &Instruction) -> Vec<String> {
        let Some(activation) = self.activations.last() else {
            return Vec::new();
        };
        let function = &program.functions[activation.function];
        let Some(node) = instruction.node.and_then(|node| function.nodes.get(node)) else {
            return Vec::new();
        };
        node.children
            .iter()
            .filter_map(|&child| function.nodes.get(child))
            .map(|child| {
                program
                    .source
                    .get(child.range.start..child.range.end)
                    .map_or_else(String::new, String::from)
            })
            .collect()
    }

    /// Append `bytes` to standard output.
    fn write(&mut self, bytes: &[u8]) -> Result<(), Diagnostic> {
        if self.state.output.len() + bytes.len() > MAX_OUTPUT {
            return Err(Diagnostic::limit(
                self.state.source_range,
                "the output limit of 4096 characters was reached",
            ));
        }
        self.state.output.extend_from_slice(bytes);
        self.state.written = bytes.len();
        Ok(())
    }

    /// The characters of the string `pointer` points to, read one element
    /// at a time up to its null character, which is not included. `function`
    /// reads it as its argument `text`.
    fn read_string(
        &self,
        pointer: &Value,
        function: &str,
        text: &str,
    ) -> Result<Vec<u8>, Diagnostic> {
        let &Value::Pointer {
            target: Some(target),
            offset,
            first,
            length,
            ..
        } = pointer
        else {
            return Err(Diagnostic::ub(
                self.state.source_range,
                &format!(
                    "`{text}` is a null pointer, but {function} needs a pointer to a string: \
                     there are no characters to read"
                ),
                &clauses::LIBRARY_ARGUMENTS,
                Undefined::NullString,
            ));
        };
        let mut bytes = Vec::new();
        let mut at = offset;
        loop {
            if at >= first + length {
                let array = self.array_name(target, first);
                let from = self.location(target, offset);
                return Err(Diagnostic::ub(
                    self.state.source_range,
                    &format!(
                        "{function} reads the string at {from} up to its null character, but no \
                         element from there to the end of {array} is a null character, so it \
                         would read past the end of {array}"
                    ),
                    &clauses::STRING,
                    Undefined::Unterminated,
                ));
            }
            match self.read(target, at)? {
                | Value::Integer(0) => return Ok(bytes),
                | Value::Integer(character) => bytes.push(character.to_le_bytes()[0]),
                | _ => return Err(self.error("a string's elements are chars")),
            }
            at += 1;
        }
    }

    /// Interpret `format` with `arguments` (the format first): what each
    /// piece did, and every byte written.
    fn format(
        &self,
        format: &[u8],
        arguments: &[Value],
        types: &[Type],
        texts: &[String],
    ) -> Result<(Vec<Piece>, Vec<u8>), Diagnostic> {
        let mut pieces = Vec::new();
        let mut written = Vec::new();
        let mut next = 1;
        let mut at = 0;
        while at < format.len() {
            if format[at] != b'%' {
                let end = format[at..]
                    .iter()
                    .position(|&byte| byte == b'%')
                    .map_or(format.len(), |end| at + end);
                written.extend_from_slice(&format[at..end]);
                pieces.push(Piece::Copied(format[at..end].to_vec()));
                at = end;
                continue;
            }
            let specification = self.specification(format, &mut at)?;
            if specification.conversion == b'%' {
                written.push(b'%');
                pieces.push(Piece::Converted {
                    specification: specification.text,
                    argument:      None,
                    source:        String::new(),
                    took:          String::new(),
                    wrote:         Vec::from(*b"%"),
                });
                continue;
            }
            let Some(argument) = arguments.get(next) else {
                return Err(Diagnostic::ub(
                    self.state.source_range,
                    &format!(
                        "the conversion `{}` needs an argument, but the call has only {} after \
                         the format",
                        specification.text,
                        arguments.len() - 1
                    ),
                    &clauses::FPRINTF,
                    Undefined::MissingArgument,
                ));
            };
            let ty = types.get(next).copied().unwrap_or(Type::Int);
            let text = texts.get(next).map_or("", String::as_str);
            let converted = self.convert(&specification, argument, ty, next, text)?;
            let padded = pad(&specification, converted);
            written.extend_from_slice(&padded);
            pieces.push(Piece::Converted {
                specification: specification.text,
                argument:      Some(next),
                source:        String::from(text),
                took:          if text.is_empty() {
                    self.argument_text(argument)
                } else {
                    format!("`{text}`, {}", self.argument_text(argument))
                },
                wrote:         padded,
            });
            next += 1;
        }
        Ok((pieces, written))
    }

    /// The conversion specification starting at the `%` at `*at`, which
    /// moves past it.
    fn specification(&self, format: &[u8], at: &mut usize) -> Result<Specification, Diagnostic> {
        let start = *at;
        *at += 1;
        let (mut left, mut zero) = (false, false);
        while let Some(&flag) = format.get(*at) {
            match flag {
                | b'-' => left = true,
                | b'0' => zero = true,
                | b'+' | b' ' | b'#' => {
                    let text = escaped(&format[start..=*at]);
                    return Err(self.error(&format!(
                        "the {} flag in `{text}` is outside this subset; the machine takes the - \
                         and 0 flags",
                        char::from(flag)
                    )));
                },
                | _ => break,
            }
            *at += 1;
        }
        let digits = format[*at..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let width = std::str::from_utf8(&format[*at..*at + digits])
            .ok()
            .and_then(|digits| digits.parse::<usize>().ok())
            .unwrap_or(0);
        *at += digits;
        if width > 256 {
            return Err(Diagnostic::limit(
                self.state.source_range,
                "a field width above 256 is outside this subset",
            ));
        }
        let Some(&conversion) = format.get(*at) else {
            return Err(Diagnostic::ub(
                self.state.source_range,
                &format!(
                    "the format ends inside the conversion specification `{}`, which has no \
                     conversion specifier",
                    escaped(&format[start..])
                ),
                &clauses::PRINTF_UNDEFINED,
                Undefined::InvalidConversion,
            ));
        };
        *at += 1;
        let text = escaped(&format[start..*at]);
        let invalid = |message: String| {
            Diagnostic::ub(
                self.state.source_range,
                &message,
                &clauses::PRINTF_UNDEFINED,
                Undefined::InvalidConversion,
            )
        };
        match conversion {
            | b'%' if *at - start != 2 => {
                return Err(invalid(format!(
                    "`{text}` is not a valid conversion specification: a percent sign is written \
                     only by exactly %%"
                )));
            },
            | b'c' | b's' if zero => {
                return Err(Diagnostic::ub(
                    self.state.source_range,
                    &format!(
                        "the 0 flag in `{text}` is defined only for numeric conversions, not %{}",
                        char::from(conversion)
                    ),
                    &clauses::PRINTF_FLAGS,
                    Undefined::InvalidConversion,
                ));
            },
            | b'%' | b'd' | b'i' | b'u' | b'x' | b'c' | b's' => {},
            | b'.' | b'*' | b'h' | b'l' | b'L' | b'j' | b'z' | b't' => {
                return Err(self.error(&format!(
                    "`{text}…` uses a precision, `*` or a length modifier, which are outside this \
                     subset; the machine takes flags, a width and a conversion"
                )));
            },
            | b'o' | b'X' | b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' | b'p' | b'n' => {
                return Err(self.error(&format!(
                    "the conversion `{text}` is outside this subset; the machine takes %d, %i, \
                     %u, %x, %c, %s and %%"
                )));
            },
            | _ => {
                return Err(invalid(format!(
                    "`{text}` is not a valid conversion specification: C has no %{} conversion",
                    escaped(&[conversion])
                )));
            },
        }
        Ok(Specification {
            left,
            zero: zero && !left,
            width,
            conversion,
            text,
        })
    }

    /// The text `specification` makes of argument `index`, `argument` of
    /// type `ty`, spelled `text` in the call.
    fn convert(
        &self,
        specification: &Specification,
        argument: &Value,
        ty: Type,
        index: usize,
        text: &str,
    ) -> Result<Vec<u8>, Diagnostic> {
        let conversion = specification.conversion;
        let wrong = |expected: &str, detail: &str| {
            Diagnostic::ub(
                self.state.source_range,
                &format!(
                    "`{}` needs {expected}, but argument {} after the format, `{text}`, is \
                     {}{detail}",
                    specification.text,
                    index,
                    type_text(ty)
                ),
                &clauses::PRINTF_UNDEFINED,
                Undefined::WrongArgument,
            )
        };
        match (conversion, argument) {
            | (b'd' | b'i' | b'c' | b'u' | b'x', Value::Integer(value)) if ty == Type::Int =>
                Ok(match conversion {
                    | b'd' | b'i' => format!("{value}").into_bytes(),
                    | b'c' => Vec::from([value.to_le_bytes()[0]]),
                    | _ if *value < 0 => {
                        return Err(wrong(
                            "an unsigned int",
                            &format!(
                                " with the value {value}, which an unsigned int cannot hold: an \
                                 int may stand for an unsigned int only when its value fits in \
                                 both"
                            ),
                        ));
                    },
                    | b'u' => format!("{value}").into_bytes(),
                    | _ => format!("{value:x}").into_bytes(),
                }),
            | (b'd' | b'i' | b'c', _) => Err(wrong("an int", "")),
            | (b'u' | b'x', _) => Err(wrong("an unsigned int", "")),
            | (_, pointer @ Value::Pointer { .. }) if matches!(ty, Type::CharPointer(_)) =>
                self.read_string(pointer, "printf's %s", text),
            | _ => Err(wrong("a pointer to a string (a char *)", "")),
        }
    }

    /// An argument's value in words.
    fn argument_text(&self, value: &Value) -> String {
        match value {
            | Value::Integer(integer) => format!("the int {integer}"),
            | Value::Pointer { target: None, .. } => String::from("a null pointer"),
            | &Value::Pointer {
                target: Some(target),
                offset,
                ..
            } => format!("a pointer to {}", self.location(target, offset)),
            | _ => String::from("a value"),
        }
    }

    fn putchar_event(range: super::SourceRange, text: &str, value: i32, byte: u8) -> Event {
        let shown = quoted(&[byte]);
        let converted = if value == i32::from(byte) {
            String::new()
        } else {
            format!(
                " The int {value} is first converted to unsigned char, which keeps its low 8 \
                 bits: {byte}."
            )
        };
        let explanation = format!(
            "putchar(c) is fputc(c, stdout): it writes one character, the int `{text}` = {value} \
             converted to unsigned char, to standard output, and returns that character as an \
             int.{converted} The character written is {} (byte {byte}).",
            character_name(byte)
        );
        let mut event = library_event(
            EventKind::LibraryCall,
            range,
            format!("putchar writes {shown}"),
            &format!(
                "putchar writes one character, {}, to standard output and returns it, {byte}.",
                character_name(byte)
            ),
            &explanation,
            &[
                clauses::PUTCHAR,
                clauses::FPUTC,
                clauses::FPUTC_RETURN,
                clauses::LIBRARY_SEQUENCE_POINT,
            ],
        );
        event.operands.push(Operand::Value(Value::Integer(value)));
        event.result = Some(Operand::Value(Value::Integer(i32::from(byte))));
        event
    }

    fn puts_event(
        &self,
        range: super::SourceRange,
        text: &str,
        pointer: &Value,
        written: &[u8],
        returned: i32,
    ) -> Event {
        let string = &written[..written.len() - 1];
        let explanation = format!(
            "puts receives `{text}`, {}, and reads the string there character by character up to \
             its null character. It writes those {} characters to standard output, and then a \
             new-line: {}. The null itself is not written. C promises only a nonnegative result \
             when puts succeeds; this machine returns the number of characters written, \
             {returned}, as glibc does.",
            self.argument_text(pointer),
            string.len(),
            quoted(written)
        );
        let mut event = library_event(
            EventKind::LibraryCall,
            range,
            format!("puts writes {}", quoted(written)),
            &format!(
                "puts writes the string {} and then a new-line to standard output.",
                quoted(string)
            ),
            &explanation,
            &[
                clauses::PUTS,
                clauses::STRING,
                clauses::PUTS_RETURN,
                clauses::LIBRARY_SEQUENCE_POINT,
            ],
        );
        event.operands.push(Operand::Value(pointer.clone()));
        event.objects.extend(pointer_target(pointer));
        event.result = Some(Operand::Value(Value::Integer(returned)));
        event
    }

    fn printf_event(
        range: super::SourceRange,
        format: &[u8],
        pointer: &Value,
        arguments: &[Value],
        pieces: &[Piece],
        written: &[u8],
    ) -> Event {
        let mut steps = String::new();
        let mut fed = Vec::new();
        for piece in pieces {
            if !steps.is_empty() {
                steps.push_str("; ");
            }
            match piece {
                | Piece::Copied(bytes) => {
                    let _ = write!(steps, "{} is copied as it is", quoted(bytes));
                },
                | Piece::Converted {
                    specification,
                    argument: None,
                    ..
                } => {
                    let _ = write!(steps, "`{specification}` writes a percent sign");
                },
                | Piece::Converted {
                    specification,
                    argument: Some(index),
                    source,
                    took,
                    wrote,
                } => {
                    let _ = write!(
                        steps,
                        "`{specification}` takes argument {index} after the format, {took}, and \
                         writes {}",
                        quoted(wrote)
                    );
                    let taken = if source.is_empty() {
                        format!("argument {index}")
                    } else {
                        format!("`{source}`")
                    };
                    fed.push(format!(
                        "{specification} took {taken} and wrote {}",
                        quoted(wrote)
                    ));
                },
            }
        }
        let count = written.len();
        let mut explanation = format!(
            "printf is fprintf writing to standard output. It reads its format, {}, from left to \
             right: ordinary characters are copied, and each conversion specification, starting \
             with %, takes the next argument and writes it as text. Here {steps}. In all it \
             writes {} ({count} character{}), and returns {count}, the number of characters \
             written.",
            quoted(format),
            quoted(written),
            if count == 1 { "" } else { "s" },
        );
        let variadic = arguments.len() > 1;
        if variadic {
            explanation.push_str(
                " Every argument after the format went through printf's `...`, where C applies \
                 the default argument promotions: a char arrives as an int, which is why %c and \
                 %d both take an int.",
            );
        }
        let extra = arguments.len()
            - 1
            - pieces
                .iter()
                .filter(|piece| {
                    matches!(
                        piece,
                        Piece::Converted {
                            argument: Some(_),
                            ..
                        }
                    )
                })
                .count();
        if extra > 0 {
            let _ = write!(
                explanation,
                " The format uses fewer arguments than the call passes: the {extra} extra \
                 argument{} were evaluated and are ignored.",
                if extra == 1 { "" } else { "s" }
            );
        }
        let summary = if fed.is_empty() {
            format!(
                "printf copies its format to standard output: {}.",
                quoted(written)
            )
        } else {
            let last = fed.pop().unwrap_or_default();
            let all = if fed.is_empty() {
                last
            } else {
                format!("{}, and {last}", fed.join(", "))
            };
            format!(
                "printf writes {} to standard output: {all}.",
                quoted(written)
            )
        };
        let mut cited = vec![clauses::PRINTF, clauses::FPRINTF, clauses::CONVERSIONS];
        if pieces.iter().any(|piece| {
            matches!(piece, Piece::Converted { specification, .. } if specification.len() > 2)
        }) {
            cited.extend([clauses::CONVERSION_SPECIFICATION, clauses::PRINTF_FLAGS]);
        }
        if variadic {
            cited.extend([clauses::VARIADIC_ARGUMENTS, clauses::DEFAULT_PROMOTIONS]);
        }
        cited.extend([clauses::PRINTF_RETURN, clauses::LIBRARY_SEQUENCE_POINT]);
        let mut event = narration::noted(
            library_event(
                EventKind::LibraryCall,
                range,
                format!("printf writes {}", quoted(written)),
                &summary,
                &explanation,
                &cited,
            ),
            Topic::Format,
        );
        event
            .operands
            .extend(arguments.iter().cloned().map(Operand::Value));
        event.objects.extend(pointer_target(pointer));
        for argument in &arguments[1..] {
            if let Some(target) = pointer_target(argument)
                && !event.objects.contains(&target)
            {
                event.objects.push(target);
            }
        }
        event.result = Some(Operand::Value(Value::Integer(
            i32::try_from(count).unwrap_or(i32::MAX),
        )));
        event
    }
}

/// `specification`'s text for `converted`, padded to its field width.
fn pad(specification: &Specification, mut converted: Vec<u8>) -> Vec<u8> {
    let Some(missing) = specification.width.checked_sub(converted.len()) else {
        return converted;
    };
    if specification.left {
        converted.resize(specification.width, b' ');
        return converted;
    }
    // Zeros go after the sign (C99 §7.19.6.1p6).
    let (sign, digits) = if specification.zero && converted.first() == Some(&b'-') {
        converted.split_at(1)
    } else {
        converted.split_at(0)
    };
    let fill = if specification.zero { b'0' } else { b' ' };
    let mut padded = Vec::with_capacity(specification.width);
    if specification.zero {
        padded.extend_from_slice(sign);
        padded.resize(padded.len() + missing, fill);
        padded.extend_from_slice(digits);
    } else {
        padded.resize(missing, fill);
        padded.extend_from_slice(&converted);
    }
    padded
}

/// The object a pointer value points into.
const fn pointer_target(value: &Value) -> Option<usize> {
    match value {
        | Value::Pointer { target, .. } => *target,
        | _ => None,
    }
}

/// How C spells a type, for a mismatched argument.
fn type_text(ty: Type) -> &'static str {
    match ty {
        | Type::Int | Type::Char => "an int",
        | Type::Pointer => "a pointer to int",
        | Type::CharPointer(_) => "a pointer to char",
        | Type::StructPointer(_) => "a pointer to a structure",
        | _ => "an aggregate",
    }
}

/// `bytes` as a C string literal spells them: `"Ada\n"`.
pub(super) fn quoted(bytes: &[u8]) -> String {
    format!("\"{}\"", escaped(bytes))
}

/// `bytes` with escape sequences for every character that is not printable
/// ASCII, as a C literal would write them.
pub(super) fn escaped(bytes: &[u8]) -> String {
    let mut text = String::new();
    for &byte in bytes {
        match byte {
            | b'\n' => text.push_str("\\n"),
            | b'\t' => text.push_str("\\t"),
            | b'\r' => text.push_str("\\r"),
            | b'\\' => text.push_str("\\\\"),
            | b'"' => text.push_str("\\\""),
            | 0 => text.push_str("\\0"),
            | 0x20..=0x7E => text.push(char::from(byte)),
            | _ => {
                let _ = write!(text, "\\x{byte:02x}");
            },
        }
    }
    text
}

/// A character as a constant spells it: `'A'`, `'\n'`, `'\0'`.
pub(super) fn character_name(byte: u8) -> String {
    match byte {
        | b'\'' => String::from("'\\''"),
        | b'"' => String::from("'\"'"),
        | _ => format!("'{}'", escaped(&[byte])),
    }
}

fn library_event(
    kind: EventKind,
    range: super::SourceRange,
    title: String,
    summary: &str,
    explanation: &str,
    cited: &[Clause],
) -> Event {
    let mut event = Event::new(kind, range, title);
    event.summary = String::from(summary);
    event.explanation = String::from(explanation);
    event.clauses.extend_from_slice(cited);
    event.sequence_point = true;
    event
}
