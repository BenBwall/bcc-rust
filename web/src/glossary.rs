//! The words of the C standard the narration uses, each defined in plain
//! language with the paragraph that defines it. The narration marks the
//! first use of each term in a step's text, and the page lists them all in
//! its glossary.

use leptos::prelude::*;

use crate::snapshot::N1256_URL;

/// One defined term.
#[derive(Debug, PartialEq, Eq)]
pub struct Term {
    /// The anchor: the glossary entry is `#term-{id}`.
    pub id:         &'static str,
    pub name:       &'static str,
    /// Spellings the narration uses, matched without regard to case.
    pub phrases:    &'static [&'static str],
    pub definition: &'static str,
    /// The defining paragraph of C99 and its page in N1256.
    pub clause:     &'static str,
    pub page:       u16,
}

impl Term {
    pub fn link(&self) -> String {
        format!("{N1256_URL}#page={}", self.page)
    }
}

/// Every term, in the glossary's order: from what a program is made of to
/// what can go wrong. Longer phrases come before the shorter ones they
/// contain (`lvalue conversion` before `lvalue`).
pub static TERMS: [Term; 22] = [
    Term {
        id:         "abstract-machine",
        name:       "Abstract machine",
        phrases:    &["abstract machine"],
        definition: "The idealized computer the C standard uses to say what a program means: \
                     objects in memory, values read and stored, and rules for when each effect \
                     happens. A compiler may produce any code that behaves the same way.",
        clause:     "5.1.2.3",
        page:       25,
    },
    Term {
        id:         "object",
        name:       "Object",
        phrases:    &["objects", "object"],
        definition: "A region of memory that holds a value of some type: a variable, or one \
                     element of an array.",
        clause:     "3.14",
        page:       17,
    },
    Term {
        id:         "lvalue-conversion",
        name:       "Lvalue conversion",
        phrases:    &["lvalue conversion"],
        definition: "Reading the value stored in the object an lvalue designates. It happens \
                     whenever an lvalue is used for its value rather than as a place.",
        clause:     "6.3.2.1p2",
        page:       58,
    },
    Term {
        id:         "lvalue",
        name:       "Lvalue",
        phrases:    &["lvalue"],
        definition: "An expression that designates an object, a place in memory: a variable's \
                     name, *p or a[i]. The operator around it decides whether the object is read \
                     or written.",
        clause:     "6.3.2.1p1",
        page:       58,
    },
    Term {
        id:         "side-effect",
        name:       "Side effect",
        phrases:    &["side effects", "side effect"],
        definition: "A change to the program's state, such as storing into an object. An \
                     expression can have side effects as well as a value: x = 5 has the value 5 \
                     and the side effect of storing it.",
        clause:     "5.1.2.3p2",
        page:       25,
    },
    Term {
        id:         "sequence-point",
        name:       "Sequence point",
        phrases:    &["sequence point"],
        definition: "A point in execution where every earlier side effect is complete and no \
                     later one has begun: the end of a full expression, after the first operand \
                     of &&, || and ?:, at a comma operator, and just before a function is called. \
                     Modifying an object twice between two of them, or modifying it and reading \
                     it for something else (i + i++), is undefined.",
        clause:     "5.1.2.3p2",
        page:       25,
    },
    Term {
        id:         "full-expression",
        name:       "Full expression",
        phrases:    &["full expression"],
        definition: "An expression that is not part of a larger one: an expression statement, an \
                     initializer, a condition or a return value. Its end is a sequence point.",
        clause:     "6.8p4",
        page:       143,
    },
    Term {
        id:         "storage-duration",
        name:       "Automatic storage duration",
        phrases:    &["automatic storage duration", "storage duration"],
        definition: "How long an object's storage lasts. Local variables not declared static have \
                     automatic storage duration: they exist from entry into their block until the \
                     block ends, and each entry gets fresh ones.",
        clause:     "6.2.4p5",
        page:       44,
    },
    Term {
        id:         "lifetime",
        name:       "Lifetime",
        phrases:    &["lifetimes", "lifetime"],
        definition: "The part of execution during which an object exists and keeps its value. \
                     Using an object, or a pointer to it, after its lifetime has ended is \
                     undefined.",
        clause:     "6.2.4p2",
        page:       44,
    },
    Term {
        id:         "scope",
        name:       "Scope",
        phrases:    &["scope"],
        definition: "The part of the program's text where a name can be used. A name declared \
                     inside a block can be used up to that block's closing brace.",
        clause:     "6.2.1p4",
        page:       41,
    },
    Term {
        id:         "frame",
        name:       "Frame (activation)",
        phrases:    &["activation", "frame"],
        definition: "The objects that one call of a function owns: its parameters and local \
                     variables. Every call, including a recursive one, gets its own. The standard \
                     describes this as each call's own automatic objects.",
        clause:     "6.2.4p5",
        page:       44,
    },
    Term {
        id:         "by-value",
        name:       "Pass by value",
        phrases:    &["by value"],
        definition: "How C passes every argument: the parameter is a new object initialized with \
                     a copy of the argument's value.",
        clause:     "6.5.2.2p4",
        page:       83,
    },
    Term {
        id:         "indeterminate",
        name:       "Indeterminate value",
        phrases:    &["indeterminate"],
        definition: "A value you cannot rely on: either an unspecified value or a trap \
                     representation. Automatic local variables are indeterminate until something \
                     is stored in them; static ones start at zero.",
        clause:     "3.17.2",
        page:       18,
    },
    Term {
        id:         "trap-representation",
        name:       "Trap representation",
        phrases:    &["trap representation"],
        definition: "A bit pattern that is not a value of its type at all. Reading one as that \
                     type is undefined; a character type such as unsigned char may read any byte.",
        clause:     "6.2.6.1p5",
        page:       49,
    },
    Term {
        id:         "integer-promotions",
        name:       "Integer promotions",
        phrases:    &["integer promotions"],
        definition: "The rule that converts values of types narrower than int, such as char, to \
                     int before arithmetic or a comparison.",
        clause:     "6.3.1.1p2",
        page:       54,
    },
    Term {
        id:         "usual-arithmetic-conversions",
        name:       "Usual arithmetic conversions",
        phrases:    &["usual arithmetic conversions"],
        definition: "The rule that converts both operands of an arithmetic operator to one common \
                     type before computing. Two ints need no conversion.",
        clause:     "6.3.1.8p1",
        page:       56,
    },
    Term {
        id:         "array-decay",
        name:       "Array-to-pointer decay",
        phrases:    &["array-to-pointer decay"],
        definition: "Using an array in an expression gives a pointer to its first element; the \
                     array is not copied. sizeof and & are the exceptions.",
        clause:     "6.3.2.1p3",
        page:       58,
    },
    Term {
        id:         "pointer-arithmetic",
        name:       "Pointer arithmetic",
        phrases:    &["pointer arithmetic"],
        definition: "Adding an integer n to a pointer moves it n elements, not n bytes, within \
                     one array.",
        clause:     "6.5.6p8",
        page:       95,
    },
    Term {
        id:         "one-past-the-end",
        name:       "One past the end",
        phrases:    &["one-past-the-end", "one past the end"],
        definition: "The position just after an array's last element. A pointer may point there \
                     and be compared, but must not be dereferenced.",
        clause:     "6.5.6p8",
        page:       95,
    },
    Term {
        id:         "null-pointer",
        name:       "Null pointer",
        phrases:    &["null pointer"],
        definition: "A pointer that points to no object. Converting the integer constant 0 to a \
                     pointer type gives one.",
        clause:     "6.3.2.3p3",
        page:       59,
    },
    Term {
        id:         "undefined-behavior",
        name:       "Undefined behavior",
        phrases:    &["undefined behavior", "undefined"],
        definition: "Behavior the standard places no requirements on, such as signed overflow. \
                     Compilers assume it never happens, so a program that does it can do anything.",
        clause:     "3.4.3",
        page:       16,
    },
    Term {
        id:         "implementation-defined",
        name:       "Implementation-defined",
        phrases:    &["implementation-defined"],
        definition: "Behavior each compiler must choose and document, such as the result of \
                     storing 200 in a char.",
        clause:     "3.4.1",
        page:       15,
    },
];

