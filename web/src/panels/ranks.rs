//! The usual arithmetic conversions as a picture: the arithmetic types laid
//! out by rank, the integer types in a signed and an unsigned row and the
//! floating types after them, with the type an operand had, the other
//! operand's type and the type it became marked. A promotion or a
//! conversion step draws it under its lead sentence, so "`-1` converts to
//! unsigned int" is seen to be a move along the ranks, decided by both
//! operands, not a rule to memorize.
//!
//! C99: §6.3.1.1 paragraph 1, p. 42; PDF p. 54 (integer conversion rank).
//! C99: §6.3.1.1 paragraph 2, pp. 42-43; PDF pp. 54-55 (the integer
//! promotions).
//! C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57 (the usual
//! arithmetic conversions).
//! C99: §6.5.2.2 paragraph 6, pp. 71-72; PDF pp. 83-84 (the default
//! argument promotions).

use leptos::prelude::*;

use crate::snapshot::{
    Conversion,
    StepEvent,
};

/// The columns, lowest rank first: the five integer ranks (§6.3.1.1p1),
/// then the floating types, which outrank every integer type.
const COLUMNS: [&str; 8] = [
    "char",
    "short",
    "int",
    "long",
    "long long",
    "float",
    "double",
    "long double",
];

/// The first floating column.
const FLOATING: usize = 5;

#[component]
pub fn Ranks(event: Memo<Option<StepEvent>>) -> impl IntoView {
    let conversion = Memo::new(move |_| {
        event.with(|event| event.as_ref().and_then(|event| event.conversion.clone()))
    });
    move || {
        conversion.get().and_then(|conversion| {
            let from = place(&conversion.from)?;
            let to = place(&conversion.to)?;
            let other = match &conversion.other {
                | Some(other) => Some(place(other)?),
                | None => None,
            };
            let rule = rule(&conversion)?;
            let caption = match (&conversion.other, conversion.promotion) {
                | (_, true) => format!(
                    "The integer promotions: {} becomes {}",
                    conversion.from, conversion.to
                ),
                | (Some(other), false) => format!(
                    "The usual arithmetic conversions: {}, meeting {other}, becomes {}",
                    conversion.from, conversion.to
                ),
                | (None, false) => format!(
                    "The default argument promotions: {} becomes {}",
                    conversion.from, conversion.to
                ),
            };
            let label = format!("{caption}. {rule}");
            let cell = move |column: usize, unsigned: bool| {
                let here = Some((column, unsigned));
                let roles = [
                    (Some(from) == here, "from"),
                    (other == here, "other"),
                    (Some(to) == here, "to"),
                ];
                let mark = roles
                    .iter()
                    .filter(|(marked, _)| *marked)
                    .map(|(_, role)| *role)
                    .collect::<Vec<_>>()
                    .join(", ");
                let class = if Some(to) == here {
                    "ranks-cell is-to"
                } else if Some(from) == here {
                    "ranks-cell is-from"
                } else if other == here {
                    "ranks-cell is-other"
                } else {
                    "ranks-cell"
                };
                let row = if column >= FLOATING {
                    "2 / span 2".to_owned()
                } else if unsigned {
                    "3".to_owned()
                } else {
                    "2".to_owned()
                };
                view! {
                    <span class=class style=format!("grid-column: {}; grid-row: {row}", column + 2)>
                        {mark}
                    </span>
                }
            };
            Some(view! {
                <figure class="ranks" aria-label=label>
                    <figcaption>{caption}</figcaption>
                    <div class="ranks-grid" aria-hidden="true">
                        {COLUMNS.iter().enumerate().map(|(column, name)| view! {
                            <span
                                class={if column >= FLOATING { "ranks-type is-floating" } else { "ranks-type" }}
                                style=format!("grid-column: {}; grid-row: 1", column + 2)
                            >
                                {*name}
                            </span>
                        }).collect_view()}
                        <span class="ranks-row" style="grid-column: 1; grid-row: 2">"signed"</span>
                        <span class="ranks-row" style="grid-column: 1; grid-row: 3">"unsigned"</span>
                        {(0..FLOATING).map(|column| cell(column, false)).collect_view()}
                        {(0..FLOATING).map(|column| cell(column, true)).collect_view()}
                        {(FLOATING..COLUMNS.len()).map(|column| cell(column, false)).collect_view()}
                    </div>
                    <p class="ranks-rule" aria-hidden="true">{rule}</p>
                </figure>
            })
        })
    }
}

