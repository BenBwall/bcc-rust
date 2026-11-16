//! The usual arithmetic conversions as a picture: the arithmetic types laid
//! out by rank, the integer types in a signed and an unsigned row and the
//! floating types after them, with the type an operand had and the type it
//! became marked. A promotion or a conversion step draws it under its lead
//! sentence, so "`-1` converts to unsigned int" is seen to be a move along
//! the ranks, not a rule to memorize.
//!
//! C99: §6.3.1.1 paragraph 2, pp. 42-43; PDF pp. 54-55 (the integer
//! promotions).
//! C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57 (the usual
//! arithmetic conversions).

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
            let caption = if conversion.promotion {
                format!("The integer promotions: {} becomes {}", conversion.from, conversion.to)
            } else {
                format!(
                    "The usual arithmetic conversions: {} becomes {}",
                    conversion.from, conversion.to
                )
            };
            let rule = rule(&conversion, from, to);
            let label = format!("{caption}. {rule}");
            let cell = move |column: usize, unsigned: bool| {
                let mark = match (from == (column, unsigned), to == (column, unsigned)) {
                    | (true, true) => "from, to",
                    | (true, false) => "from",
                    | (false, true) => "to",
                    | (false, false) => "",
                };
                let class = match mark {
                    | "from" => "ranks-cell is-from",
                    | "to" | "from, to" => "ranks-cell is-to",
                    | _ => "ranks-cell",
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

/// The rule that moved the value, in a sentence.
fn rule(conversion: &Conversion, from: (usize, bool), to: (usize, bool)) -> String {
    if conversion.promotion {
        return String::from(
            "Every type left of int is promoted before arithmetic: an int holds all its values, \
             so it becomes an int.",
        );
    }
    if to.0 >= FLOATING && from.0 < FLOATING {
        String::from("A floating type outranks every integer type, so the integer converts to it.")
    } else if to.0 >= FLOATING {
        String::from("The narrower floating type converts to the wider one, which holds its value.")
    } else if from.0 == to.0 && !from.1 && to.1 {
        String::from(
            "The two have the same rank, and then the signed operand converts to the unsigned \
             type, even when its value is negative.",
        )
    } else if from.0 < to.0 && to.1 && !from.1 {
        String::from(
            "The unsigned operand has the higher rank, so the signed one converts to its type.",
        )
    } else if from.0 < to.0 {
        String::from("The operand of lower rank converts to the type of higher rank.")
    } else {
        String::from(
            "The signed type cannot hold every value of the unsigned one, so both become its \
             unsigned version.",
        )
    }
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

    #[test]
    fn says_which_rule_moved_the_value() {
        let conversion = |from: &str, to: &str| {
            let converted = Conversion {
                from:      from.to_owned(),
                to:        to.to_owned(),
                promotion: false,
            };
            rule(
                &converted,
                place(from).expect("a place"),
                place(to).expect("a place"),
            )
        };
        assert!(conversion("int", "unsigned int").contains("same rank"));
        assert!(conversion("int", "double").contains("outranks every integer type"));
        assert!(conversion("int", "long").contains("lower rank"));
        assert!(conversion("unsigned int", "long").contains("lower rank"));
        assert!(conversion("float", "double").contains("wider one"));
        assert!(conversion("int", "unsigned long").contains("higher rank"));
    }
}
