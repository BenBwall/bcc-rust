//! Under the head: the objects at the playhead as a strip of cells, one run
//! per active call, innermost first, then the objects with static storage
//! duration, the way a tape passes under a read/write head. An array is a
//! run of element cells, a structure a run of its members' cells with a
//! padding cell wherever the layout leaves unused bytes. The cell the step
//! wrote is lit, the one it read underlined, the one it only designated
//! outlined; an element a pointer points at names that pointer. The objects of
//! returned calls fold into one cell.
//!
//! A string literal's array, or any char array of more than eight bytes, is
//! one wide cell instead: its characters in a row, one column per byte with
//! the null character boxed as a byte of its own, over an index ruler, the
//! bytes the step used marked in place. It wraps onto more rows before it
//! outgrows a phone's strip, so every byte a step uses is in view with the
//! rest of the string, and a key opens it into its byte cells. An array of
//! more than eight elements of another type folds the same way, into one
//! cell of its values, a column per element, two rows high and scrolled to
//! the elements the step used. On a phone
//! each call's cells are a row of their own, which follows the cells the
//! step used as the strip does on a wide screen.

use leptos::{
    html,
    prelude::*,
};

use super::tape::value_text;
use crate::{
    palette,
    snapshot::{
        Object,
        Snapshot,
        Value,
    },
};

#[component]
pub fn Cells(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    // The strings the reader opened into their byte cells, by object and
    // name, kept from step to step.
    let opened = RwSignal::new(Vec::<(usize, String)>::new());
    // The string whose key the reader just pressed: the key is drawn anew
    // with the strip, so the focus moves to its counterpart, and the change
    // is said aloud.
    let toggle = Toggle {
        opened,
        focus: StoredValue::new(None),
        announcement: RwSignal::new(String::new()),
    };
    // The strip is drawn anew at each step, at its start. Where it is wider
    // than the window (or, on a phone, where a call's row is), bring the
    // cells the step used into the middle, or, when they are wider than the
    // window too, the first of them to its left edge. A folded array's
    // values scroll within their cell, two rows high, to the elements the
    // step used (follow_down).
    let section = NodeRef::<html::Section>::new();
    Effect::new(move || {
        snapshot.track();
        opened.track();
        let Some(section) = section.get() else {
            return;
        };
        if let Ok(folded) = section.query_selector_all(".cell.is-values .string-bytes") {
            for index in 0..folded.length() {
                if let Some(values) = element(folded.item(index)) {
                    follow_down(&values);
                }
            }
        }
        let Ok(rows) = section.query_selector_all(".cells, .cell-run") else {
            return;
        };
        for index in 0..rows.length() {
            if let Some(row) = element(rows.item(index)) {
                follow(&row);
            }
        }
        if let Some(key) = toggle.focus.get_value() {
            toggle.focus.set_value(None);
            refocus(&section, &key);
        }
    });
    view! {
        <section class="head-cells" aria-labelledby="cells-title" node_ref=section>
            <h2 class="module-caption" id="cells-title">
                <b>"Under the head"</b>" the objects at the playhead, by call"
            </h2>
            <p class="sr-only" role="status" aria-live="polite" aria-atomic="true">{toggle.announcement}</p>
            {move || snapshot.with(|snapshot| match snapshot {
                | Some(snapshot) if !snapshot.objects.is_empty() => strip(snapshot, toggle).into_any(),
                // A run that stopped before its first step (at a directive
                // the machine does not run) will put nothing here.
                | Some(snapshot) if snapshot.step == 0 && !snapshot.can_step() => view! {
                    <p class="cells-empty">"The machine stopped before its first step, so no object began its lifetime."</p>
                }.into_any(),
                | Some(_) => view! { <p class="cells-empty">"No objects yet. Entering a block puts the objects it declares on the tape."</p> }.into_any(),
                | None => view! { <p class="cells-empty">"The objects of the run appear here as cells, under the playhead."</p> }.into_any(),
            })}
        </section>
    }
}

/// Scrolls `row` sideways so the cells under it that the step used are in
/// view, when it is too narrow to show all of its cells.
fn follow(row: &web_sys::Element) {
    if row.scroll_width() <= row.client_width() {
        return;
    }
    let Ok(lit) = row.query_selector_all("[data-access]") else {
        return;
    };
    let mut span: Option<(f64, f64)> = None;
    for index in 0..lit.length() {
        let Some(cell) = element(lit.item(index)) else {
            continue;
        };
        let cell = cell.get_bounding_client_rect();
        span = Some(span.map_or((cell.left(), cell.right()), |(left, right)| {
            (left.min(cell.left()), right.max(cell.right()))
        }));
    }
    let Some((left, right)) = span else {
        return;
    };
    let offset = left - row.get_bounding_client_rect().left();
    let room = f64::from(row.client_width());
    let left = f64::from(row.scroll_left()) + offset - ((room - (right - left)) / 2.0).max(8.0);
    let options = web_sys::ScrollToOptions::new();
    options.set_left(left.max(0.0));
    row.scroll_to_with_scroll_to_options(&options);
}

