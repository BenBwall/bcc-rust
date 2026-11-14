//! The explanations of the floating types: a constant's exact stored value,
//! the conversions to, from and between floating types, floating
//! arithmetic and its rounding, comparisons with their exact operands and
//! NaN, and division by zero under Annex F.

use super::{
    Topic,
    UNORDERED,
    explained,
    noted,
    result,
    snippet,
};
use crate::abstract_machine::{
    clauses::{
        self,
        Clause,
    },
    floating::{
        Floating,
        Real,
    },
    machine::Machine,
    memory::article,
    model::{
        Arithmetic,
        Event,
        EventKind,
        Operand,
        SourceRange,
        Type,
        Value,
    },
};

/// The longest exact decimal the lead sentences write out; the details
/// give every digit.
const LEAD_DIGITS: usize = 60;

/// The floating constant spelled `spelling` evaluates to `value`, which
/// its type holds exactly when `exact`.
///
/// C99: §6.4.4.2 paragraphs 3-4, p. 58; PDF p. 70.
pub(in crate::abstract_machine) fn floating_constant(
    range: SourceRange,
    value: Real,
    exact: bool,
    spelling: &str,
    operand: Operand,
) -> Event {
    let format = value.format();
    let name = format.spelling();
    let suffix = match spelling.as_bytes().last() {
        | Some(b'f' | b'F') if format == Floating::Float => "its suffix f makes it a float",
        | Some(b'l' | b'L') => "its suffix l makes it a long double",
        | _ => "it has no suffix, so it is a double (f would make it a float, l a long double)",
    };
    let hexadecimal = spelling.starts_with("0x") || spelling.starts_with("0X");
    let shown = value.short();
    let stored = value.exact();
    let (summary, detail) = if exact {
        (
            format!("The constant {spelling} is the {name} {shown}, exactly: the type holds it."),
            format!(
                "Its value, {stored}, is a sum of powers of two that fits in {}'s {} bits of \
                 precision, so the {name} holds it exactly.",
                name,
                precision(format)
            ),
        )
    } else if hexadecimal {
        (
            format!(
                "The constant {spelling} has more bits than a {name} holds, so it is rounded to \
                 the nearest {name}, {shown}."
            ),
            format!(
                "A {name} has {} bits of precision, so the constant is rounded correctly to the \
                 nearest {name}, which is exactly {stored}.",
                precision(format)
            ),
        )
    } else {
        (
            format!(
                "{spelling} has no exact {name}, so the constant is the nearest one: {}.",
                lead_text(&stored)
            ),
            format!(
                "Its decimal value has no exact binary form that fits: a {name} is a binary \
                 fraction with {} significant bits, and {spelling} is not one. So the constant \
                 becomes the {name} nearest to it, which is exactly {stored}. C lets each \
                 implementation choose the nearest value or one of its two neighbours; bcc, like \
                 GCC and Clang, takes the nearest, as IEEE 754 does. printf with %.{}g shows it \
                 as {}.",
                precision(format),
                format.digits() + 2,
                value.printf(b'g', Some(format.digits() as usize + 2))
            ),
        )
    };
    let text = format!(
        "The floating constant {spelling} has type {name}: {suffix}. {detail} Evaluating a \
         constant just produces that value."
    );
    let mut cited = vec![clauses::FLOATING_CONSTANT, clauses::FLOATING_SUFFIX];
    if !exact {
        cited.push(clauses::FLOATING_FORMATS);
    }
    let mut event = explained(
        Event::new(
            EventKind::Constant,
            range,
            format!("Evaluate the constant {spelling} → {shown}"),
        ),
        &summary,
        &text,
        &cited,
    );
    if !exact {
        event = noted(event, Topic::FloatingPoint);
    }
    event.result_type = Some(String::from(name));
    result(event, operand)
}

/// The first digits of an exact decimal, as code, with an ellipsis when it
/// goes on.
fn lead_text(exact: &str) -> String {
    // Set as code, so the digits line up and wrap as one long number.
    if exact.len() <= LEAD_DIGITS {
        return format!("`{exact}`");
    }
    let cut = exact
        .char_indices()
        .nth(LEAD_DIGITS - 1)
        .map_or(exact.len(), |(at, _)| at);
    format!("`{}…`", &exact[..cut])
}

