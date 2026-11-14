//! The three output functions `<stdio.h>` declares that the machine runs
//! itself: `printf`, `puts` and `putchar`. Each call is one step that reads
//! its arguments, writes to standard output (`State::output`) and returns.
//! Reading a string follows its pointer element by element to the null
//! character, with the machine's ordinary lifetime, bounds and
//! indeterminate checks; a format is read the same way, then interpreted
//! one conversion specification at a time.
//!
//! The subset of `printf` is the conversions `%d %i %u %x %c %s %%` and the
//! floating `%f %F %e %E %g %G %a %A`, with the `-` and `0` flags, a field
//! width, a precision, the length modifiers `hh h l ll z` for the integer
//! conversions and `L` (and `l`, which changes nothing) for the floating
//! ones. Floating values are written exactly as glibc writes them: the
//! exact binary value's decimal digits, rounded to the nearest, ties to
//! even. Other valid specifications (`*`, the `j t` modifiers, other
//! conversions and flags) stop with an explicit diagnostic; invalid ones (a
//! precision with `%c` among them), a missing argument and an argument of
//! the wrong type are undefined behavior.
//!
//! C99: §7.19.6.1 paragraphs 2, 4, 6, 7, 8 and 9, pp. 274-280; PDF pp.
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
    floating::Floating,
    integers::Integer,
    machine::Machine,
    memory::article,
    model::{
        Arithmetic,
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
        /// What the precision did, or empty.
        precision:     String,
    },
}