/// Scrolls the folded array's `values` up or down so the elements under it
/// that the step used are in view, when it holds more rows than it shows:
/// on its last row, after the row before them, or, when they take more rows
/// than it shows, the first of them on its first row.
fn follow_down(values: &web_sys::Element) {
    if values.scroll_height() <= values.client_height() {
        return;
    }
    let Ok(lit) = values.query_selector_all("[data-access]") else {
        return;
    };
    let mut span: Option<(f64, f64)> = None;
    for index in 0..lit.length() {
        let Some(element) = element(lit.item(index)) else {
            continue;
        };
        let element = element.get_bounding_client_rect();
        span = Some(
            span.map_or((element.top(), element.bottom()), |(top, bottom)| {
                (top.min(element.top()), bottom.max(element.bottom()))
            }),
        );
    }
    let Some((top, bottom)) = span else {
        return;
    };
    let offset = top - values.get_bounding_client_rect().top();
    let room = f64::from(values.client_height());
    let span = bottom - top;
    let shift = if span > room {
        offset
    } else {
        offset + span - room
    };
    let options = web_sys::ScrollToOptions::new();
    options.set_top((f64::from(values.scroll_top()) + shift).max(0.0));
    values.scroll_to_with_scroll_to_options(&options);
}

fn element(node: Option<web_sys::Node>) -> Option<web_sys::Element> {
    wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(node?).ok()
}

/// What the keys of a folded cell (a string's `BYTES` and `TEXT`, a wide
/// array's `CELLS` and `VALUES`) change: which of them are open,
/// where the focus goes once the strip is drawn again, and what a screen
/// reader hears.
#[derive(Clone, Copy)]
struct Toggle {
    opened:       RwSignal<Vec<(usize, String)>>,
    /// The [`string_key`] of the key to focus after the strip is redrawn.
    focus:        StoredValue<Option<String>>,
    announcement: RwSignal<String>,
}

impl Toggle {
    /// Opens the string or wide array `key` into its `count` cells, or folds
    /// it back; `values` for an array of values rather than characters.
    fn set(self, key: &(usize, String), open: bool, count: usize, values: bool) {
        self.opened.update(|opened| {
            opened.retain(|known| known != key);
            if open {
                opened.push(key.clone());
            }
        });
        self.focus.set_value(Some(string_key(key)));
        self.announcement
            .set(toggle_announcement(&key.1, open, count, values));
    }
}

/// What the string's keys carry as `data-string`: the object's id and
/// name, the same on the `BYTES` key and on the `TEXT` key that replaces it.
fn string_key((id, name): &(usize, String)) -> String {
    format!("{id}:{name}")
}

/// What a screen reader says when the string or wide array `name` opens or
/// folds.
fn toggle_announcement(name: &str, open: bool, count: usize, values: bool) -> String {
    let name = shorten(name, 24);
    match (open, values) {
        | (true, false) => format!("{name} opened into its {count} byte cells"),
        | (true, true) => format!("{name} opened into its {count} element cells"),
        | (false, false) => format!("{name} shown as one cell of its characters"),
        | (false, true) => format!("{name} shown as one cell of its values"),
    }
}

/// Moves the focus to the string key under `section` whose `data-string`
/// is `key`, as a press of the key it replaced left it.
fn refocus(section: &web_sys::Element, key: &str) {
    let Ok(keys) = section.query_selector_all(".cell-open") else {
        return;
    };
    for index in 0..keys.length() {
        let Some(button) = element(keys.item(index)) else {
            continue;
        };
        if button.get_attribute("data-string").as_deref() == Some(key) {
            if let Ok(button) = wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(button) {
                _ = button.focus();
            }
            return;
        }
    }
}

/// One cell of the strip: a scalar, or the padding after a member.
enum Part {
    Cell {
        value: Value,
        /// How the source names it: `total`, `values[1]`, `p.x`.
        name:  String,
        ty:    String,
        /// Its scalar index in the object, for an array or a structure.
        leaf:  Option<usize>,
        /// Its size in bytes on x86-64 Linux.
        bytes: usize,
    },
    Padding {
        bytes: usize,
        /// The member it follows.
        after: String,
    },
    /// A char array as one cell: each byte's character as a constant spells
    /// it without its quotes (`A`, `\n`, `\0`), or `None` while
    /// indeterminate. Or a wide array of other scalars as one cell: each
    /// element's value as the cells write it.
    Text {
        glyphs: Vec<Option<String>>,
        ty:     String,
        /// Whether the glyphs are an array's values, not a string's
        /// characters.
        values: bool,
    },
    /// The key before an opened string's or wide array's cells that folds
    /// them back.
    TextHead {
        ty:     String,
        /// How many cells it opened into.
        count:  usize,
        values: bool,
    },
}

/// A char array of up to this many bytes keeps a cell per byte, as the
/// hello lesson's `name` does; a longer one reads as its text, one cell.
const BYTE_CELLS: usize = 8;

/// The cells of `object`: one per scalar, in memory order, with a padding
/// cell after each structure member that has padding; or, for a string or a
/// wide array the reader has not opened, one cell of its characters or
/// values.
fn parts(object: &Object, open: bool) -> Vec<Part> {
    let mut parts = Vec::new();
    let ty = element_type(&object.ty);
    let folded = text(object)
        .map(|glyphs| (glyphs, false))
        .or_else(|| values(object).map(|glyphs| (glyphs, true)));
    if let Some((glyphs, values)) = folded {
        if !open {
            return vec![Part::Text {
                glyphs,
                ty: object.ty.clone(),
                values,
            }];
        }
        parts.push(Part::TextHead {
            ty: object.ty.clone(),
            count: glyphs.len(),
            values,
        });
        // A literal's bytes are named by their index alone: its spelling
        // heads the run, and would make every cell as wide as the string.
        let name = if object.literal { "" } else { &object.name };
        collect(&object.value, name, &ty, object.size, &mut 0, &mut parts);
        return parts;
    }
    match &object.value {
        | Value::Array { .. } | Value::Struct { .. } => collect(
            &object.value,
            &object.name,
            &ty,
            object.size,
            &mut 0,
            &mut parts,
        ),
        | value => parts.push(Part::Cell {
            value: value.clone(),
            name: object.name.clone(),
            ty,
            leaf: None,
            bytes: object.size,
        }),
    }
    parts
}