fn precision(format: Floating) -> u32 {
    format.precision()
}

/// How a value of type `from` converted to `to`, giving `converted`, in
/// words, with the rule and the note; `None` when neither type is
/// floating or nothing converted.
pub(in crate::abstract_machine) fn conversion_words(
    from: Arithmetic,
    to: Arithmetic,
    value: &Value,
    converted: &Value,
) -> Option<(String, Vec<Clause>, Topic)> {
    let (a, b) = (from.spelling(), to.spelling());
    let shown = value_text(value);
    let new = value_text(converted);
    match (from, to, value, converted) {
        | (Arithmetic::Integer(_), Arithmetic::Floating(format), &Value::Integer(number), _) => {
            let exact = Real::from_integer(number, format).exact;
            Some(if exact {
                (
                    format!("the {a} {shown} converts to the {b} {new}, exactly"),
                    vec![clauses::INTEGER_TO_FLOATING],
                    Topic::IntegerToFloating,
                )
            } else {
                (
                    format!(
                        "the {a} {shown} has no exact {b}, which holds every integer only up to \
                         2^{}, so it becomes the nearest {b}, {new}",
                        format.precision()
                    ),
                    vec![clauses::INTEGER_TO_FLOATING, clauses::ROUND_TO_NEAREST],
                    Topic::FloatingPoint,
                )
            })
        },
        | (Arithmetic::Floating(_), Arithmetic::Integer(_), Value::Floating(real), _) => {
            let whole = real
                .to_integer(crate::abstract_machine::integers::Integer::LongLong)
                .ok()
                .map(|whole| Real::from_integer(whole, real.format()).value);
            let fraction =
                whole.is_some_and(|whole| whole.compare(*real) != Some(std::cmp::Ordering::Equal));
            Some(if fraction {
                (
                    format!(
                        "the {a} {shown} converts to {b} by discarding its fraction, truncating \
                         toward zero: {new}"
                    ),
                    vec![clauses::FLOATING_TO_INTEGER],
                    Topic::Truncation,
                )
            } else {
                (
                    format!("the {a} {shown} has no fraction, so it converts to the {b} {new}"),
                    vec![clauses::FLOATING_TO_INTEGER],
                    Topic::Truncation,
                )
            })
        },
        | (
            Arithmetic::Floating(source),
            Arithmetic::Floating(target),
            Value::Floating(real),
            Value::Floating(result),
        ) => {
            if source == target {
                return None;
            }
            if target > source {
                return Some((
                    format!("the {a} {shown} converts to {b} with its value unchanged"),
                    vec![clauses::FLOATING_PROMOTION],
                    Topic::FloatingSizes,
                ));
            }
            let rounded = real.convert(target);
            Some(if result.is_infinite() && !real.is_infinite() {
                (
                    format!(
                        "the {a} {shown} is beyond the largest {b}, about {}, so it becomes \
                         {new}: C leaves that undefined, and Annex F, which bcc follows, makes it \
                         an infinity",
                        target.maximum_text()
                    ),
                    vec![clauses::FLOATING_DEMOTION, clauses::FLOATING_OPERATIONS],
                    Topic::FloatingSizes,
                )
            } else if rounded.exact {
                (
                    format!("the {a} {shown} is also a {b}, so it converts exactly"),
                    vec![clauses::FLOATING_DEMOTION],
                    Topic::FloatingSizes,
                )
            } else {
                (
                    format!(
                        "the {a} {shown} has no exact {b}, so it rounds to the nearest {b}, {new} \
                         (exactly {})",
                        lead_text(&result.exact())
                    ),
                    vec![clauses::FLOATING_DEMOTION, clauses::ROUND_TO_NEAREST],
                    Topic::FloatingPoint,
                )
            })
        },
        | _ => None,
    }
}

/// A value as the floating narration writes it.
fn value_text(value: &Value) -> String {
    match value {
        | Value::Integer(integer) => format!("{integer}"),
        | Value::Floating(real) => real.short(),
        | _ => String::from("?"),
    }
}