/// Where a type sits: its column, and whether it is in the unsigned row.
/// Plain char is signed on x86-64 (§6.2.5p15); a floating type has one
/// cell, in the signed row's place. Types the picture has no place for,
/// such as `_Bool`, give `None`.
fn place(spelling: &str) -> Option<(usize, bool)> {
    let (unsigned, base) = match spelling.strip_prefix("unsigned ") {
        | Some(base) => (true, base),
        | None => (false, spelling.strip_prefix("signed ").unwrap_or(spelling)),
    };
    let base = base.strip_suffix(" int").unwrap_or(base);
    if spelling == "size_t" {
        return Some((3, true));
    }
    let column = COLUMNS.iter().position(|name| *name == base)?;
    (!unsigned || column < FLOATING).then_some((column, unsigned))
}

/// The rule that moved the value, in a sentence: for a usual arithmetic
/// conversion, the branch of §6.3.1.8p1 the two operands' types select.
/// The result type tells the last two integer branches apart: whether the
/// signed type of higher rank holds every value of the unsigned one is a
/// question of widths, which the engine answered. `None` when the types
/// fit no branch, so the picture is left out rather than wrong.
pub(crate) fn rule(conversion: &Conversion) -> Option<String> {
    let (from, to) = (conversion.from.as_str(), conversion.to.as_str());
    if conversion.promotion {
        return Some(format!(
            "On x86-64 Linux an int holds every {from} value, so the integer promotions make it \
             an int."
        ));
    }
    let Some(other) = conversion.other.as_deref() else {
        return Some(format!(
            "A {from} passed to the ... of a function such as printf gets the default argument \
             promotions and becomes a {to}, which holds every {from} value."
        ));
    };
    let (mine, theirs, result) = (place(from)?, place(other)?, place(to)?);
    if theirs.0 >= FLOATING {
        if mine.0 < FLOATING {
            return (result == theirs).then(|| {
                format!(
                    "The other operand is floating, so the integer {from} converts to its type, \
                     {other}: C puts every floating type above every integer type."
                )
            });
        }
        return (mine.0 < theirs.0 && result == theirs).then(|| {
            format!(
                "Both are floating, and {other} is the wider, so the {from} converts to it; it \
                 holds every {from} value."
            )
        });
    }
    if mine.0 >= FLOATING {
        return None;
    }
    let (signed, unsigned) = (!mine.1, mine.1);
    let sentence = if mine.1 == theirs.1 {
        (mine.0 < theirs.0 && result == theirs).then(|| {
            format!(
                "Both are {}, and {other} has the higher rank, so {from} converts to it.",
                if signed { "signed" } else { "unsigned" }
            )
        })?
    } else if signed && theirs.0 == mine.0 && result == theirs {
        format!(
            "{from} and {other} have the same rank, so the signed operand converts to the \
             unsigned type, {other}, even when its value is negative."
        )
    } else if signed && theirs.0 > mine.0 && result == theirs {
        format!(
            "{other} is unsigned and has the higher rank, so the signed {from} converts to it, \
             even when its value is negative."
        )
    } else if signed && mine.0 > theirs.0 && result == (mine.0, true) {
        format!(
            "{from} has the higher rank, but on x86-64 Linux it cannot hold every {other} value, \
             so both operands convert to {to}, the unsigned type of {from}'s rank."
        )
    } else if unsigned && theirs.0 > mine.0 && result == theirs {
        format!(
            "{other} is signed, has the higher rank and on x86-64 Linux holds every {from} value, \
             so the {from} converts to it."
        )
    } else if unsigned && theirs.0 > mine.0 && result == (theirs.0, true) {
        format!(
            "{other} has the higher rank, but on x86-64 Linux it cannot hold every {from} value, \
             so both operands convert to {to}, the unsigned type of {other}'s rank."
        )
    } else {
        return None;
    };
    Some(sentence)
}

#[cfg(test)]
mod tests {
    use super::{
        Conversion,
        place,
        rule,
    };