/// The cells of `value`, which takes `size` bytes.
fn collect(
    value: &Value,
    name: &str,
    ty: &str,
    size: usize,
    leaf: &mut usize,
    parts: &mut Vec<Part>,
) {
    match value {
        | Value::Array { elements } =>
            for (index, element) in elements.iter().enumerate() {
                let each = size / elements.len().max(1);
                collect(element, &format!("{name}[{index}]"), ty, each, leaf, parts);
            },
        | Value::Struct { members, .. } =>
            for member in members {
                let path = format!("{name}.{}", member.name);
                let ty = element_type(&member.ty);
                collect(&member.value, &path, &ty, member.size, leaf, parts);
                if member.padding > 0 {
                    parts.push(Part::Padding {
                        bytes: member.padding,
                        after: path,
                    });
                }
            },
        | scalar => {
            parts.push(Part::Cell {
                value: scalar.clone(),
                name:  name.to_owned(),
                ty:    ty.to_owned(),
                leaf:  Some(*leaf),
                bytes: size,
            });
            *leaf += 1;
        },
    }
}

/// The type of an element of the array type spelled `ty`, which is the
/// type without its length: `int` for `int[3]`, `char *` for `char *[2]`,
/// and `int (*)(int)` for a table of pointers to functions, `int
/// (*[2])(int)`, whose length C writes inside the parentheses. A type that
/// is not an array is returned as it is.
fn element_type(ty: &str) -> String {
    match (ty.find('['), ty.find(']')) {
        | (Some(open), Some(close)) if open < close =>
            format!("{}{}", ty[..open].trim_end(), &ty[close + 1..])
                .trim()
                .to_owned(),
        | _ => ty.trim().to_owned(),
    }
}

/// The characters of `object` when it shows as one wide cell: a string
/// literal's array, or a char array of more than [`BYTE_CELLS`] bytes.
fn text(object: &Object) -> Option<Vec<Option<String>>> {
    let Value::Array { elements } = &object.value else {
        return None;
    };
    let element = element_type(&object.ty);
    let element = element
        .trim_start_matches("const ")
        .trim_start_matches("volatile ");
    if !matches!(element, "char" | "signed char" | "unsigned char")
        || object.size != elements.len()
        || !(object.literal || elements.len() > BYTE_CELLS)
    {
        return None;
    }
    elements
        .iter()
        .map(|element| match element {
            | Value::Indeterminate => Some(None),
            | Value::Integer {
                character: Some(character),
                ..
            } => Some(Some(glyph(character))),
            | Value::Integer { text, .. } => Some(Some(text.clone())),
            | _ => None,
        })
        .collect()
}

/// The values of `object` when it shows as one wide cell: an array of more
/// than [`BYTE_CELLS`] elements of a type other than char, which would
/// otherwise be a cell per element, wider than the window. `None` stands for
/// an indeterminate element.
fn values(object: &Object) -> Option<Vec<Option<String>>> {
    let Value::Array { elements } = &object.value else {
        return None;
    };
    if elements.len() <= BYTE_CELLS || text(object).is_some() {
        return None;
    }
    elements
        .iter()
        .map(|element| match element {
            | Value::Indeterminate => Some(None),
            | Value::Integer {
                character: None,
                text,
                ..
            }
            | Value::Floating { text, .. } => Some(Some(text.clone())),
            | _ => None,
        })
        .collect()
}

/// How many elements each word of a wide array's values holds: a word ends
/// before it would be wider than [`WORD`] columns, each value taking its
/// digits and a column of space after them.
fn value_words(values: &[Option<String>]) -> Vec<usize> {
    let mut words = Vec::new();
    let (mut length, mut width) = (0, 0);
    for value in values {
        let columns = value.as_deref().map_or(1, |value| value.chars().count()) + 1;
        if length > 0 && width + columns > WORD {
            words.push(length);
            (length, width) = (0, 0);
        }
        length += 1;
        width += columns;
    }
    if length > 0 {
        words.push(length);
    }
    words
}

/// How many bytes each word of a string holds, so a string that wraps
/// breaks between words: a space ends its word, and a word ends before it
/// would be wider than [`WORD`] columns. An escape is as wide as its
/// spelling (`\xff` four columns), so a word of escapes is no wider than
/// one of letters, and a phone's row holds it.
fn words(glyphs: &[Option<String>]) -> Vec<usize> {
    let mut words = Vec::new();
    let (mut length, mut width) = (0, 0);
    for glyph in glyphs {
        let columns = columns(glyph.as_deref());
        if length > 0 && width + columns > WORD {
            words.push(length);
            (length, width) = (0, 0);
        }
        length += 1;
        width += columns;
        if glyph.as_deref() == Some(" ") {
            words.push(length);
            (length, width) = (0, 0);
        }
    }
    if length > 0 {
        words.push(length);
    }
    words
}

/// The most columns of characters a string's row keeps together before it
/// may wrap: about 125 px, under half a phone's row.
const WORD: usize = 16;

/// How many columns of characters a byte's glyph takes: one per character
/// of its spelling, and one more for the box around the null character.
/// An escape is set smaller than a letter, so this errs wide.
fn columns(glyph: Option<&str>) -> usize {
    match glyph {
        | None => 1,
        | Some("\\0") => 3,
        | Some(glyph) => glyph.chars().count().max(1),
    }
}

