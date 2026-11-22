//! Enumerations: `enum color { RED, GREEN = 5, BLUE };` declares a type and
//! three named constants. The constants are no objects: each is an integer
//! constant of type `int`, which the compiler puts wherever its name appears,
//! so they take no storage and nothing reads them at run time. The type is
//! compatible with an integer type the implementation chooses; GCC chooses
//! `unsigned int` when no constant is negative, and `int` otherwise, and
//! this machine does the same, so an enumeration's objects compute exactly
//! as GCC's do.
//!
//! C99: §6.4.4.3 paragraph 2, p. 59; PDF p. 71.
//! C99: §6.7.2.2 paragraphs 2-4, p. 105; PDF p. 117.
//! C99: §6.7.2.3 paragraphs 1-3, p. 106; PDF p. 118.
//! C99: §6.2.1 paragraph 7, p. 30; PDF p. 42 (a constant's scope begins
//! just after its enumerator).

use super::{
    clauses,
    expressions::combined,
    integers::Integer,
    lexer::Token,
    model::{
        Diagnostic,
        Enumeration,
        Enumerator,
        MAX_MEMBERS,
        MAX_STRUCTURES,
        Op,
        SourceRange,
        Value,
    },
    parser::{
        Parser,
        is_identifier,
    },
};

/// An enumeration constant in scope: its name, value, and where it was
/// declared.
pub(super) struct Constant<'a> {
    pub name:        &'a str,
    pub value:       i128,
    pub enumeration: usize,
    /// Its index among the enumeration's constants.
    pub index:       usize,
    pub scope:       usize,
}

/// An enumeration tag in scope: `color` in `enum color`.
pub(super) struct Tag<'a> {
    pub name:        &'a str,
    pub enumeration: usize,
    pub scope:       usize,
}

impl<'a> Parser<'a> {
    /// `enum tag { … };` or `enum { … };` on its own, at file or block
    /// scope: it declares the type and its constants and nothing else, so
    /// it creates no object. At file scope its step comes before the static
    /// objects are initialized; in a block, it is a step where execution
    /// reaches it.
    pub(super) fn enumeration_definition(&mut self) -> Result<(), Diagnostic> {
        let start = self.current();
        let id = self.enumeration_specifier(true)?;
        if !self.at(";") {
            return Err(self.error(
                "end the enumeration's definition with `;` and declare its objects separately, as \
                 in `enum color c = RED;`",
            ));
        }
        let end = self.current();
        self.position += 1;
        self.declare_enumeration(id, combined(start.range, end.range))
    }

    /// Record that the declaration at `range` defined enumeration `id`: its
    /// step runs before `main` at file scope, and where execution reaches it
    /// in a block.
    pub(super) fn declare_enumeration(
        &mut self,
        id: usize,
        range: SourceRange,
    ) -> Result<(), Diagnostic> {
        self.enumerations[id].range = range;
        if self.scope == 0 {
            self.file_enumerations.push(id);
            Ok(())
        } else {
            self.emit(Op::Enumeration(id), range)
        }
    }

    /// An enumeration specifier: `enum tag`, naming an enumeration defined
    /// before, or, when `definition` allows it, `enum tag { … }` or `enum {
    /// … }`, which defines one. Returns the enumeration's index.
    pub(super) fn enumeration_specifier(&mut self, definition: bool) -> Result<usize, Diagnostic> {
        let keyword = self.current();
        self.expect("enum")?;
        let tag = is_identifier(self.current().text).then(|| self.current());
        if tag.is_some() {
            self.position += 1;
        }
        if !self.at("{") {
            let Some(tag) = tag else {
                return Err(self.error("expected the enumeration's tag or its list of constants"));
            };
            return self
                .enumeration_tag(tag.text)
                .ok_or_else(|| self.incomplete_enumeration(keyword, tag));
        }
        if !definition {
            return Err(self.error(
                "define an enumeration on its own, as in `enum color { RED, GREEN };`, and then \
                 declare objects of it; a definition inside another declaration is outside this \
                 subset",
            ));
        }
        if let Some(tag) = tag {
            self.tag_conflict(tag)?;
        }
        if self.enumerations.len() >= MAX_STRUCTURES {
            return Err(Diagnostic::limit(
                keyword.range,
                "enumeration type limit reached",
            ));
        }
        let id = self.enumerations.len();
        self.enumerations.push(Enumeration {
            name:      tag.map_or_else(String::new, |tag| format!("enum {}", tag.text)),
            constants: Vec::new(),
            tag:       tag.map(|tag| String::from(tag.text)),
            integer:   Integer::UnsignedInt,
            range:     keyword.range,
            function:  self.function_name.clone(),
        });
        // The tag's scope begins just after it, so the list may already
        // name it (C99 §6.2.1p7), though the type is incomplete until its
        // closing brace (§6.7.2.2p4).
        if let Some(tag) = tag {
            self.enumeration_tags.push(Tag {
                name:        tag.text,
                enumeration: id,
                scope:       self.scope,
            });
        }
        self.enumerator_list(id)?;
        Ok(id)
    }