    #[test]
    fn places_each_arithmetic_type_by_rank_and_signedness() {
        assert_eq!(place("char"), Some((0, false)));
        assert_eq!(place("unsigned char"), Some((0, true)));
        assert_eq!(place("short"), Some((1, false)));
        assert_eq!(place("unsigned int"), Some((2, true)));
        assert_eq!(place("long long"), Some((4, false)));
        assert_eq!(place("unsigned long long"), Some((4, true)));
        assert_eq!(place("size_t"), Some((3, true)));
        assert_eq!(place("double"), Some((6, false)));
        assert_eq!(place("long double"), Some((7, false)));
        assert_eq!(place("_Bool"), None);
    }

    /// The rule for the operand of type `from` that met one of type
    /// `other` and became `to`.
    fn converted(from: &str, other: &str, to: &str) -> String {
        rule(&Conversion {
            from:      from.to_owned(),
            to:        to.to_owned(),
            other:     Some(other.to_owned()),
            promotion: false,
        })
        .unwrap_or_else(|| panic!("a rule for {from} meeting {other}"))
    }

    /// Each branch of §6.3.1.8p1 from the side of each operand that
    /// converts: the integer branches after the integer promotions, then
    /// the floating ones.
    #[test]
    fn states_the_rule_from_both_operands() {
        // Both signed, or both unsigned: the lower rank converts.
        assert_eq!(
            converted("int", "long", "long"),
            "Both are signed, and long has the higher rank, so int converts to it."
        );
        assert_eq!(
            converted("unsigned int", "unsigned long long", "unsigned long long"),
            "Both are unsigned, and unsigned long long has the higher rank, so unsigned int \
             converts to it."
        );
        // The unsigned rank is at least the signed one: the signed converts.
        assert_eq!(
            converted("int", "unsigned int", "unsigned int"),
            "int and unsigned int have the same rank, so the signed operand converts to the \
             unsigned type, unsigned int, even when its value is negative."
        );
        assert_eq!(
            converted("long", "unsigned long long", "unsigned long long"),
            "unsigned long long is unsigned and has the higher rank, so the signed long converts \
             to it, even when its value is negative."
        );
        // The signed type holds every value of the unsigned one.
        assert_eq!(
            converted("unsigned int", "long", "long"),
            "long is signed, has the higher rank and on x86-64 Linux holds every unsigned int \
             value, so the unsigned int converts to it."
        );
        // It does not: both become the unsigned type of the signed one's
        // rank, seen from either operand.
        let both = "long long has the higher rank, but on x86-64 Linux it cannot hold every \
                    unsigned long value, so both operands convert to unsigned long long, the \
                    unsigned type of long long's rank.";
        assert_eq!(
            converted("long long", "unsigned long", "unsigned long long"),
            both
        );
        assert_eq!(
            converted("unsigned long", "long long", "unsigned long long"),
            both
        );
        // Floating operands.
        assert!(converted("int", "double", "double").contains("the integer int converts"));
        assert!(
            converted("float", "long double", "long double").contains("long double is the wider")
        );
    }

    #[test]
    fn leaves_out_a_rule_the_types_do_not_select() {
        let rule_for = |from: &str, other: &str, to: &str| {
            rule(&Conversion {
                from:      from.to_owned(),
                to:        to.to_owned(),
                other:     Some(other.to_owned()),
                promotion: false,
            })
        };
        // The higher rank never converts, and the result is the common type.
        assert_eq!(rule_for("long", "int", "long"), None);
        assert_eq!(rule_for("long long", "unsigned long", "long long"), None);
        assert_eq!(rule_for("double", "int", "double"), None);
        assert_eq!(rule_for("int", "_Bool", "int"), None);
    }

    #[test]
    fn says_which_promotion_moved_a_value_on_its_own() {
        let promoted = |from: &str, to: &str, promotion: bool| {
            rule(&Conversion {
                from: from.to_owned(),
                to: to.to_owned(),
                other: None,
                promotion,
            })
            .expect("a rule")
        };
        assert!(promoted("char", "int", true).contains("integer promotions make it an int"));
        assert!(promoted("float", "double", false).contains("default argument promotions"));
    }
}