/// A character constant's spelling as it stands in a string: `'A'` is `A`,
/// `'\''` is `'`, `'\n'` stays `\n`.
fn glyph(character: &str) -> String {
    let inner = character
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
        .unwrap_or(character);
    match inner {
        | "\\'" => "'".to_owned(),
        | inner => inner.to_owned(),
    }
}

/// A cell's value as it shows: a pointer into a string literal names it by
/// the start of its spelling, as the tape's lanes do, and keeps the index
/// (`→"\xff\xff\x…"[31]`), so a literal of escapes leaves the cell narrow
/// enough for a phone. Its label keeps the whole spelling.
fn pointer_text(text: &str) -> String {
    let (Some(spelling), Some(end)) = (text.strip_prefix("→\""), text.rfind('"')) else {
        return text.to_owned();
    };
    let Some(body) = end
        .checked_sub("→\"".len())
        .and_then(|end| spelling.get(..end))
    else {
        return text.to_owned();
    };
    format!("→\"{}{}", shorten(body, POINTER_LITERAL), &text[end..])
}

/// The most characters of a literal's spelling a pointer's cell shows.
const POINTER_LITERAL: usize = 11;

/// `"%zu bytes…"` cut to `limit` characters, as the tape cuts a long value.
fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(limit - 1).collect();
    short.push('…');
    short
}