    /// The constants between `{` and `}`: each a name, optionally `=` and
    /// an integer constant expression whose value an int can hold. Without
    /// one, a constant is the previous one plus 1, or 0 for the first. Each
    /// name is in scope from just after it, so a later constant's value may
    /// use an earlier one.
    fn enumerator_list(&mut self, id: usize) -> Result<(), Diagnostic> {
        self.expect("{")?;
        let mut next: i128 = 0;
        loop {
            if self.at("}") {
                if self.enumerations[id].constants.is_empty() {
                    return Err(Diagnostic::new(
                        "syntax",
                        self.current().range,
                        "an enumeration lists at least one constant (C99 §6.7.2.2p1)",
                    ));
                }
                break;
            }
            let name = self.current();
            if !is_identifier(name.text) {
                return Err(self.error("expected the name of an enumeration constant"));
            }
            self.ordinary_conflict(name, "an enumeration constant")?;
            self.position += 1;
            let (value, written) = if self.take("=") {
                let expression = self.expression(0)?;
                let range = self.expressions[expression].range;
                let value = self.constant_integer(
                    expression,
                    "the value of an enumeration constant",
                    &clauses::ENUMERATOR_CONSTRAINT,
                )?;
                let written = self.source.get(range.start..range.end).unwrap_or_default();
                if i32::try_from(value).is_err() {
                    return Err(Diagnostic::constraint(
                        range,
                        &format!(
                            "`{written}` is {value}, which an int cannot hold: an enumeration \
                             constant has type int, so its value must fit in one, -2147483648 to \
                             2147483647 (C99 §6.7.2.2p2)"
                        ),
                        &clauses::ENUMERATOR_CONSTRAINT,
                    ));
                }
                (value, Some(String::from(written)))
            } else {
                if i32::try_from(next).is_err() {
                    return Err(Diagnostic::constraint(
                        name.range,
                        &format!(
                            "`{}` would be the previous constant plus 1, {next}, which an int \
                             cannot hold; give it a value with = (C99 §6.7.2.2p2, p3)",
                            name.text
                        ),
                        &clauses::ENUMERATOR_CONSTRAINT,
                    ));
                }
                (next, None)
            };
            let enumeration = &mut self.enumerations[id];
            if enumeration.constants.len() >= MAX_MEMBERS {
                return Err(Diagnostic::limit(
                    name.range,
                    "enumeration constant limit reached",
                ));
            }
            enumeration.constants.push(Enumerator {
                name: String::from(name.text),
                value,
                range: name.range,
                written,
            });
            let constant = enumeration.constants.len() - 1;
            self.constants.push(Constant {
                name: name.text,
                value,
                enumeration: id,
                index: constant,
                scope: self.scope,
            });
            next = value + 1;
            if !self.take(",") {
                break;
            }
        }
        self.expect("}")?;
        // GCC's choice of compatible type: unsigned int unless a constant
        // is negative.
        let enumeration = &mut self.enumerations[id];
        enumeration.integer = if enumeration
            .constants
            .iter()
            .any(|constant| constant.value < 0)
        {
            Integer::Int
        } else {
            Integer::UnsignedInt
        };
        Ok(())
    }

    /// The enumeration tagged `tag`, innermost first, if one is in scope
    /// and complete.
    pub(super) fn enumeration_tag(&self, tag: &str) -> Option<usize> {
        self.enumeration_tags
            .iter()
            .rev()
            .find(|known| known.name == tag)
            .map(|known| known.enumeration)
            .filter(|&id| !self.enumerations[id].constants.is_empty())
    }