/// A parsed conversion specification.
struct Specification {
    left:         bool,
    zero:         bool,
    width:        usize,
    /// The precision, when the specification has one.
    precision:    Option<usize>,
    /// Whether a precision made the 0 flag be ignored (C99 §7.19.6.1p6).
    zero_ignored: bool,
    /// The length modifier: `""`, `hh`, `h`, `l`, `ll`, `z` or `L`.
    length:       &'static str,
    conversion:   u8,
    text:         String,
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
                let Some(&Value::Integer(argument)) = arguments.first() else {
                    return Err(self.error("putchar needs an int"));
                };
                // The prototype converts the argument to int, and fputc
                // converts that int to unsigned char (C99 §7.19.7.3p2).
                let character = Integer::Int.convert(argument);
                let byte = character.to_le_bytes()[0];
                self.write(&[byte])?;
                let returned = i128::from(byte);
                self.push(Operand::Value(Value::Integer(returned)))?;
                Ok(Self::putchar_event(range, text(0), character, byte))
            },
            | Library::Puts => {
                let pointer = arguments.first().cloned().unwrap_or(Value::Indeterminate);
                let mut string = self.read_string(&pointer, "puts", text(0), None)?;
                string.push(b'\n');
                self.write(&string)?;
                // C promises only a nonnegative value; glibc returns the
                // number of characters written.
                let returned = i128::try_from(string.len()).unwrap_or(i128::MAX);
                self.push(Operand::Value(Value::Integer(returned)))?;
                Ok(self.puts_event(range, text(0), &pointer, &string, returned))
            },
            | Library::Printf => {
                let pointer = arguments.first().cloned().unwrap_or(Value::Indeterminate);
                let format = self.read_string(&pointer, "printf", text(0), None)?;
                let (pieces, written) = self.format(&format, &arguments, types, &texts)?;
                self.write(&written)?;
                let returned = i128::try_from(written.len()).unwrap_or(i128::MAX);
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
    /// at a time up to its null character, which is not included, or up to
    /// `limit` characters: a `%s` with a precision reads no further, so its
    /// array needs no null (C99 §7.19.6.1p8). `function` reads it as its
    /// argument `text`.
    fn read_string(
        &self,
        pointer: &Value,
        function: &str,
        text: &str,
        limit: Option<usize>,
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
            if limit == Some(bytes.len()) {
                return Ok(bytes);
            }
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
                    precision:     String::new(),
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
            let ty = types.get(next).copied().unwrap_or(Type::INT);
            let text = texts.get(next).map_or("", String::as_str);
            let (converted, printed) = self.convert(&specification, argument, ty, next, text)?;
            let precision = precision_note(&specification, printed, &converted);
            let padded = pad(&specification, converted);
            written.extend_from_slice(&padded);
            // %zu takes a size_t, and an unsigned long given to it is one,
            // named so (C99 §7.19.6.1p7, §7.17p2).
            let named = (specification.length == "z" && ty == Type::Integer(Integer::UnsignedLong))
                .then_some("size_t");
            let took = self.argument_text(argument, Some(ty), named);
            pieces.push(Piece::Converted {
                specification: specification.text,
                argument: Some(next),
                source: String::from(text),
                took: if text.is_empty() {
                    took
                } else {
                    format!("`{text}`, {took}")
                },
                wrote: padded,
                precision,
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
        // A period and an optional decimal integer: just the period is a
        // precision of zero (C99 §7.19.6.1p4).
        let precision = if format.get(*at) == Some(&b'.') && format.get(*at + 1) != Some(&b'*') {
            *at += 1;
            let digits = format[*at..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            let precision = std::str::from_utf8(&format[*at..*at + digits])
                .ok()
                .and_then(|digits| digits.parse::<usize>().ok())
                .unwrap_or(if digits == 0 { 0 } else { usize::MAX });
            *at += digits;
            if precision > 256 {
                return Err(Diagnostic::limit(
                    self.state.source_range,
                    "a precision above 256 is outside this subset",
                ));
            }
            Some(precision)
        } else {
            None
        };
        // A length modifier: hh, h, l, ll, z or L (C99 §7.19.6.1p7).
        let rest = &format[*at..];
        let length = ["hh", "ll", "h", "l", "z", "L"]
            .into_iter()
            .find(|modifier| rest.starts_with(modifier.as_bytes()))
            .unwrap_or("");
        *at += length.len();
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
            | b'c' if precision.is_some() => {
                return Err(invalid(format!(
                    "`{text}` gives %c a precision, which C defines only for the numeric \
                     conversions and %s"
                )));
            },
            | b'c' | b's' if length == "l" => {
                return Err(self.error(&format!(
                    "`{text}` takes a wide character or string, which is outside this subset"
                )));
            },
            | b'c' | b's' | b'%' if !length.is_empty() => {
                return Err(invalid(format!(
                    "`{text}` puts the length modifier {length} before %{}, which C defines it \
                     for only with the integer conversions d, i, o, u, x and X",
                    char::from(conversion)
                )));
            },
            | b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A'
                if !matches!(length, "" | "l" | "L") =>
            {
                return Err(invalid(format!(
                    "`{text}` puts the length modifier {length} before %{}, which C defines only \
                     with the integer conversions; a floating conversion takes L for a long \
                     double, or nothing for a double",
                    char::from(conversion)
                )));
            },
            | b'd' | b'i' | b'u' | b'x' if length == "L" => {
                return Err(invalid(format!(
                    "`{text}` puts the length modifier L before %{}, which C defines only with \
                     the floating conversions; ll is the modifier for a long long",
                    char::from(conversion)
                )));
            },
            | b'%' | b'd' | b'i' | b'u' | b'x' | b'c' | b's' | b'f' | b'F' | b'e' | b'E' | b'g'
            | b'G' | b'a' | b'A' => {},
            | b'.' | b'*' | b'h' | b'l' | b'L' | b'j' | b'z' | b't' => {
                return Err(self.error(&format!(
                    "`{text}…` uses `*` or the length modifier j or t, which are outside this \
                     subset; the machine takes flags, a width, a precision, hh, h, l, ll, z or L, \
                     and a conversion"
                )));
            },
            | b'o' | b'X' | b'p' | b'n' => {
                return Err(self.error(&format!(
                    "the conversion `{text}` is outside this subset; the machine takes %d, %i, \
                     %u, %x, %c, %s, %%, %f, %e, %g and %a"
                )));
            },
            | _ => {
                return Err(invalid(format!(
                    "`{text}` is not a valid conversion specification: C has no %{} conversion",
                    escaped(&[conversion])
                )));
            },
        }
        // With a precision, the integer conversions ignore the 0 flag
        // (C99 §7.19.6.1p6); the floating ones keep it.
        let zero_ignored =
            zero && !left && precision.is_some() && matches!(conversion, b'd' | b'i' | b'u' | b'x');
        Ok(Specification {
            left,
            zero: zero && !left && !zero_ignored,
            width,
            precision,
            zero_ignored,
            length,
            conversion,
            text,
        })
    }

    /// The text `specification` makes of argument `index`, `argument` of
    /// type `ty`, spelled `text` in the call, and the integer it printed.
    /// An integer argument must have the type its conversion and length
    /// modifier name, or that type's signed or unsigned counterpart with a
    /// value both can hold (C99 §6.2.5p9, §7.19.6.1p9).
    fn convert(
        &self,
        specification: &Specification,
        argument: &Value,
        ty: Type,
        index: usize,
        text: &str,
    ) -> Result<(Vec<u8>, Option<i128>), Diagnostic> {
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
        let digits = |text: String| {
            // The precision is the minimum number of digits; a zero value
            // with precision zero writes none (C99 §7.19.6.1p8).
            let (sign, digits) = text.split_at(usize::from(text.starts_with('-')));
            let mut written = Vec::from(sign.as_bytes());
            match specification.precision {
                | Some(0) if digits == "0" => {},
                | Some(precision) => {
                    written.resize(written.len() + precision.saturating_sub(digits.len()), b'0');
                    written.extend_from_slice(digits.as_bytes());
                },
                | None => written.extend_from_slice(digits.as_bytes()),
            }
            written
        };
        if matches!(conversion.to_ascii_lowercase(), b'f' | b'e' | b'g' | b'a') {
            let (expected, wanted) = if specification.length == "L" {
                (Floating::LongDouble, "a long double")
            } else {
                (Floating::Double, "a double")
            };
            return match (ty, argument) {
                | (Type::Floating(given), &Value::Floating(real)) if given == expected => Ok((
                    real.printf(conversion, specification.precision)
                        .into_bytes(),
                    None,
                )),
                | (Type::Floating(Floating::LongDouble), _) => Err(wrong(
                    wanted,
                    "; a long double needs the L modifier, as in %Lf",
                )),
                | (Type::Floating(_), _) => Err(wrong(
                    wanted,
                    "; %Lf and the other L conversions take only a long double",
                )),
                | _ => Err(wrong(
                    wanted,
                    "; printf does not convert an integer to floating, it reads the argument's \
                     bytes as the type the conversion names",
                )),
            };
        }
        if conversion == b's' {
            return match argument {
                | pointer @ Value::Pointer { .. }
                    if matches!(ty, Type::Pointer(Arithmetic::CHAR, _)) =>
                    self.read_string(pointer, "printf's %s", text, specification.precision)
                        .map(|bytes| (bytes, None)),
                | _ => Err(wrong("a pointer to a string (a char *)", "")),
            };
        }
        let signed = matches!(conversion, b'd' | b'i' | b'c');
        // The type the argument must have after the default argument
        // promotions, and the type whose value is printed.
        let (expected, printed) = match (specification.length, signed) {
            | ("hh", true) => (Integer::Int, Integer::SignedChar),
            | ("hh", false) => (Integer::Int, Integer::UnsignedChar),
            | ("h", true) => (Integer::Int, Integer::Short),
            | ("h", false) => (Integer::Int, Integer::UnsignedShort),
            | ("l" | "z", true) => (Integer::Long, Integer::Long),
            | ("l" | "z", false) => (Integer::UnsignedLong, Integer::UnsignedLong),
            | ("ll", true) => (Integer::LongLong, Integer::LongLong),
            | ("ll", false) => (Integer::UnsignedLongLong, Integer::UnsignedLongLong),
            | (_, true) => (Integer::Int, Integer::Int),
            | (_, false) => (Integer::UnsignedInt, Integer::UnsignedInt),
        };
        let wanted = article(expected.spelling());
        let (Type::Integer(given), &Value::Integer(value)) = (ty, argument) else {
            return Err(wrong(&wanted, ""));
        };
        let counterpart = if expected.signed() {
            expected.to_unsigned()
        } else {
            expected.to_signed()
        };
        if given != expected {
            if given != counterpart {
                return Err(wrong(&wanted, ""));
            }
            if !(expected.fits(value) && counterpart.fits(value)) {
                return Err(wrong(
                    &wanted,
                    &format!(
                        " with the value {value}, which {wanted} cannot hold: {} may stand for {} \
                         only when its value fits in both",
                        article(given.spelling()),
                        expected.spelling()
                    ),
                ));
            }
        }
        // hh and h convert the promoted value back before printing it (C99
        // §7.19.6.1p7).
        let shown = printed.convert(value);
        let written = match conversion {
            | b'c' => Vec::from([value.to_le_bytes()[0]]),
            | b'x' => digits(format!("{shown:x}")),
            | _ => digits(format!("{shown}")),
        };
        Ok((written, Some(shown)))
    }

    /// An argument's value in words, of type `ty` when known, spelled as
    /// the typedef name `named` when the conversion names one.
    fn argument_text(&self, value: &Value, ty: Option<Type>, named: Option<&str>) -> String {
        match value {
            | Value::Integer(integer) => {
                let name = ty.and_then(Type::integer).unwrap_or(Integer::Int);
                format!("the {} {integer}", named.unwrap_or(name.spelling()))
            },
            | Value::Floating(real) => format!("the {} {}", real.format().spelling(), real.short()),
            | Value::Pointer { target: None, .. } => String::from("a null pointer"),
            | &Value::Pointer {
                target: Some(target),
                offset,
                ..
            } => format!("a pointer to {}", self.location(target, offset)),
            | _ => String::from("a value"),
        }
    }

    fn putchar_event(range: super::SourceRange, text: &str, value: i128, byte: u8) -> Event {
        let shown = quoted(&[byte]);
        let converted = if value == i128::from(byte) {
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
        event.result = Some(Operand::Value(Value::Integer(i128::from(byte))));
        event
    }

    fn puts_event(
        &self,
        range: super::SourceRange,
        text: &str,
        pointer: &Value,
        written: &[u8],
        returned: i128,
    ) -> Event {
        let string = &written[..written.len() - 1];
        let explanation = format!(
            "puts receives `{text}`, {}, and reads the string there character by character up to \
             its null character. It writes those {} characters to standard output, and then a \
             new-line: {}. The null itself is not written. C promises only a nonnegative result \
             when puts succeeds; this machine returns the number of characters written, \
             {returned}, as glibc does.",
            self.argument_text(pointer, None, None),
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

    /// printf's step: what it wrote, and which argument each conversion
    /// took. The lead says what each conversion wrote too, for up to
    /// [`LEAD_CONVERSIONS`] of them.
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
                    precision,
                } => {
                    let _ = write!(
                        steps,
                        "`{specification}` takes argument {index} after the format, {took}, and \
                         writes {}",
                        quoted(wrote)
                    );
                    if !precision.is_empty() {
                        let _ = write!(steps, " ({precision})");
                    }
                    let taken = if source.is_empty() {
                        format!("argument {index}")
                    } else {
                        format!("`{source}`")
                    };
                    fed.push((
                        format!("{specification} took {taken}"),
                        format!(" and wrote {}", quoted(wrote)),
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
                 the default argument promotions: a char or a short arrives as an int, which is \
                 why %c and %d both take an int, and why hh and h say only how to print it.",
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
            // Past three conversions the lead says only which argument fed
            // each, and leaves the text to the title, so it stays a
            // sentence of a few lines; the explanation says what each wrote.
            let conversions = fed.len();
            let wrote = conversions <= LEAD_CONVERSIONS;
            let mut fed: Vec<String> = fed
                .into_iter()
                .map(|(took, what)| if wrote { took + &what } else { took })
                .collect();
            let last = fed.pop().unwrap_or_default();
            let all = if fed.is_empty() {
                last
            } else {
                format!("{}, and {last}", fed.join(", "))
            };
            if wrote {
                format!(
                    "printf writes {} to standard output: {all}.",
                    quoted(written)
                )
            } else {
                format!(
                    "printf writes its format to standard output with its {conversions} \
                     conversions filled in: {all}."
                )
            }
        };
        let mut cited = vec![clauses::PRINTF, clauses::FPRINTF, clauses::CONVERSIONS];
        if pieces.iter().any(|piece| {
            matches!(piece, Piece::Converted { specification, .. } if specification.len() > 2)
        }) {
            cited.extend([clauses::CONVERSION_SPECIFICATION, clauses::PRINTF_FLAGS]);
        }
        if pieces.iter().any(
            |piece| matches!(piece, Piece::Converted { precision, .. } if !precision.is_empty()),
        ) {
            cited.insert(3, clauses::PRECISION);
        }
        let floating = pieces.iter().any(|piece| {
            matches!(piece, Piece::Converted { specification, .. }
                if specification.ends_with(['f', 'F', 'e', 'E', 'g', 'G', 'a', 'A']))
        });
        if floating {
            cited.insert(3, clauses::FLOATING_CONVERSIONS);
            cited.push(clauses::FLOATING_OPERATIONS);
        }
        if pieces.iter().any(|piece| {
            matches!(piece, Piece::Converted { specification, .. } if specification.contains('L'))
        }) {
            cited.push(clauses::LONG_DOUBLE_MODIFIER);
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
            i128::try_from(count).unwrap_or(i128::MAX),
        )));
        event
    }
}

/// What the precision of `specification` meant for `argument`, which it
/// converted to `converted` (before any padding), in words; empty without
/// a precision.
///
/// C99: §7.19.6.1 paragraph 4, p. 275; PDF p. 287.
fn precision_note(
    specification: &Specification,
    printed: Option<i128>,
    converted: &[u8],
) -> String {
    if matches!(
        specification.conversion.to_ascii_lowercase(),
        b'f' | b'e' | b'g' | b'a'
    ) {
        return narration::floating_precision(specification.conversion, specification.precision);
    }
    let Some(precision) = specification.precision else {
        return String::new();
    };
    let ignored = if specification.zero_ignored {
        "; with a precision the 0 flag is ignored, so any padding is spaces"
    } else {
        ""
    };
    if specification.conversion == b's' {
        let plural = if precision == 1 { "" } else { "s" };
        return if converted.len() == precision {
            format!(
                "the precision .{precision} writes at most {precision} character{plural} of the \
                 string, so printf stops there, without needing to reach the null"
            )
        } else {
            format!(
                "the precision .{precision} allows up to {precision} character{plural}, and the \
                 string has only {}, so all of it is written",
                converted.len()
            )
        };
    }
    let written = converted
        .iter()
        .filter(|byte| byte.is_ascii_hexdigit())
        .count();
    let zero = printed == Some(0);
    let plural = if precision == 1 { "" } else { "s" };
    if precision == 0 && zero {
        return format!("the precision .0 with the value 0 writes no digits at all{ignored}");
    }
    let natural = match printed {
        | Some(value) if specification.conversion == b'x' => format!("{value:x}").len(),
        | Some(value) => format!("{}", value.unsigned_abs()).len(),
        | None => written,
    };
    let leading = written.saturating_sub(natural);
    if leading == 0 {
        format!(
            "the precision .{precision} asks for at least {precision} digit{plural}, and the \
             value already has {natural}, so it adds nothing{ignored}"
        )
    } else {
        format!(
            "the precision .{precision} asks for at least {precision} digit{plural}, so {leading} \
             leading zero{} {} added{ignored}",
            if leading == 1 { "" } else { "s" },
            if leading == 1 { "is" } else { "are" }
        )
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
    // Zeros go after the sign, and after the 0x of %a (C99 §7.19.6.1p6);
    // glibc pads an infinity or a NaN with spaces.
    let signed = usize::from(converted.first() == Some(&b'-'));
    let prefix = if converted[signed..].starts_with(b"0x") || converted[signed..].starts_with(b"0X")
    {
        signed + 2
    } else {
        signed
    };
    let special = converted
        .iter()
        .any(|byte| matches!(byte.to_ascii_lowercase(), b'n' | b'i'));
    let zero = specification.zero && !special;
    let (sign, digits) = if zero {
        converted.split_at(prefix)
    } else {
        converted.split_at(0)
    };
    let fill = if zero { b'0' } else { b' ' };
    let mut padded = Vec::with_capacity(specification.width);
    if zero {
        padded.extend_from_slice(sign);
        padded.resize(padded.len() + missing, fill);
        padded.extend_from_slice(digits);
    } else {
        padded.resize(missing, fill);
        padded.extend_from_slice(&converted);
    }
    padded
}

/// The most conversions a printf step's lead lists with the text each
/// wrote; past that the lead names only their arguments, so it stays one
/// sentence of a few lines.
const LEAD_CONVERSIONS: usize = 3;

/// The object a pointer value points into.
const fn pointer_target(value: &Value) -> Option<usize> {
    match value {
        | Value::Pointer { target, .. } => *target,
        | _ => None,
    }
}

/// How C spells a type, for a mismatched argument.
fn type_text(ty: Type) -> String {
    match ty {
        | Type::Integer(integer) => article(integer.spelling()),
        | Type::Floating(floating) => article(floating.spelling()),
        | Type::Pointer(element, _) => format!("a pointer to {}", element.spelling()),
        | Type::StructPointer(_) => String::from("a pointer to a structure"),
        | _ => String::from("an aggregate"),
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