/// A piece of narration text: plain, or a defined term as written.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Term(&'a str, &'static Term),
}

/// `text` split at the first use of each term not in `seen`, which gains
/// the terms found, so a term is marked once per step. A phrase matches
/// whole words only, ignoring case.
pub fn pieces<'a>(text: &'a str, seen: &mut Vec<&'static str>) -> Vec<Piece<'a>> {
    let lower = text.to_ascii_lowercase();
    let mut found: Vec<(usize, usize, &'static Term)> = Vec::new();
    for term in &TERMS {
        if seen.contains(&term.id) {
            continue;
        }
        let first = term.phrases.iter().find_map(|phrase| {
            lower
                .match_indices(phrase)
                .map(|(start, _)| start)
                .find(|&start| {
                    let end = start + phrase.len();
                    let before = lower[..start].chars().next_back();
                    let after = lower[end..].chars().next();
                    let boundary = |c: Option<char>| {
                        !c.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_')
                    };
                    boundary(before)
                        && boundary(after)
                        && !found.iter().any(|&(s, e, _)| start < e && s < end)
                })
                .map(|start| (start, start + phrase.len()))
        });
        if let Some((start, end)) = first {
            found.push((start, end, term));
            seen.push(term.id);
        }
    }
    found.sort_by_key(|&(start, ..)| start);
    let mut pieces = Vec::new();
    let mut at = 0;
    for (start, end, term) in found {
        if start > at {
            pieces.push(Piece::Text(&text[at..start]));
        }
        pieces.push(Piece::Term(&text[start..end], term));
        at = end;
    }
    if at < text.len() {
        pieces.push(Piece::Text(&text[at..]));
    }
    pieces
}

/// The glossary section: every term with its definition and the paragraph
/// of C99 that defines it.
#[component]
pub fn Glossary() -> impl IntoView {
    view! {
        <section class="glossary" id="glossary" aria-labelledby="glossary-title">
            <div class="section-heading">
                <h2 id="glossary-title"><span class="section-number">"Table 2"</span>"Glossary"</h2>
                <p>"The words of the C standard the explanations use. Each links to the paragraph that defines it."</p>
            </div>
            <dl class="glossary-list">
                {TERMS.iter().map(|term| view! {
                    <div class="glossary-entry" id=format!("term-{}", term.id)>
                        <dt>{term.name}</dt>
                        <dd>
                            {term.definition}" "
                            <a class="glossary-clause" href=term.link() target="_blank" rel="noopener">
                                {format!("C99 §{}", term.clause)}
                                <span class="sr-only">" (opens the standard in a new tab)"</span>
                            </a>
                        </dd>
                    </div>
                }).collect_view()}
            </dl>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Piece,
        TERMS,
        pieces,
    };

    fn marked(text: &str) -> Vec<(&str, &str)> {
        pieces(text, &mut Vec::new())
            .into_iter()
            .filter_map(|piece| match piece {
                | Piece::Term(written, term) => Some((written, term.id)),
                | Piece::Text(_) => None,
            })
            .collect()
    }

    #[test]
    fn marks_the_first_use_of_each_term_on_word_boundaries() {
        assert_eq!(
            marked("Lvalue conversion: an lvalue is read. The lvalue stays."),
            [
                ("Lvalue conversion", "lvalue-conversion"),
                ("lvalue", "lvalue")
            ]
        );
        assert_eq!(
            marked("Its objects end; the object ends."),
            [("objects", "object")]
        );
        // Not inside other words.
        assert_eq!(
            marked("subobjects and scoped and undefinedness"),
            [] as [(&str, &str); 0]
        );
        assert_eq!(
            marked("a pointer one past the end of `a`"),
            [("one past the end", "one-past-the-end")]
        );
    }

    #[test]
    fn keeps_the_text_whole() {
        let text = "The full expression ends here, which is a sequence point.";
        let joined: String = pieces(text, &mut Vec::new())
            .into_iter()
            .map(|piece| match piece {
                | Piece::Text(text) | Piece::Term(text, _) => text,
            })
            .collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn terms_have_unique_anchors_and_pages() {
        for (index, term) in TERMS.iter().enumerate() {
            assert!(
                TERMS[..index].iter().all(|other| other.id != term.id),
                "{}",
                term.id
            );
            assert!((15..=560).contains(&term.page), "{}", term.id);
        }
    }
}