impl Machine {
    /// The usual arithmetic conversion, with a floating operand, of the
    /// operand at `range` (the left one when `depth` is 1) from `from` to
    /// `to`, since the other operand has type `other`.
    ///
    /// C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57.
    #[expect(
        clippy::too_many_arguments,
        reason = "One conversion's complete observation."
    )]
    pub(in crate::abstract_machine) fn floating_conversion(
        range: SourceRange,
        whole: &str,
        from: Arithmetic,
        to: Arithmetic,
        other: Arithmetic,
        depth: usize,
        value: Value,
        converted: Value,
    ) -> Event {
        let side = if depth == 1 { "left" } else { "right" };
        let (a, b, target) = (from.spelling(), other.spelling(), to.spelling());
        let (shown, new) = (value_text(&value), value_text(&converted));
        let (words, mut cited, topic) = conversion_words(from, to, &value, &converted)
            .unwrap_or_else(|| (String::new(), Vec::new(), Topic::ArithmeticConversion));
        let why = match from {
            | Arithmetic::Integer(_) => format!(
                "with a floating operand the integer promotions do not happen: the {a} operand \
                 converts straight to the floating type, {target}"
            ),
            | Arithmetic::Floating(_) => format!(
                "both are floating, and the narrower one, {a}, converts to the wider, {target}"
            ),
        };
        let summary = format!(
            "`{whole}` mixes {} and {}, so the {side} operand converts to {target}: {shown} \
             becomes {new}.",
            article(a),
            article(b)
        );
        let text = format!(
            "Before the operator can compute, the usual arithmetic conversions bring both \
             operands to one common type. The {side} operand is {} and the other is {}: {why}. \
             Here {words}.",
            article(a),
            article(b)
        );
        cited.insert(0, clauses::USUAL_ARITHMETIC);
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::ArithmeticConversion,
                    range,
                    format!("Convert {shown} to {target} → {new}"),
                ),
                &summary,
                &text,
                &cited,
            ),
            topic,
        );
        event.from_type = Some(String::from(a));
        event.result_type = Some(String::from(target));
        event.operands.push(Operand::Value(value));
        result(event, Operand::Value(converted))
    }

    /// `left operator right` computed in the floating type `format`.
    ///
    /// C99: §F.3 paragraph 1, pp. 445-447; PDF pp. 457-459.
    #[expect(
        clippy::many_single_char_names,
        reason = "The operands and the result, short as the formulas in the text are."
    )]
    pub(in crate::abstract_machine) fn floating_arithmetic(
        range: SourceRange,
        operator: &str,
        format: Floating,
        left: &Value,
        right: &Value,
        output: &Value,
    ) -> Event {
        let name = format.spelling();
        let (a, b, shown) = (value_text(left), value_text(right), value_text(output));
        let (Value::Floating(x), Value::Floating(y), Value::Floating(z)) = (left, right, output)
        else {
            return Event::new(
                EventKind::Arithmetic,
                range,
                format!("Compute {a} {operator} {b}"),
            );
        };
        let (word, clause) = match operator {
            | "+" => ("sum", clauses::ADDITION),
            | "-" => ("difference", clauses::ADDITION),
            | "*" => ("product", clauses::MULTIPLICATION),
            | _ => ("quotient", clauses::DIVISION),
        };
        let rounded = x.arithmetic(operator, *y);
        let mut cited = vec![
            clause,
            clauses::USUAL_ARITHMETIC,
            clauses::FLOATING_OPERATIONS,
        ];
        let mut topic = Topic::FloatingPoint;
        let (summary, mut text) = if operator == "/" && y.is_zero() && !x.is_nan() {
            topic = Topic::AnnexF;
            cited.extend([clauses::ANNEX_F, clauses::IEC_559_MACRO]);
            let what = if z.is_nan() {
                format!(
                    "0 / 0 has no meaningful value, so the result is a NaN, {shown}: not a number"
                )
            } else {
                format!(
                    "dividing a nonzero value by zero gives an infinity of the matching sign, \
                     {shown}"
                )
            };
            (
                format!(
                    "{a} / {b} is {shown}: under Annex F, {what}; C without Annex F leaves it \
                     undefined."
                ),
                format!(
                    "The operands are {}, and the divisor is zero. For integers that is undefined \
                     behavior (C99 §6.5.5p5 says so for every type). But bcc defines \
                     __STDC_IEC_559__, and so follows Annex F, which makes / IEEE 754's division: \
                     {what}. The program goes on, and the value can be printed and tested.",
                    plural_type(name)
                ),
            )
        } else if x.is_nan() || y.is_nan() {
            topic = Topic::NotANumber;
            (
                format!("An operand is a NaN, so {a} {operator} {b} is a NaN too: {shown}."),
                format!(
                    "A NaN, not a number, spreads: every arithmetic operation with a NaN operand \
                     gives a NaN, so {a} {operator} {b} = {shown}."
                ),
            )
        } else if z.is_nan() {
            topic = Topic::NotANumber;
            (
                format!(
                    "{a} {operator} {b} has no meaningful value, so the result is a NaN, {shown}."
                ),
                format!(
                    "IEEE 754 gives the invalid operation {a} {operator} {b} the result NaN, not \
                     a number; x86-64 makes it the default NaN, whose sign bit is set, which \
                     glibc prints as {shown}."
                ),
            )
        } else if rounded.exact {
            (
                format!("{a} {operator} {b} = {shown}, exactly: the {word} is itself a {name}."),
                format!(
                    "Both operands are {}, so the machine computes in {name}, as IEEE 754 does: \
                     the exact {word} of the two values, rounded to the nearest {name}. Here the \
                     exact {word}, {}, is a {name} already, so nothing is lost.",
                    plural_type(name),
                    z.exact()
                ),
            )
        } else {
            cited.push(clauses::ROUND_TO_NEAREST);
            let exact = x.exact_result(operator, *y);
            let shown_exact = exact.as_deref().map_or_else(
                || format!("{a} {operator} {b}"),
                |exact| format!("{a} {operator} {b} is exactly {exact}, which"),
            );
            (
                format!(
                    "No {name} holds the exact {word}{}, so it rounds to the nearest one: {shown}.",
                    exact
                        .as_deref()
                        .map_or_else(String::new, |exact| format!(" {}", lead_text(exact)))
                ),
                format!(
                    "Both operands are {}, so the machine computes in {name}, as IEEE 754 does: \
                     the exact {word} of the two values, rounded to the nearest {name}. \
                     {shown_exact} needs more than the {} bits of precision a {name} has, so the \
                     result is the nearest {name}, exactly {}. That rounding is why a floating \
                     result can differ from the decimal arithmetic you would do by hand.",
                    plural_type(name),
                    format.precision(),
                    z.exact()
                ),
            )
        };
        if format == Floating::Float {
            text.push_str(
                " On x86-64 float arithmetic is done in float itself (FLT_EVAL_METHOD 0), not in \
                 a wider type.",
            );
            cited.push(clauses::EVALUATION_FORMAT);
        }
        text.push_str(UNORDERED);
        cited.push(clauses::EVALUATION_ORDER);
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Arithmetic,
                    range,
                    format!("Compute {a} {operator} {b} = {shown}"),
                ),
                &summary,
                &text,
                &cited,
            ),
            topic,
        );
        event.result_type = Some(String::from(name));
        event
    }

    /// A comparison of two values of the floating type `format`: their
    /// exact values decide it, and a NaN is unordered.
    ///
    /// C99: §6.5.8 paragraph 6, p. 86; PDF p. 98.
    /// C99: §6.5.9 paragraph 3, p. 86; PDF p. 98.
    pub(in crate::abstract_machine) fn floating_comparison(
        range: SourceRange,
        source: &str,
        operator: &str,
        format: Floating,
        left: &Value,
        right: &Value,
        output: &Value,
    ) -> Event {
        let name = format.spelling();
        let (a, b, shown) = (value_text(left), value_text(right), value_text(output));
        let holds = *output == Value::Integer(1);
        let truth = if holds { "holds" } else { "does not hold" };
        let clause = if matches!(operator, "==" | "!=") {
            clauses::EQUALITY
        } else {
            clauses::RELATIONAL
        };
        let mut cited = vec![
            clause,
            clauses::USUAL_ARITHMETIC,
            clauses::FLOATING_OPERATIONS,
        ];
        let (Value::Floating(x), Value::Floating(y)) = (left, right) else {
            return Event::new(
                EventKind::Comparison,
                range,
                format!("Compare {a} {operator} {b}"),
            );
        };
        let written = snippet(source, range);
        let (summary, text, topic) = if x.is_nan() || y.is_nan() {
            (
                format!(
                    "A NaN is unordered with every value, itself included, so `{written}` {truth} \
                     and yields {shown}."
                ),
                format!(
                    "A NaN, not a number, compares unequal to everything: <, <=, >, >= and == are \
                     all false with a NaN operand, and != is true. That holds even when both \
                     sides are the same NaN, which is why x != x is the classic test for a NaN. \
                     So {a} {operator} {b} {truth}, and the result is the int {shown}."
                ),
                Topic::NotANumber,
            )
        } else {
            let (exact_left, exact_right) = (x.exact(), y.exact());
            let same = x.compare(*y) == Some(std::cmp::Ordering::Equal);
            let lead = if same {
                format!(
                    "Both are exactly {}, so `{written}` {truth} and yields {shown}.",
                    lead_text(&exact_left)
                )
            } else {
                format!(
                    "The two {} differ: {} against {}, so `{written}` {truth} and yields {shown}.",
                    plural_type(name),
                    lead_text(&exact_left),
                    lead_text(&exact_right)
                )
            };
            let mut text = format!(
                "A comparison yields an int: 1 when the relation holds and 0 when it does not. It \
                 compares the values the {} actually hold, exactly: the left is {exact_left} and \
                 the right is {exact_right}. So {a} {operator} {b} {truth}, and the result is \
                 {shown}.",
                plural_type(name)
            );
            if !same && x.short().len() + y.short().len() > 24 {
                text.push_str(
                    " Two computations that should give the same number in decimal can round \
                     differently in binary, which is why floating results are compared with a \
                     tolerance, such as fabs(x - y) < 1e-9, rather than with ==.",
                );
            }
            (lead, text, Topic::FloatingPoint)
        };
        cited.push(clauses::EVALUATION_ORDER);
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Comparison,
                    range,
                    format!("Compare {a} {operator} {b} → {shown}"),
                ),
                &summary,
                &format!("{text}{UNORDERED}"),
                &cited,
            ),
            topic,
        );
        event.result_type = Some(String::from("int"));
        event
    }

    /// The default argument promotion of a float passed through `...`: it
    /// arrives as a double with the same value.
    ///
    /// C99: §6.5.2.2 paragraph 6, pp. 71-72; PDF pp. 83-84.
    pub(in crate::abstract_machine) fn float_promotion(
        range: SourceRange,
        value: Value,
        promoted: Value,
    ) -> Event {
        let (shown, new) = (value_text(&value), value_text(&promoted));
        let exact = match &promoted {
            | Value::Floating(real) => real.exact(),
            | _ => String::new(),
        };
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::ArithmeticConversion,
                    range,
                    format!("Promote float {shown} to double for `...`"),
                ),
                &format!(
                    "This argument goes through printf's `...`, so the default argument \
                     promotions apply: the float {shown} arrives as the double {new}, with the \
                     same value."
                ),
                &format!(
                    "printf's prototype, int printf(const char *format, ...), gives a type only \
                     to the format. Every later argument matches the `...`, and C applies the \
                     default argument promotions to it, which turn a float into a double. A \
                     double holds every float value exactly, so the value is unchanged: {exact}. \
                     That is why %f, %e and %g take a double, and why printf has no conversion \
                     for a float. Printed as a double with many digits, a float shows how much \
                     less precision it has: {new}."
                ),
                &[
                    clauses::VARIADIC_ARGUMENTS,
                    clauses::DEFAULT_PROMOTIONS,
                    clauses::FLOATING_PROMOTION,
                ],
            ),
            Topic::VariadicPromotion,
        );
        event.from_type = Some(String::from("float"));
        event.result_type = Some(String::from("double"));
        event.operands.push(Operand::Value(value));
        result(event, Operand::Value(promoted))
    }

    /// A cast that converts to or from a floating type.
    ///
    /// C99: §6.5.4 paragraph 4, p. 81; PDF p. 93.
    pub(in crate::abstract_machine) fn floating_cast(
        range: SourceRange,
        source: &str,
        from: Type,
        to: Type,
        value: Value,
        converted: Value,
    ) -> Option<Event> {
        let (from_arithmetic, to_arithmetic) = (from.arithmetic()?, to.arithmetic()?);
        if from_arithmetic.floating().is_none() && to_arithmetic.floating().is_none() {
            return None;
        }
        let form = snippet(source, range);
        let (a, b) = (from_arithmetic.spelling(), to_arithmetic.spelling());
        let (shown, new) = (value_text(&value), value_text(&converted));
        let (title, summary, text, mut cited, topic) = if from == to {
            (
                format!("Cast to {b}: no conversion, {new}"),
                format!(
                    "`({b})` names the type the value already has, so the cast changes nothing: \
                     {new}."
                ),
                format!(
                    "A cast converts its operand's value to the type it names. The operand is \
                     already {}, so the cast has no effect: {new}.",
                    article(a)
                ),
                vec![clauses::CAST],
                Topic::Cast,
            )
        } else {
            let (words, cited, topic) =
                conversion_words(from_arithmetic, to_arithmetic, &value, &converted)?;
            let summary = match (from_arithmetic, to_arithmetic) {
                | (Arithmetic::Floating(_), Arithmetic::Integer(_)) => format!(
                    "`{form}` discards the fraction of {shown}, truncating toward zero, so the \
                     result is the {b} {new}, not a rounded value."
                ),
                | (Arithmetic::Integer(_), Arithmetic::Floating(_)) => format!(
                    "`{form}` converts the {a} {shown} to the {b} {new}, so the arithmetic it \
                     takes part in is floating."
                ),
                | _ => format!("`{form}` converts the {a} {shown} to {b}: {new}."),
            };
            let text = format!(
                "A cast converts its operand's value to the type it names, here from {a} to {b}: \
                 {words}."
            );
            (
                format!("Cast {shown} to {b} → {new}"),
                summary,
                text,
                cited,
                topic,
            )
        };
        cited.insert(0, clauses::CAST);
        cited.dedup();
        let mut event = noted(
            explained(
                Event::new(EventKind::Cast, range, title),
                &summary,
                &text,
                &cited,
            ),
            topic,
        );
        event.from_type = Some(String::from(a));
        event.result_type = Some(String::from(b));
        event.operands.push(Operand::Value(value));
        Some(result(event, Operand::Value(converted)))
    }
}

