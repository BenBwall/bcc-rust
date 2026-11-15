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
//! rest of the string, and a key opens it into its byte cells. On a phone
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
    // The strip is drawn anew at each step, at its start. Where it is wider
    // than the window (or, on a phone, where a call's row is), bring the
    // cells the step used into the middle, or, when they are wider than the
    // window too, the first of them to its left edge.
    let section = NodeRef::<html::Section>::new();
    Effect::new(move || {
        snapshot.track();
        opened.track();
        let Some(section) = section.get() else {
            return;
        };
        let Ok(rows) = section.query_selector_all(".cells, .cell-run") else {
            return;
        };
        for index in 0..rows.length() {
            if let Some(row) = element(rows.item(index)) {
                follow(&row);
            }
        }
    });
    view! {
        <section class="head-cells" aria-labelledby="cells-title" node_ref=section>
            <h2 class="module-caption" id="cells-title">
                <b>"Under the head"</b>" the objects at the playhead, by call"
            </h2>
            {move || snapshot.with(|snapshot| match snapshot {
                | Some(snapshot) if !snapshot.objects.is_empty() => strip(snapshot, opened).into_any(),
                | Some(_) => view! { <p class="cells-empty">"No objects yet. Each declaration that runs puts one on the tape."</p> }.into_any(),
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

fn element(node: Option<web_sys::Node>) -> Option<web_sys::Element> {
    wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(node?).ok()
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
        /// Its size in bytes on x86-64.
        bytes: usize,
    },
    Padding {
        bytes: usize,
        /// The member it follows.
        after: String,
    },
    /// A char array as one cell: each byte's character as a constant spells
    /// it without its quotes (`A`, `\n`, `\0`), or `None` while
    /// indeterminate.
    Text {
        glyphs: Vec<Option<String>>,
        ty:     String,
    },
    /// The key before an opened string's byte cells that folds them back.
    TextHead {
        ty: String,
    },
}

/// A char array of up to this many bytes keeps a cell per byte, as the
/// hello lesson's `name` does; a longer one reads as its text, one cell.
const BYTE_CELLS: usize = 8;

/// The cells of `object`: one per scalar, in memory order, with a padding
/// cell after each structure member that has padding; or, for a string the
/// reader has not opened, one cell of its characters.
fn parts(object: &Object, open: bool) -> Vec<Part> {
    let mut parts = Vec::new();
    let ty = object.ty.split('[').next().unwrap_or_default().trim().to_owned();
    if let Some(glyphs) = text(object) {
        if !open {
            return vec![Part::Text {
                glyphs,
                ty: object.ty.clone(),
            }];
        }
        parts.push(Part::TextHead {
            ty: object.ty.clone(),
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
                let ty = member.ty.split('[').next().unwrap_or_default().trim();
                collect(&member.value, &path, ty, member.size, leaf, parts);
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

/// The characters of `object` when it shows as one wide cell: a string
/// literal's array, or a char array of more than [`BYTE_CELLS`] bytes.
fn text(object: &Object) -> Option<Vec<Option<String>>> {
    let Value::Array { elements } = &object.value else {
        return None;
    };
    let element = object.ty.split('[').next().unwrap_or_default().trim();
    let element = element.trim_start_matches("const ").trim_start_matches("volatile ");
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
fn strip(snapshot: &Snapshot, opened: RwSignal<Vec<(usize, String)>>) -> impl IntoView {
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
                        let description = format!(
                            "{name}, {ty}, {bytes} byte{plural}: {}{}{}",
                            text.as_deref().unwrap_or("indeterminate"),
                            raw.as_deref().map(|raw| format!(", stored as the bytes {raw}")).unwrap_or_default(),
                            kind.as_deref().map(|kind| format!(", {}", verb(kind))).unwrap_or_default()
                        );
                        view! {
                            <li
                                class=format!("cell {}", palette::class(&object.name))
                                class:is-indeterminate=indeterminate
                                class:is-long=long
                                data-access=kind
                                aria-label=description
                                style=format!("--bytes: {bytes}")
                            >
                                <span class="cell-type" aria-hidden="true">{ty}<span class="cell-bytes">{format!("{bytes}B")}</span></span>
                                <b class="cell-value" aria-hidden="true">{text.unwrap_or_else(|| "?".to_owned())}</b>
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
                    | Part::Text { glyphs, ty } => {
                        let kinds: Vec<Option<String>> =
                            (0..glyphs.len()).map(|index| access(object.id, Some(index))).collect();
                        let marks: Vec<String> = (0..glyphs.len())
                            .filter_map(|index| {
                                let marks = pointed(object.id, index);
                                (!marks.is_empty()).then(|| format!("{marks} at [{index}]"))
                            })
                            .collect();
                        let description = text_description(object, &glyphs, &kinds, &marks);
                        let count = glyphs.len();
                        let key = (object.id, object.name.clone());
                        let label = format!("Show {} as its {count} byte cells", shorten(&object.name, 24));
                        let words = words(&glyphs);
                        let mut bytes = glyphs.into_iter().zip(kinds).enumerate();
                        let bytes = words
                            .into_iter()
                            .map(|length| {
                                let word = bytes
                                    .by_ref()
                                    .take(length)
                                    .map(|(index, (glyph, kind))| {
                                        let pointed = !pointed(object.id, index).is_empty();
                                        let null = glyph.as_deref() == Some("\\0");
                                        let escape = glyph.as_deref().is_some_and(|glyph| glyph.starts_with('\\'));
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
                                data-access=whole(object.id)
                                aria-label=description
                            >
                                <span class="cell-type">
                                    <span aria-hidden="true">{ty}<span class="cell-bytes">{format!("{count}B")}</span></span>
                                    <button
                                        type="button"
                                        class="cell-open"
                                        aria-expanded="false"
                                        aria-label=label
                                        title="Show each byte as a cell of its own"
                                        on:click=move |_| opened.update(|opened| opened.push(key.clone()))
                                    >
                                        "bytes"
                                    </button>
                                </span>
                                <span class="string-bytes" aria-hidden="true">{bytes}</span>
                                <span class="cell-name" aria-hidden="true">{name}<span class="cell-pointers">{marks.join(" ")}</span></span>
                            </li>
                        }
                        .into_any()
                    },
                    | Part::TextHead { ty } => {
                        let key = (object.id, object.name.clone());
                        let label = format!("Show {} as one cell of its characters", shorten(&object.name, 24));
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
                                    title="Show the string as one cell again"
                                    on:click=move |_| opened.update(|opened| opened.retain(|open| *open != key))
                                >
                                    "text"
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

/// A string cell, in words: what it holds and which bytes the step used.
fn text_description(
    object: &Object,
    glyphs: &[Option<String>],
    kinds: &[Option<String>],
    marks: &[String],
) -> String {
    let characters: String = glyphs
        .iter()
        .map(|glyph| glyph.as_deref().unwrap_or("?"))
        .collect();
    let mut used: Vec<String> = Vec::new();
    for (index, kind) in kinds.iter().enumerate() {
        if let Some(kind) = kind {
            used.push(format!("byte {index} {}", verb(kind)));
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
        "{}, {}, {} bytes: the characters {characters}{used}{marks}",
        object.name,
        object.ty,
        glyphs.len()
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

/// What a step did to a cell, in words.
fn verb(kind: &str) -> &'static str {
    match kind {
        | "read" => "read by this step",
        | "write" => "written by this step",
        | "create" => "created by this step",
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

    /// The objects at the end of a lesson's run.
    fn objects(key: &str) -> Vec<Object> {
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
        last.objects
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
    fn a_character_constant_is_spelled_as_in_a_string() {
        assert_eq!(glyph("'A'"), "A");
        assert_eq!(glyph("'\\''"), "'");
        assert_eq!(glyph("'\\n'"), "\\n");
        assert_eq!(glyph("'\\0'"), "\\0");
    }
}