    /// The enumeration constant named `name`, if one is in scope.
    pub(super) fn constant(&self, name: &str) -> Option<&Constant<'a>> {
        self.constants.iter().rev().find(|known| known.name == name)
    }

    /// The error for `enum tag` naming no enumeration defined so far.
    fn incomplete_enumeration(&self, keyword: Token<'_>, tag: Token<'_>) -> Diagnostic {
        let range = combined(keyword.range, tag.range);
        if self
            .enumeration_tags
            .iter()
            .any(|known| known.name == tag.text)
        {
            return Diagnostic::constraint(
                range,
                &format!(
                    "enum {} is still being defined here: it is incomplete until the closing \
                     brace of its list, so its own constants cannot use it (C99 §6.7.2.2p4, \
                     §6.7.2.3p3)",
                    tag.text
                ),
                &clauses::ENUMERATION_COMPLETE,
            );
        }
        if self
            .structures
            .iter()
            .any(|structure| structure.tagged && structure.name == tag.text)
        {
            return Diagnostic::constraint(
                range,
                &format!(
                    "{} is the tag of a structure, struct {}, not of an enumeration: one tag \
                     names one kind of type (C99 §6.7.2.3p2)",
                    tag.text, tag.text
                ),
                &clauses::TAG_KIND,
            );
        }
        Diagnostic::constraint(
            range,
            &format!(
                "no enumeration tagged {} is defined before this point: `enum tag` without its \
                 list of constants may name only an enumeration already defined (C99 §6.7.2.3p3)",
                tag.text
            ),
            &clauses::ENUMERATION_COMPLETE,
        )
    }

    /// A new enumeration tag may not be defined twice in one scope, nor
    /// reuse a structure's tag: tags share one name space (C99 §6.2.3p1,
    /// §6.7.2.3p1-2).
    fn tag_conflict(&self, tag: Token<'_>) -> Result<(), Diagnostic> {
        if self
            .structures
            .iter()
            .any(|structure| structure.tagged && structure.name == tag.text)
        {
            return Err(Diagnostic::constraint(
                tag.range,
                &format!(
                    "{} is already the tag of a structure: structure and enumeration tags share \
                     one name space, and one tag names one kind of type (C99 §6.2.3p1, §6.7.2.3p2)",
                    tag.text
                ),
                &clauses::TAG_KIND,
            ));
        }
        if let Some(known) = self
            .enumeration_tags
            .iter()
            .rev()
            .find(|known| known.name == tag.text)
        {
            if known.scope == self.scope {
                return Err(Diagnostic::constraint(
                    tag.range,
                    &format!(
                        "enum {} is already defined in this scope: a type's contents may be \
                         defined only once (C99 §6.7.2.3p1)",
                        tag.text
                    ),
                    &clauses::TAG_DEFINED_ONCE,
                ));
            }
            return Err(Diagnostic::new(
                "unsupported",
                tag.range,
                &format!(
                    "enum {} is already defined in an enclosing scope; defining another \
                     enumeration with the same tag inside it is outside this subset",
                    tag.text
                ),
            ));
        }
        Ok(())
    }

    /// The value of the integer constant expression `expression`, which
    /// `what` must be, or why it is not one.
    ///
    /// C99: §6.6 paragraph 6, p. 95; PDF p. 107.
    pub(super) fn constant_integer(
        &self,
        expression: usize,
        what: &str,
        clause: &'static clauses::Clause,
    ) -> Result<i128, Diagnostic> {
        let node = &self.expressions[expression];
        let range = node.range;
        let written = self.source.get(range.start..range.end).unwrap_or_default();
        let refused = |reason: &str| {
            Diagnostic::constraint(
                range,
                &format!(
                    "{what} must be an integer constant expression, made of integer, character \
                     and enumeration constants and sizeof joined by operators (C99 §6.6p6); \
                     `{written}` is not one{reason}"
                ),
                clause,
            )
        };
        if !self.is_integer_constant_expression(expression) {
            let reason = match self.constant_value(expression) {
                | Err(error) => error
                    .message
                    .split_once(": ")
                    .map_or_else(String::new, |(_, why)| format!(": {why}")),
                | Ok(_) => String::new(),
            };
            return Err(refused(&reason));
        }
        match self.constant_value(expression) {
            | Ok((Value::Integer(value), _)) => Ok(value),
            | Ok(_) => Err(refused("")),
            | Err(error) => Err(error),
        }
    }
}