/// The plural of a type's name: `doubles`, `long doubles`.
fn plural_type(name: &str) -> String {
    format!("{name}s")
}

/// How a declaration describes a floating type: its size and format, and
/// how precise it is.
pub(in crate::abstract_machine) fn floating_declaration(format: Floating) -> String {
    let padding = if format == Floating::LongDouble {
        ", 10 of them the value and 6 padding"
    } else {
        ""
    };
    format!(
        "{} object ({} bytes on x86-64{padding}: {}, about {} significant decimal digits)",
        article(format.spelling()),
        format.bytes(),
        format.format_name(),
        format.digits()
    )
}

/// The words for `%f`, `%e`, `%g` or `%a`'s precision: what it asked for.
pub(in crate::abstract_machine) fn floating_precision(
    conversion: u8,
    precision: Option<usize>,
) -> String {
    let lower = conversion.to_ascii_lowercase();
    match (lower, precision) {
        | (b'a', None) =>
            String::from("without a precision %a writes every hexadecimal digit the value needs"),
        | (b'a', Some(precision)) => format!(
            "the precision .{precision} writes {precision} hexadecimal digits after the point, \
             rounding the rest"
        ),
        | (b'g', None) => String::from(
            "without a precision %g keeps 6 significant digits and drops trailing zeros",
        ),
        | (b'g', Some(precision)) => format!(
            "the precision .{precision} keeps {} significant digit{}, and trailing zeros are \
             dropped",
            precision.max(1),
            if precision.max(1) == 1 { "" } else { "s" }
        ),
        | (_, None) => String::from(
            "without a precision it writes 6 digits after the point, rounding the exact value to \
             the nearest",
        ),
        | (_, Some(precision)) => format!(
            "the precision .{precision} writes {precision} digit{} after the point, rounding the \
             exact value to the nearest",
            if precision == 1 { "" } else { "s" }
        ),
    }
}