#[expect(clippy::too_many_lines, reason = "One block per kind of cell.")]
fn strip(snapshot: &Snapshot, toggle: Toggle) -> impl IntoView {
    let opened = toggle.opened;
    let access = |object: usize, cell: Option<usize>| {
        snapshot
            .event
            .access
            .iter()
            .find(|access| {
                access.object == object
                    && (access.cell == cell
                        || access.cell.is_none()
                        || (cell.is_none() && access.cell == Some(0)))
            })
            .map(|access| access.kind.clone())
    };
    // Which pointer points at which cell, to name it under the cell.
    let pointers: Vec<(usize, i32, String)> = snapshot
        .objects
        .iter()
        .filter(|object| object.alive)
        .filter_map(|object| match object.value {
            | Value::Pointer {
                target: Some(target),
                offset,
                ended: false,
                ..
            } => Some((target, offset, object.name.clone())),
            | _ => None,
        })
        .collect();
    let pointed = |object: usize, offset: usize| {
        pointers
            .iter()
            .filter(|(target, at, _)| {
                *target == object && usize::try_from(*at).ok() == Some(offset)
            })
            .map(|(_, _, name)| format!("◂{name}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let whole = |object: usize| {
        snapshot
            .event
            .access
            .iter()
            .find(|access| access.object == object && access.cell.is_none())
            .map(|access| access.kind.clone())
    };
    let cells = |objects: Vec<&Object>| {
        objects
            .into_iter()
            .flat_map(|object| {
                let key = (object.id, object.name.clone());
                let open = opened.with(|opened| opened.contains(&key));
                parts(object, open).into_iter().map(move |part| match part {
                    | Part::Cell {
                        value,
                        name,
                        ty,
                        leaf,
                        bytes,
                    } => {
                        let kind = access(object.id, leaf);
                        let text = value_text(&value);
                        let indeterminate = text.is_none();
                        // A pointer's name is set smaller than a number, so
                        // its cell stays narrow enough for a phone's strip.
                        let long = text.as_deref().is_some_and(|text| text.chars().count() > 5);
                        let marks = pointed(object.id, leaf.unwrap_or(0));
                        // A floating value shows its bytes, lowest address
                        // first: the value's own and, for long double, the
                        // padding after them.
                        let raw = match &value {
                            | Value::Floating { bytes, .. } => Some(bytes.clone()),
                            | _ => None,
                        };
                        let plural = if bytes == 1 { "" } else { "s" };
                        // A pointer whose target's lifetime has ended is
                        // dangling: its value is indeterminate (C99
                        // §6.2.4p2).
                        let dangling = matches!(value, Value::Pointer { ended: true, .. });
                        // A pointer to a function is read out as the
                        // function and where the listing defines it.
                        let function = match &value {
                            | Value::Function { name, line, .. } => Some(format!("a pointer to the function {name}, defined on line {line}")),
                            | _ => None,
                        };
                        let shown = if dangling {
                            "dangling: the object it pointed to has ended, so its value is indeterminate"
                        } else if let Some(function) = &function {
                            function
                        } else {
                            text.as_deref().unwrap_or("indeterminate")
                        };
                        let description = format!(
                            "{name}, {ty}, {bytes} byte{plural}: {shown}{}{}{}",
                            raw.as_deref().map_or_default(|raw| format!(", stored as the bytes {raw}")),
                            kind.as_deref().map_or_default(|kind| format!(", {}", verb(kind))),
                            if object.declared { "" } else { UNDECLARED }
                        );
                        view! {
                            <li
                                class=format!("cell {}", palette::class(&object.name))
                                class:is-indeterminate=indeterminate
                                class:is-dangling=dangling
                                class:is-function=function.is_some()
                                class:is-undeclared=!object.declared
                                class:is-long=long
                                data-access=kind
                                aria-label=description
                                title=(!object.declared).then_some(UNDECLARED_TITLE)
                                style=format!("--bytes: {bytes}")
                            >
                                <span class="cell-type" aria-hidden="true">{ty}<span class="cell-bytes">{format!("{bytes}B")}</span></span>
                                <b class="cell-value" aria-hidden="true">{text.as_deref().map_or_else(|| "?".to_owned(), pointer_text)}</b>
                                {raw.map(|raw| view! { <span class="cell-raw" aria-hidden="true">{raw}</span> })}
                                <span class="cell-name" aria-hidden="true">{name}<span class="cell-pointers">{marks}</span></span>
                            </li>
                        }
                        .into_any()
                    },
                    | Part::Padding { bytes, after } => {
                        let plural = if bytes == 1 { "" } else { "s" };
                        view! {
                            <li
                                class="cell is-padding"
                                aria-label=format!("{bytes} byte{plural} of padding after {after}, holding no value")
                                title=format!("{bytes} byte{plural} of unnamed padding after {after}: no member lives here, and its bytes hold unspecified values")
                            >
                                <span class="cell-type" aria-hidden="true">"pad"</span>
                                <b class="cell-value" aria-hidden="true">{format!("{bytes}B")}</b>
                                <span class="cell-name" aria-hidden="true">"padding"</span>
                            </li>
                        }
                        .into_any()
                    },
                    | Part::Text { glyphs, ty, values } => {
                        let kinds: Vec<Option<String>> =
                            (0..glyphs.len()).map(|index| access(object.id, Some(index))).collect();
                        let marks: Vec<String> = (0..glyphs.len())
                            .filter_map(|index| {
                                let marks = pointed(object.id, index);
                                (!marks.is_empty()).then(|| format!("{marks} at [{index}]"))
                            })
                            .collect();
                        let mut description = text_description(object, &glyphs, &kinds, &marks, values);
                        if !object.declared {
                            description.push_str(UNDECLARED);
                        }
                        let count = glyphs.len();
                        let key = (object.id, object.name.clone());
                        let (unit, open, title) = if values {
                            ("element", "cells", "Show each element as a cell of its own")
                        } else {
                            ("byte", "bytes", "Show each byte as a cell of its own")
                        };
                        let label = format!("Show {} as its {count} {unit} cells", shorten(&object.name, 24));
                        let words = if values { value_words(&glyphs) } else { words(&glyphs) };
                        let mut bytes = glyphs.into_iter().zip(kinds).enumerate();
                        let bytes = words
                            .into_iter()
                            .map(|length| {
                                let word = bytes
                                    .by_ref()
                                    .take(length)
                                    .map(|(index, (glyph, kind))| {
                                        let pointed = !pointed(object.id, index).is_empty();
                                        let null = !values && glyph.as_deref() == Some("\\0");
                                        let escape = !values && glyph.as_deref().is_some_and(|glyph| glyph.starts_with('\\'));
                                        let indeterminate = glyph.is_none();
                                        view! {
                                            <span
                                                class="string-byte"
                                                class:is-ruled=index % 8 == 0
                                                class:is-null=null
                                                class:is-escape=escape && !null
                                                class:is-indeterminate=indeterminate
                                                class:is-pointed=pointed
                                                data-access=kind
                                            >
                                                <span class="string-char">{glyph.unwrap_or_default()}</span>
                                                {(index % 8 == 0).then(|| view! { <span class="string-index">{index}</span> })}
                                            </span>
                                        }
                                    })
                                    .collect_view();
                                view! { <span class="string-word">{word}</span> }
                            })
                            .collect_view();
                        let name = if object.literal { "string literal".to_owned() } else { object.name.clone() };
                        view! {
                            <li
                                class=format!("cell is-string {}", palette::class(&object.name))
                                class:is-values=values
                                class:is-undeclared=!object.declared
                                data-access=whole(object.id)
                                aria-label=description
                            >
                                <span class="cell-type">
                                    <span aria-hidden="true">{ty}<span class="cell-bytes">{format!("{}B", object.size)}</span></span>
                                    <button
                                        type="button"
                                        class="cell-open"
                                        aria-expanded="false"
                                        aria-label=label
                                        data-string=string_key(&key)
                                        title=title
                                        on:click=move |_| toggle.set(&key, true, count, values)
                                    >
                                        {open}
                                    </button>
                                </span>
                                <span class="string-bytes" aria-hidden="true">{bytes}</span>
                                <span class="cell-name" aria-hidden="true">{name}<span class="cell-pointers">{marks.join(" ")}</span></span>
                            </li>
                        }
                        .into_any()
                    },
                    | Part::TextHead { ty, count, values } => {
                        let key = (object.id, object.name.clone());
                        let (what, close, title) = if values {
                            ("values", "values", "Show the array as one cell again")
                        } else {
                            ("characters", "text", "Show the string as one cell again")
                        };
                        let label = format!("Show {} as one cell of its {what}", shorten(&object.name, 24));
                        view! {
                            <li
                                class=format!("cell is-string-head {}", palette::class(&object.name))
                                data-access=whole(object.id)
                            >
                                <span class="cell-type" aria-hidden="true">{ty}</span>
                                <button
                                    type="button"
                                    class="cell-open"
                                    aria-expanded="true"
                                    aria-label=label
                                    data-string=string_key(&key)
                                    title=title
                                    on:click=move |_| toggle.set(&key, false, count, values)
                                >
                                    {close}
                                </button>
                                <span class="cell-name" aria-hidden="true">{shorten(&object.name, 16)}</span>
                            </li>
                        }
                        .into_any()
                    },
                })
                .collect::<Vec<_>>()
            })
            .collect_view()
    };
    let statics: Vec<&Object> = snapshot
        .objects
        .iter()
        .filter(|object| object.frame_id == 0 && object.alive)
        .collect();
    let statics = (!statics.is_empty()).then(|| {
        view! {
            <li class="cell-frame is-static">
                <span class="cell-frame-name">"static"</span>
                <ol class="cell-run" aria-label="Static storage: objects that exist for the whole run">{cells(statics)}</ol>
            </li>
        }
    });
    let frames = snapshot
        .stack
        .iter()
        .map(|frame| {
            let objects: Vec<&Object> = snapshot
                .objects
                .iter()
                .filter(|object| object.frame_id == frame.id && object.alive)
                .collect();
            view! {
                <li class="cell-frame">
                    <span class="cell-frame-name">{format!("{}()", frame.name)}</span>
                    <ol class="cell-run" aria-label=format!("{}() frame", frame.name)>{cells(objects)}</ol>
                </li>
            }
        })
        .collect_view();
    let ended: Vec<&Object> = snapshot
        .objects
        .iter()
        .filter(|object| !object.alive)
        .collect();
    view! {
        <ol class="cells">
            {frames}
            {statics}
            {(!ended.is_empty()).then(|| {
                let names = tally(ended.iter().map(|object| object.name.as_str()));
                let label = format!("Lifetimes over: {names}");
                view! {
                    <li class="cell is-ended" aria-label=label>
                        <span class="cell-type" aria-hidden="true">"ended"</span>
                        <b class="cell-value" aria-hidden="true">{ended.len()}</b>
                        <span class="cell-name" aria-hidden="true">{names}</span>
                    </li>
                }
            })}
        </ol>
    }
}

/// A string or wide array cell, in words: what it holds and which bytes or
/// elements the step used.
fn text_description(
    object: &Object,
    glyphs: &[Option<String>],
    kinds: &[Option<String>],
    marks: &[String],
    values: bool,
) -> String {
    let shown = glyphs.iter().map(|glyph| glyph.as_deref().unwrap_or("?"));
    let (contents, unit) = if values {
        (
            format!("the values {}", shown.collect::<Vec<_>>().join(", ")),
            "element",
        )
    } else {
        (
            format!("the characters {}", shown.collect::<String>()),
            "byte",
        )
    };
    let mut used: Vec<String> = Vec::new();
    for (index, kind) in kinds.iter().enumerate() {
        if let Some(kind) = kind {
            used.push(format!("{unit} {index} {}", verb(kind)));
        }
    }
    let used = if used.len() == glyphs.len() && !used.is_empty() {
        format!(", {}", verb(kinds[0].as_deref().unwrap_or_default()))
    } else if used.is_empty() {
        String::new()
    } else {
        format!(", {}", used.join(", "))
    };
    let marks = if marks.is_empty() {
        String::new()
    } else {
        format!(", pointed to by {}", marks.join(", ").replace('◂', ""))
    };
    format!(
        "{}, {}, {} bytes: {contents}{used}{marks}",
        object.name, object.ty, object.size
    )
}

/// Names, each once, with how many times it repeats: `x ×30 before`.
pub(super) fn tally<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for name in names {
        match counts.iter_mut().find(|(known, _)| *known == name) {
            | Some((_, count)) => *count += 1,
            | None => counts.push((name, 1)),
        }
    }
    counts
        .into_iter()
        .map(|(name, count)| {
            if count > 1 {
                format!("{name} ×{count}")
            } else {
                name.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a cell's label adds for an object whose block has begun but whose
/// declaration execution has not reached: it exists, and its name cannot
/// be used yet (C99 §6.2.4p5, §6.2.1p7).
const UNDECLARED: &str = ", created on entry to its block; its declaration is not reached yet";

/// The tooltip of such a cell.
const UNDECLARED_TITLE: &str =
    "Created when execution entered its block; its name can be used from its declaration on";

/// What a step did to a cell, in words.
fn verb(kind: &str) -> &'static str {
    match kind {
        | "read" => "read by this step",
        | "write" => "written by this step",
        | "create" => "created by this step",
        | "declare" => "its declaration reached by this step",
        | "end" => "its lifetime ends",
        | "address" => "its address taken",
        | _ => "designated by this step",
    }
}

#[cfg(all(test, feature = "worker"))]
mod tests {
    use super::*;
    use crate::{
        demos,
        machine::Machine,
        protocol::{
            Answer,
            Command,
        },
    };

    // A table of pointers to functions writes its length inside the
    // parentheses, so its elements' type is the rest of it.
    #[test]
    fn an_array_type_gives_its_element_type() {
        assert_eq!(element_type("int[3]"), "int");
        assert_eq!(element_type("const char *[2]"), "const char *");
        assert_eq!(element_type("int (*[2])(int, int)"), "int (*)(int, int)");
        assert_eq!(element_type("int (*)(int)"), "int (*)(int)");
    }

    /// The objects at the end of a lesson's run.
    fn objects(key: &str) -> Vec<Object> {
        last(key).objects
    }

    /// The snapshot at the end of a lesson's run.
    fn last(key: &str) -> Snapshot {
        let mut machine = Machine::default();
        let snapshot = |answer| match answer {
            | Ok(Answer::Snapshot(snapshot)) => *snapshot,
            | _ => panic!("a snapshot"),
        };
        let source = demos::find(key).expect("a lesson").source.to_owned();
        let mut last: Snapshot = snapshot(machine.run(Command::Load(source)));
        while last.can_step() {
            last = snapshot(machine.run(Command::Step));
        }
        last
    }

    fn named<'a>(objects: &'a [Object], prefix: &str) -> &'a Object {
        objects
            .iter()
            .find(|object| object.name.starts_with(prefix))
            .expect("the object")
    }

    #[test]
    fn a_string_literal_is_one_cell_of_its_characters_and_its_null() {
        let unsigned = objects("unsigned");
        let format = named(&unsigned, "\"%zu");
        let glyphs = text(format).expect("a literal is one cell");
        assert_eq!(glyphs.len(), 42);
        let spelled: String = glyphs
            .iter()
            .map(|glyph| glyph.as_deref().unwrap_or("?"))
            .collect();
        assert_eq!(spelled, "%zu bytes, sum %u, -1 < 1u is %d, %u, %u\\n\\0");
        // The newline and the null character are one byte each.
        assert_eq!(glyphs[40].as_deref(), Some("\\n"));
        assert_eq!(glyphs[41].as_deref(), Some("\\0"));
        let parts = parts(format, false);
        assert!(matches!(parts.as_slice(), [Part::Text { .. }]));
    }

    #[test]
    fn an_opened_literal_names_its_byte_cells_by_index() {
        let unsigned = objects("unsigned");
        let format = named(&unsigned, "\"2u");
        let parts = parts(format, true);
        assert!(matches!(parts[0], Part::TextHead { .. }));
        let names: Vec<&str> = parts[1..]
            .iter()
            .map(|part| match part {
                | Part::Cell { name, .. } => name.as_str(),
                | _ => panic!("a byte cell"),
            })
            .collect();
        assert_eq!(names.len(), 14);
        assert_eq!(names[0], "[0]");
        assert_eq!(names[13], "[13]");
    }

    #[test]
    fn short_char_arrays_and_other_arrays_keep_their_cells() {
        // `char name[] = "Ada"` keeps a cell per byte, the null character
        // among them; an int array keeps a cell per element.
        let hello = objects("hello");
        let name = named(&hello, "name");
        assert_eq!(text(name), None);
        assert_eq!(parts(name, false).len(), 4);
        let unsigned = objects("unsigned");
        let values = named(&unsigned, "values");
        assert_eq!(text(values), None);
        assert_eq!(parts(values, false).len(), 3);
    }

    /// An `int` array of `length` elements, each its index squared, the
    /// last indeterminate.
    fn wide(length: usize) -> Object {
        let mut elements: Vec<Value> = (0..length - 1)
            .map(|index| Value::Integer {
                text:      (index * index).to_string(),
                character: None,
                constant:  None,
            })
            .collect();
        elements.push(Value::Indeterminate);
        Object {
            id:        3,
            name:      "squares".to_owned(),
            ty:        format!("int[{length}]"),
            storage:   "automatic".to_owned(),
            alive:     true,
            declared:  true,
            frame_id:  1,
            function:  "main".to_owned(),
            scope:     None,
            parameter: false,
            literal:   false,
            size:      4 * length,
            value:     Value::Array { elements },
        }
    }

    #[test]
    fn a_wide_array_is_one_cell_of_its_values_until_opened() {
        let squares = wide(12);
        let folded = values(&squares).expect("twelve ints fold");
        assert_eq!(folded.len(), 12);
        assert_eq!(folded[3].as_deref(), Some("9"));
        assert_eq!(folded[11], None);
        assert!(matches!(
            parts(&squares, false).as_slice(),
            [Part::Text { values: true, .. }]
        ));
        let opened = parts(&squares, true);
        assert!(matches!(
            opened[0],
            Part::TextHead {
                count: 12,
                values: true,
                ..
            }
        ));
        let names: Vec<&str> = opened[1..]
            .iter()
            .map(|part| match part {
                | Part::Cell { name, .. } => name.as_str(),
                | _ => panic!("an element cell"),
            })
            .collect();
        assert_eq!(names.len(), 12);
        assert_eq!(names[0], "squares[0]");
        assert_eq!(names[11], "squares[11]");
        // Eight elements still fit the window as cells of their own.
        assert_eq!(values(&wide(8)), None);
        assert_eq!(parts(&wide(8), false).len(), 8);
        // A string is not an array of values.
        let unsigned = objects("unsigned");
        assert_eq!(values(named(&unsigned, "\"%zu")), None);
    }

    #[test]
    fn a_wide_array_wraps_between_words_of_values() {
        // `0 1 4 9 16 25 ` is 14 columns, a space after each value; `36`
        // would make 17.
        let squares = values(&wide(40)).expect("forty ints fold");
        let words = value_words(&squares);
        assert_eq!(words[0], 6);
        assert_eq!(words.iter().sum::<usize>(), 40);
        let wide: Vec<Option<String>> = vec![Some("-2147483648".to_owned()); 3];
        assert_eq!(value_words(&wide), [1, 1, 1]);
    }

    #[test]
    fn a_wide_arrays_keys_say_what_it_opens_into() {
        assert_eq!(
            toggle_announcement("squares", true, 40, true),
            "squares opened into its 40 element cells"
        );
        assert_eq!(
            toggle_announcement("squares", false, 40, true),
            "squares shown as one cell of its values"
        );
        let squares = wide(10);
        let glyphs = values(&squares).expect("ten ints fold");
        let kinds: Vec<Option<String>> = (0..10)
            .map(|index| (index == 2).then(|| "write".to_owned()))
            .collect();
        assert_eq!(
            text_description(&squares, &glyphs, &kinds, &[], true),
            "squares, int[10], 40 bytes: the values 0, 1, 4, 9, 16, 25, 36, 49, 64, ?, element 2 \
             written by this step"
        );
    }

    #[test]
    fn a_string_wraps_between_words() {
        let glyphs: Vec<Option<String>> = "2u - 3 is %u"
            .chars()
            .map(|glyph| Some(glyph.to_string()))
            .chain([Some("\\n".to_owned()), Some("\\0".to_owned())])
            .collect();
        assert_eq!(words(&glyphs), [3, 2, 2, 3, 4]);
        let long = vec![Some("x".to_owned()); WORD + 3];
        assert_eq!(words(&long), [WORD, 3]);
    }

    #[test]
    fn a_word_of_escapes_is_no_wider_than_a_word_of_letters() {
        // 32 `\xff` bytes and the null character: a word of 16 such bytes
        // was 448 px, wider than a phone's 318 px row.
        let escapes: Vec<Option<String>> = std::iter::repeat_n(Some("\\xff".to_owned()), 32)
            .chain([Some("\\0".to_owned())])
            .collect();
        assert_eq!(words(&escapes), [4, 4, 4, 4, 4, 4, 4, 4, 1]);
        // Octal escapes, short escapes and letters mixed with no space.
        let mixed: Vec<Option<String>> = ["\\377", "\\n", "\\t", "a", "\\x80", "b", "\\r", "\\\\"]
            .into_iter()
            .cycle()
            .take(40)
            .map(|glyph| Some(glyph.to_owned()))
            .collect();
        let mut start = 0;
        for length in words(&mixed) {
            let width: usize = mixed[start..start + length]
                .iter()
                .map(|glyph| columns(glyph.as_deref()))
                .sum();
            assert!(width <= WORD, "a word of {width} columns");
            start += length;
        }
        assert_eq!(start, mixed.len());
        // The null character's box takes a column more than its spelling.
        assert_eq!(columns(Some("\\0")), 3);
        assert_eq!(columns(Some("\\377")), 4);
        assert_eq!(columns(None), 1);
    }

    #[test]
    fn a_long_run_without_spaces_wraps_every_word_width() {
        let run = vec![Some("w".to_owned()); 3 * WORD + 5];
        assert_eq!(words(&run), [WORD, WORD, WORD, 5]);
    }

    #[test]
    fn a_string_key_moves_the_focus_to_its_counterpart_and_says_so() {
        Owner::new().with(|| {
            let toggle = Toggle {
                opened:       RwSignal::new(Vec::new()),
                focus:        StoredValue::new(None),
                announcement: RwSignal::new(String::new()),
            };
            let key = (7, "s".to_owned());
            toggle.set(&key, true, 33, false);
            assert_eq!(toggle.opened.get_untracked(), std::slice::from_ref(&key));
            assert_eq!(toggle.focus.get_value().as_deref(), Some("7:s"));
            assert_eq!(
                toggle.announcement.get_untracked(),
                "s opened into its 33 byte cells"
            );
            // Pressed twice before a redraw, the string stays open once.
            toggle.set(&key, true, 33, false);
            assert_eq!(toggle.opened.get_untracked().len(), 1);
            toggle.focus.set_value(None);
            toggle.set(&key, false, 33, false);
            assert_eq!(toggle.opened.get_untracked(), Vec::<(usize, String)>::new());
            assert_eq!(toggle.focus.get_value().as_deref(), Some("7:s"));
            assert_eq!(
                toggle.announcement.get_untracked(),
                "s shown as one cell of its characters"
            );
        });
    }

    /// Both keys of a string carry its key, so the focus can find the one
    /// that replaces the other, and each says whether the string is open.
    #[cfg(feature = "ssr")]
    #[test]
    fn the_bytes_and_text_keys_carry_the_same_string_key() {
        let snapshot = last("unsigned");
        let format = named(&snapshot.objects, "\"2u").clone();
        let render = |open: bool| {
            Owner::new().with(|| {
                let toggle = Toggle {
                    opened:       RwSignal::new(if open {
                        vec![(format.id, format.name.clone())]
                    } else {
                        Vec::new()
                    }),
                    focus:        StoredValue::new(None),
                    announcement: RwSignal::new(String::new()),
                };
                strip(&snapshot, toggle).to_html()
            })
        };
        let marker = format!("data-string=\"{}:", format.id);
        for (open, expanded) in [
            (false, "aria-expanded=\"false\""),
            (true, "aria-expanded=\"true\""),
        ] {
            let html = render(open);
            assert_eq!(html.matches(&marker).count(), 1, "{html}");
            let button = html.find(&marker).expect("the key");
            let start = html[..button].rfind("<button").expect("a button");
            assert!(
                html[start..button].contains(expanded),
                "{}",
                &html[start..button]
            );
        }
    }

    #[test]
    fn a_pointer_into_a_literal_shows_the_start_of_its_spelling() {
        let escapes = format!("→\"{}\"[31]", "\\xff".repeat(32));
        assert_eq!(pointer_text(&escapes), "→\"\\xff\\xff\\x…\"[31]");
        assert_eq!(pointer_text("→\"Ada\""), "→\"Ada\"");
        assert_eq!(
            pointer_text("→\"\\\"quoted\\\" and more\"[2]"),
            "→\"\\\"quoted\\\"…\"[2]"
        );
        // An opening quote with no closing one is left as it is.
        assert_eq!(pointer_text("→\""), "→\"");
        assert_eq!(pointer_text("→values[1]"), "→values[1]");
        assert_eq!(pointer_text("null"), "null");
    }

    #[test]
    fn a_character_constant_is_spelled_as_in_a_string() {
        assert_eq!(glyph("'A'"), "A");
        assert_eq!(glyph("'\\''"), "'");
        assert_eq!(glyph("'\\n'"), "\\n");
        assert_eq!(glyph("'\\0'"), "\\0");
    }
}