impl Machine {
    /// Unary `+` or `-` of a floating value: `-` flips the sign bit, of a
    /// zero too.
    ///
    /// C99: §6.5.3.3 paragraph 3, p. 79; PDF p. 91.
    pub(in crate::abstract_machine) fn floating_unary(
        range: SourceRange,
        operator: &str,
        format: Floating,
        input: &str,
        shown: &str,
    ) -> Event {
        let name = format.spelling();
        let (title, summary, text, cited) = if operator == "-" {
            let zero = if input.trim_start_matches('-') == "0.0" {
                " A floating zero has a sign of its own: -0.0 compares equal to 0.0, but printf \
                 shows the minus."
            } else {
                ""
            };
            (
                format!("Negate {input} → {shown}"),
                format!("Unary minus flips the sign of the {name}: -({input}) is {shown}."),
                format!(
                    "Unary minus negates its {name} operand by flipping its sign bit, which is \
                     exact: -({input}) is {shown}.{zero}"
                ),
                vec![clauses::UNARY_MINUS, clauses::FLOATING_OPERATIONS],
            )
        } else {
            (
                format!("Unary plus: {shown}"),
                format!("Unary plus leaves the {name} as it is: {shown}."),
                format!("Unary plus yields its operand unchanged: the {name} {shown}."),
                vec![clauses::UNARY_PLUS],
            )
        };
        let mut event = explained(
            Event::new(EventKind::Unary, range, title),
            &summary,
            &text,
            &cited,
        );
        event.result_type = Some(String::from(name));
        event
    }

    /// A store or initialization that converts to, from or between
    /// floating types: the value, its conversion and the stored result.
    #[expect(
        clippy::too_many_arguments,
        reason = "One store's complete observation."
    )]
    pub(in crate::abstract_machine) fn floating_store(
        &self,
        range: SourceRange,
        initializer: bool,
        full_expression: bool,
        (from, to): (Arithmetic, Arithmetic),
        target: usize,
        offset: i32,
        value: Value,
        stored: Value,
    ) -> Option<Event> {
        let (words, mut cited, topic) = conversion_words(from, to, &value, &stored)?;
        let location = self.location(target, offset);
        let (shown, new) = (value_text(&value), value_text(&stored));
        let (kind, title, clause, operation) = if initializer {
            (
                EventKind::Initialize,
                format!("Initialize {location} = {new}"),
                clauses::SCALAR_INITIALIZER,
                "Initialization",
            )
        } else {
            (
                EventKind::Assign,
                format!("Store {new} into {location}"),
                clauses::SIMPLE_ASSIGNMENT,
                "Simple assignment",
            )
        };
        let mut summary = format!("{location} gets {new}: {words}.");
        let mut text = format!(
            "The value to store is the {} {shown}. {operation} converts it to {location}'s type, \
             {}: {words}. So {location} now holds {new}.",
            from.spelling(),
            to.spelling()
        );
        cited.insert(0, clause);
        if full_expression {
            text.push_str(" The full expression ends here, a sequence point.");
            summary.push_str(" That ends the full expression: a sequence point.");
            cited.push(clauses::FULL_EXPRESSION);
        }
        let mut event = noted(
            explained(
                Event::new(kind, range, title),
                &super::sentence_case(&summary),
                &text,
                &cited,
            ),
            topic,
        );
        event.from_type = Some(String::from(from.spelling()));
        event.result_type = Some(String::from(to.spelling()));
        event.sequence_point = full_expression;
        event.operands.extend([
            Operand::Address {
                target,
                offset,
                structure: None,
            },
            Operand::Value(value),
        ]);
        event.objects.push(target);
        Some(result(event, Operand::Value(stored)))
    }
}
