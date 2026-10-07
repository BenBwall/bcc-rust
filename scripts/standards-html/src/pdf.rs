//! Interprets PDF page content streams into positioned glyphs and rules.
//!
//! Only the parts of the PDF imaging model that place text and draw ruling
//! lines are modelled (ISO 32000-1 §8.4, §9.3-9.4): the current transformation
//! matrix, the text state, simple (one-byte) fonts, and stroked or filled
//! paths that are thin enough to be rules.

use std::collections::HashMap;

use lopdf::{
    Dictionary,
    Document,
    Object,
    ObjectId,
    content::Content,
};

/// A 2D affine matrix `[a b c d e f]` in PDF row-vector convention.
#[derive(Clone, Copy, Debug)]
pub struct Matrix([f64; 6]);

impl Matrix {
    const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn translate(tx: f64, ty: f64) -> Self {
        Self([1.0, 0.0, 0.0, 1.0, tx, ty])
    }

    /// `self × other`: apply `self` first, then `other`.
    fn then(self, other: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Self([
            a * a2 + b * c2,
            a * b2 + b * d2,
            c * a2 + d * c2,
            c * b2 + d * d2,
            e * a2 + f * c2 + e2,
            e * b2 + f * d2 + f2,
        ])
    }

    fn apply(self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// Length of the transformed unit vertical vector: the scale applied to
    /// font size.
    fn vertical_scale(self) -> f64 {
        let [_, _, c, d, _, _] = self.0;
        c.hypot(d)
    }
}

/// Typeface classification of a font, taken from its base font name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Serif,
    Sans,
    Mono,
    Symbol,
}

/// A font's style, which is all the converter needs to know about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontStyle {
    pub family: Family,
    pub bold:   bool,
    pub italic: bool,
}

/// One shown glyph, positioned in default user space (origin bottom left).
#[derive(Clone, Debug)]
pub struct Glyph {
    pub text:  char,
    /// Origin of the glyph, including text rise.
    pub x:     f64,
    pub y:     f64,
    /// Horizontal advance in user space.
    pub width: f64,
    /// Rendered font size in user space.
    pub size:  f64,
    pub style: FontStyle,
    /// Fill colour as 8-bit RGB.
    pub color: [u8; 3],
}

/// A horizontal or vertical stroke or thin filled rectangle.
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Everything drawn on one page that the converter uses.
#[derive(Debug)]
pub struct PageContent {
    pub width:  f64,
    pub height: f64,
    pub glyphs: Vec<Glyph>,
    pub rules:  Vec<Rule>,
    /// Bounding boxes of images, as rules spanning their extent.
    pub images: Vec<Rule>,
    /// Glyphs shown per base font name, subset tags removed.
    pub fonts:  HashMap<String, usize>,
}

struct Font {
    /// Base font name without a subset tag, such as `URWPalladioL-Roma`.
    name:   String,
    style:  FontStyle,
    codes:  [Option<char>; 256],
    widths: [f64; 256],
}

fn classify(base: &str) -> FontStyle {
    let lower = base.to_ascii_lowercase();
    // Subset fonts carry a six-letter tag such as `ABCDEF+Times-Roman`.
    let lower = lower
        .split_once('+')
        .map_or(lower.as_str(), |(_, name)| name);
    let family = if lower.contains("symbol") {
        Family::Symbol
    } else if lower.contains("courier") || lower.contains("mono") {
        Family::Mono
    } else if lower.contains("helvetica") || lower.contains("arial") || lower.contains("sans") {
        Family::Sans
    } else {
        Family::Serif
    };
    FontStyle {
        family,
        bold: lower.contains("bold") || lower.contains("black") || lower.contains("heavy"),
        italic: lower.contains("italic") || lower.contains("oblique"),
    }
}

impl Font {
    fn load(doc: &Document, dict: &Dictionary) -> Self {
        let base = dict
            .get(b"BaseFont")
            .and_then(Object::as_name)
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        let style = classify(&base);
        let mut codes = [None; 256];
        if style.family == Family::Symbol && !dict.has(b"Encoding") {
            for (code, slot) in codes.iter_mut().enumerate() {
                *slot = symbol_char(code as u8);
            }
        } else if let Ok(encoding) = dict.get_font_encoding(doc) {
            for (code, slot) in codes.iter_mut().enumerate() {
                *slot = encoding
                    .bytes_to_string(&[code as u8])
                    .ok()
                    .and_then(|s| text_char(&s));
            }
        }
        // A ToUnicode map is the authoritative text of each code (ISO 32000-1
        // §9.10.2); lopdf prefers /Encoding when both exist, so read the map
        // through a copy of the font without one.
        if dict.has(b"ToUnicode") && dict.has(b"Encoding") {
            let mut unicode_only = dict.clone();
            let _ = unicode_only.remove(b"Encoding");
            if let Ok(map) = unicode_only.get_font_encoding(doc) {
                for (code, slot) in codes.iter_mut().enumerate() {
                    if let Some(c) = map
                        .bytes_to_string(&[code as u8])
                        .ok()
                        .and_then(|s| text_char(&s))
                    {
                        *slot = Some(c);
                    }
                }
            }
        }
        let mut widths = [0.5; 256];
        if style.family == Family::Symbol {
            for (code, w) in widths.iter_mut().enumerate() {
                *w = codes[code].map_or(0.5, symbol_width);
            }
        }
        let first = dict.get(b"FirstChar").and_then(Object::as_i64).unwrap_or(0);
        if let Ok(list) = dict
            .get(b"Widths")
            .map(|o| deref(doc, o))
            .and_then(Object::as_array)
        {
            for (i, w) in list.iter().enumerate() {
                let code = usize::try_from(first).unwrap_or(0) + i;
                if code < 256 {
                    widths[code] = number(deref(doc, w)) / 1000.0;
                }
            }
        } else if style.family == Family::Mono {
            widths = [0.6; 256];
        }
        let name = base
            .split_once('+')
            .map_or(base.as_str(), |(_, n)| n)
            .to_owned();
        Self {
            name,
            style,
            codes,
            widths,
        }
    }
}

/// The character a decoded code stands for. Ligatures that decode to their
/// letters become the ligature character, which the line builder expands
/// again; undecodable codes have none.
fn text_char(text: &str) -> Option<char> {
    match text {
        | "ff" => Some('ﬀ'),
        | "fi" => Some('ﬁ'),
        | "fl" => Some('ﬂ'),
        | "ffi" => Some('ﬃ'),
        | "ffl" => Some('ﬄ'),
        | _ => text
            .chars()
            .next()
            .filter(|&c| c != '\u{FFFD}' && c != '\0'),
    }
}

/// Adobe's built-in encoding for the Symbol font (PDF 1.7 Annex D.5).
fn symbol_char(code: u8) -> Option<char> {
    const GREEK_LOWER: &str = "αβχδεφγηιϕκλμνοπθρστυϖωξψζ";
    const GREEK_UPPER: &str = "ΑΒΧΔΕΦΓΗΙϑΚΛΜΝΟΠΘΡΣΤΥςΩΞΨΖ";
    Some(match code {
        | b'a'..=b'z' => GREEK_LOWER.chars().nth(usize::from(code - b'a'))?,
        | b'A'..=b'Z' => GREEK_UPPER.chars().nth(usize::from(code - b'A'))?,
        | b'0'..=b'9'
        | b' '
        | b'!'
        | b'#'
        | b'%'
        | b'&'
        | b'('
        | b')'
        | b'+'
        | b','
        | b'.'
        | b'/'
        | b':'
        | b';'
        | b'<'
        | b'='
        | b'>'
        | b'?'
        | b'['
        | b']'
        | b'_'
        | b'{'
        | b'|'
        | b'}' => char::from(code),
        | b'"' => '∀',
        | b'$' => '∃',
        | b'\'' => '∋',
        | b'*' => '∗',
        | b'-' => '−',
        | b'@' => '≅',
        | b'\\' => '∴',
        | b'^' => '⊥',
        | b'~' => '∼',
        | 0xA2 => '′',
        | 0xA3 => '≤',
        | 0xA4 => '⁄',
        | 0xA5 => '∞',
        | 0xA6 => 'ƒ',
        | 0xAB => '↔',
        | 0xAC => '←',
        | 0xAD => '↑',
        | 0xAE => '→',
        | 0xAF => '↓',
        | 0xB0 => '°',
        | 0xB1 => '±',
        | 0xB2 => '″',
        | 0xB3 => '≥',
        | 0xB4 => '×',
        | 0xB5 => '∝',
        | 0xB6 => '∂',
        | 0xB7 => '•',
        | 0xB8 => '÷',
        | 0xB9 => '≠',
        | 0xBA => '≡',
        | 0xBB => '≈',
        | 0xBC => '…',
        | 0xC4 => '⊗',
        | 0xC5 => '⊕',
        | 0xC6 => '∅',
        | 0xC7 => '∩',
        | 0xC8 => '∪',
        | 0xC9 => '⊃',
        | 0xCA => '⊇',
        | 0xCB => '⊄',
        | 0xCC => '⊂',
        | 0xCD => '⊆',
        | 0xCE => '∈',
        | 0xCF => '∉',
        | 0xD0 => '∠',
        | 0xD1 => '∇',
        | 0xD5 => '∏',
        | 0xD6 => '√',
        | 0xD7 => '⋅',
        | 0xD8 => '¬',
        | 0xD9 => '∧',
        | 0xDA => '∨',
        | 0xDB => '⇔',
        | 0xDC => '⇐',
        | 0xDD => '⇑',
        | 0xDE => '⇒',
        | 0xDF => '⇓',
        | 0xE1 => '〈',
        | 0xE5 => '∑',
        | 0xF1 => '〉',
        | 0xF2 => '∫',
        // Pieces of tall brackets and the radical's overbar.
        | b'`' => '‾',
        | 0xE6 => '⎛',
        | 0xE7 => '⎜',
        | 0xE8 => '⎝',
        | 0xE9 => '⎡',
        | 0xEA => '⎢',
        | 0xEB => '⎣',
        | 0xEC => '⎧',
        | 0xED => '⎨',
        | 0xEE => '⎩',
        | 0xEF => '⎪',
        | 0xF6 => '⎞',
        | 0xF7 => '⎟',
        | 0xF8 => '⎠',
        | 0xF9 => '⎤',
        | 0xFA => '⎥',
        | 0xFB => '⎦',
        | 0xFC => '⎫',
        | 0xFD => '⎬',
        | 0xFE => '⎭',
        | _ => return None,
    })
}

/// Approximate Symbol advance widths in text space units (Adobe Symbol AFM).
fn symbol_width(c: char) -> f64 {
    match c {
        | '≤' | '≥' | '≠' | '×' | '±' | '−' | '≡' | '≈' | '÷' | '∼' | '≅' | '+' | '=' | '<'
        | '>' => 0.549,
        | '→' | '←' | '↔' | '⇒' | '⇐' | '⇔' => 0.987,
        | '…' => 1.0,
        | '∞' => 0.713,
        | '•' => 0.46,
        | '⋅' | ' ' | ',' | '.' | ';' | ':' => 0.25,
        | _ => 0.5,
    }
}

fn deref<'a>(doc: &'a Document, object: &'a Object) -> &'a Object {
    match object {
        | Object::Reference(id) => doc.get_object(*id).unwrap_or(object),
        | _ => object,
    }
}

fn number(object: &Object) -> f64 {
    match object {
        | Object::Integer(i) => *i as f64,
        | Object::Real(r) => f64::from(*r),
        | _ => 0.0,
    }
}

#[derive(Clone)]
struct State {
    ctm:          Matrix,
    fill:         [f64; 3],
    stroke:       [f64; 3],
    font:         Option<ObjectId>,
    size:         f64,
    char_spacing: f64,
    word_spacing: f64,
    scale:        f64,
    leading:      f64,
    rise:         f64,
}

struct Interpreter<'a> {
    doc:          &'a Document,
    fonts:        HashMap<ObjectId, Font>,
    out:          PageContent,
    /// Codes that the font's encoding does not map, for diagnostics.
    pub unmapped: HashMap<(String, u8), usize>,
}

/// Resolves the `/Resources` dictionary of a page or form, inheriting from
/// page-tree ancestors.
fn resources<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<&'a Dictionary> {
    let mut current = dict;
    loop {
        if let Ok(res) = current.get(b"Resources") {
            return deref(doc, res).as_dict().ok();
        }
        let parent = current.get(b"Parent").and_then(Object::as_reference).ok()?;
        current = doc.get_dictionary(parent).ok()?;
    }
}

fn font_ids(doc: &Document, res: Option<&Dictionary>) -> HashMap<Vec<u8>, ObjectId> {
    let mut out = HashMap::new();
    if let Some(fonts) = res
        .and_then(|r| r.get(b"Font").ok())
        .map(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
    {
        for (name, value) in fonts {
            if let Object::Reference(id) = value {
                out.insert(name.clone(), *id);
            }
        }
    }
    out
}

impl Interpreter<'_> {
    fn font(&mut self, id: ObjectId) -> &Font {
        let doc = self.doc;
        self.fonts.entry(id).or_insert_with(|| {
            Font::load(doc, doc.get_dictionary(id).unwrap_or(&Dictionary::new()))
        })
    }

    fn run(&mut self, content: &[u8], res: Option<&Dictionary>, mut gs: State, depth: usize) {
        let Ok(content) = Content::decode(content) else {
            return;
        };
        let fonts = font_ids(self.doc, res);
        let mut stack: Vec<State> = Vec::new();
        let mut tm = Matrix::IDENTITY;
        let mut tlm = Matrix::IDENTITY;
        let mut path: Vec<(f64, f64)> = Vec::new();
        let mut segments: Vec<((f64, f64), (f64, f64))> = Vec::new();
        let mut rects: Vec<[f64; 4]> = Vec::new();
        let mut line_width = 1.0;
        for op in &content.operations {
            let n = |i: usize| op.operands.get(i).map_or(0.0, number);
            match op.operator.as_str() {
                | "q" => stack.push(gs.clone()),
                | "Q" =>
                    if let Some(saved) = stack.pop() {
                        gs = saved;
                    },
                | "cm" => gs.ctm = Matrix([n(0), n(1), n(2), n(3), n(4), n(5)]).then(gs.ctm),
                | "w" => line_width = n(0),
                | "rg" | "RG" | "g" | "G" | "k" | "K" | "sc" | "scn" | "SC" | "SCN" => {
                    let values: Vec<f64> = op
                        .operands
                        .iter()
                        .filter(|o| !matches!(o, Object::Name(_)))
                        .map(number)
                        .collect();
                    let rgb = match values.as_slice() {
                        | [gray] => Some([*gray; 3]),
                        | [r, g, b] => Some([*r, *g, *b]),
                        | [c, m, y, k] => Some([
                            (1.0 - c) * (1.0 - k),
                            (1.0 - m) * (1.0 - k),
                            (1.0 - y) * (1.0 - k),
                        ]),
                        | _ => None,
                    };
                    if let Some(rgb) = rgb {
                        if op.operator.chars().next().is_some_and(char::is_uppercase) {
                            gs.stroke = rgb;
                        } else {
                            gs.fill = rgb;
                        }
                    }
                },
                | "BT" => {
                    tm = Matrix::IDENTITY;
                    tlm = Matrix::IDENTITY;
                },
                | "Tf" => {
                    gs.font = op
                        .operands
                        .first()
                        .and_then(|o| o.as_name().ok())
                        .and_then(|name| fonts.get(name).copied());
                    gs.size = n(1);
                },
                | "Tc" => gs.char_spacing = n(0),
                | "Tw" => gs.word_spacing = n(0),
                | "Tz" => gs.scale = n(0) / 100.0,
                | "TL" => gs.leading = n(0),
                | "Ts" => gs.rise = n(0),
                | "Td" => {
                    tlm = Matrix::translate(n(0), n(1)).then(tlm);
                    tm = tlm;
                },
                | "TD" => {
                    gs.leading = -n(1);
                    tlm = Matrix::translate(n(0), n(1)).then(tlm);
                    tm = tlm;
                },
                | "Tm" => {
                    tlm = Matrix([n(0), n(1), n(2), n(3), n(4), n(5)]);
                    tm = tlm;
                },
                | "T*" => {
                    tlm = Matrix::translate(0.0, -gs.leading).then(tlm);
                    tm = tlm;
                },
                | "Tj" | "'" | "\"" | "TJ" => {
                    if op.operator == "'" || op.operator == "\"" {
                        if op.operator == "\"" {
                            gs.word_spacing = n(0);
                            gs.char_spacing = n(1);
                        }
                        tlm = Matrix::translate(0.0, -gs.leading).then(tlm);
                        tm = tlm;
                    }
                    let items: Vec<&Object> = match op.operator.as_str() {
                        | "TJ" => op
                            .operands
                            .first()
                            .and_then(|o| o.as_array().ok())
                            .map(|a| a.iter().collect())
                            .unwrap_or_default(),
                        | _ => op.operands.last().into_iter().collect(),
                    };
                    for item in items {
                        match item {
                            | Object::String(bytes, _) => self.show(bytes, &gs, &mut tm),
                            | other => {
                                let adjust = -number(other) / 1000.0 * gs.size * gs.scale;
                                tm = Matrix::translate(adjust, 0.0).then(tm);
                            },
                        }
                    }
                },
                | "m" => {
                    path.clear();
                    path.push(gs.ctm.apply(n(0), n(1)));
                },
                | "l" => {
                    let to = gs.ctm.apply(n(0), n(1));
                    if let Some(&from) = path.last() {
                        segments.push((from, to));
                    }
                    path.push(to);
                },
                | "re" => {
                    let (x0, y0) = gs.ctm.apply(n(0), n(1));
                    let (x1, y1) = gs.ctm.apply(n(0) + n(2), n(1) + n(3));
                    rects.push([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]);
                },
                | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                    let width = line_width * gs.ctm.vertical_scale();
                    // Rules are drawn in black or grey; coloured strokes are
                    // decoration such as the underlines and strike-throughs
                    // of diff marks.
                    let neutral =
                        |c: [f64; 3]| (c[0] - c[1]).abs() < 0.1 && (c[1] - c[2]).abs() < 0.1;
                    if !neutral(if op.operator.starts_with(['S', 's']) {
                        gs.stroke
                    } else {
                        gs.fill
                    }) {
                        segments.clear();
                        rects.clear();
                        path.clear();
                        continue;
                    }
                    if op.operator.starts_with(['S', 's', 'B', 'b']) {
                        for ((x0, y0), (x1, y1)) in segments.drain(..) {
                            if (y0 - y1).abs() < 0.5 || (x0 - x1).abs() < 0.5 {
                                let half = width / 2.0;
                                self.out.rules.push(Rule {
                                    x0: x0.min(x1) - if (x0 - x1).abs() < 0.5 { half } else { 0.0 },
                                    y0: y0.min(y1) - if (y0 - y1).abs() < 0.5 { half } else { 0.0 },
                                    x1: x0.max(x1) + if (x0 - x1).abs() < 0.5 { half } else { 0.0 },
                                    y1: y0.max(y1) + if (y0 - y1).abs() < 0.5 { half } else { 0.0 },
                                });
                            }
                        }
                    }
                    for [x0, y0, x1, y1] in rects.drain(..) {
                        // Filled or stroked boxes thinner than 3 points are
                        // rules.
                        if (x1 - x0).min(y1 - y0) < 3.0 {
                            self.out.rules.push(Rule { x0, y0, x1, y1 });
                        }
                    }
                    segments.clear();
                    path.clear();
                },
                | "n" => {
                    segments.clear();
                    rects.clear();
                    path.clear();
                },
                | "Do" if depth < 8 => {
                    let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) else {
                        continue;
                    };
                    let Some(xobject) = res
                        .and_then(|r| r.get(b"XObject").ok())
                        .map(|o| deref(self.doc, o))
                        .and_then(|o| o.as_dict().ok())
                        .and_then(|d| d.get(name).ok())
                        .and_then(|o| o.as_reference().ok())
                        .and_then(|id| self.doc.get_object(id).ok())
                        .and_then(|o| o.as_stream().ok())
                    else {
                        continue;
                    };
                    match xobject.dict.get(b"Subtype").and_then(Object::as_name) {
                        | Ok(b"Form") => {
                            let mut inner = gs.clone();
                            if let Ok(m) = xobject.dict.get(b"Matrix").and_then(Object::as_array) {
                                let v: Vec<f64> = m.iter().map(number).collect();
                                if v.len() == 6 {
                                    inner.ctm =
                                        Matrix([v[0], v[1], v[2], v[3], v[4], v[5]]).then(gs.ctm);
                                }
                            }
                            let data = xobject.decompressed_content().unwrap_or_default();
                            let form_res = xobject
                                .dict
                                .get(b"Resources")
                                .ok()
                                .map(|o| deref(self.doc, o))
                                .and_then(|o| o.as_dict().ok())
                                .or(res);
                            self.run(&data, form_res, inner, depth + 1);
                        },
                        | Ok(b"Image") => {
                            let (x0, y0) = gs.ctm.apply(0.0, 0.0);
                            let (x1, y1) = gs.ctm.apply(1.0, 1.0);
                            self.out.images.push(Rule {
                                x0: x0.min(x1),
                                y0: y0.min(y1),
                                x1: x0.max(x1),
                                y1: y0.max(y1),
                            });
                        },
                        | _ => {},
                    }
                },
                | _ => {},
            }
        }
    }

    fn show(&mut self, bytes: &[u8], gs: &State, tm: &mut Matrix) {
        let Some(font_id) = gs.font else {
            return;
        };
        let (style, chars, widths, name) = {
            let font = self.font(font_id);
            (font.style, font.codes, font.widths, font.name.clone())
        };
        *self.out.fonts.entry(name).or_default() += bytes.len();
        for &code in bytes {
            let advance = widths[usize::from(code)] * gs.size;
            let trm = Matrix([gs.size * gs.scale, 0.0, 0.0, gs.size, 0.0, gs.rise])
                .then(*tm)
                .then(gs.ctm);
            let (x, y) = trm.apply(0.0, 0.0);
            let (x_end, _) = Matrix::translate(advance * gs.scale, 0.0)
                .then(*tm)
                .then(gs.ctm)
                .apply(0.0, 0.0);
            let size = trm.vertical_scale();
            match chars[usize::from(code)] {
                | Some(text) => self.out.glyphs.push(Glyph {
                    text,
                    x,
                    y,
                    width: x_end - x,
                    size,
                    style,
                    color: gs.fill.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8),
                }),
                | None => {
                    *self
                        .unmapped
                        .entry((format!("{style:?}"), code))
                        .or_default() += 1;
                },
            }
            let spacing = gs.char_spacing + if code == b' ' { gs.word_spacing } else { 0.0 };
            *tm = Matrix::translate((advance + spacing) * gs.scale, 0.0).then(*tm);
        }
    }
}

/// Interprets one page.
pub fn page_content(
    doc: &Document,
    page: ObjectId,
    unmapped: &mut HashMap<(String, u8), usize>,
) -> PageContent {
    let dict = doc.get_dictionary(page).expect("page dictionary");
    let mut media = [0.0, 0.0, 612.0, 792.0];
    let mut current = Some(dict);
    while let Some(d) = current {
        if let Ok(b) = d
            .get(b"MediaBox")
            .map(|o| deref(doc, o))
            .and_then(Object::as_array)
        {
            for (slot, v) in media.iter_mut().zip(b) {
                *slot = number(deref(doc, v));
            }
            break;
        }
        current = d
            .get(b"Parent")
            .and_then(Object::as_reference)
            .ok()
            .and_then(|p| doc.get_dictionary(p).ok());
    }
    let mut interp = Interpreter {
        doc,
        fonts: HashMap::new(),
        out: PageContent {
            width:  media[2] - media[0],
            height: media[3] - media[1],
            glyphs: Vec::new(),
            rules:  Vec::new(),
            images: Vec::new(),
            fonts:  HashMap::new(),
        },
        unmapped: HashMap::new(),
    };
    let content = doc.get_page_content(page);
    let state = State {
        ctm:          Matrix::translate(-media[0], -media[1]),
        fill:         [0.0; 3],
        stroke:       [0.0; 3],
        font:         None,
        size:         0.0,
        char_spacing: 0.0,
        word_spacing: 0.0,
        scale:        1.0,
        leading:      0.0,
        rise:         0.0,
    };
    interp.run(&content, resources(doc, dict), state, 0);
    for (k, v) in interp.unmapped {
        *unmapped.entry(k).or_default() += v;
    }
    interp.out
}

#[cfg(test)]
mod tests {
    use super::{
        Family,
        classify,
        symbol_char,
    };

    #[test]
    fn symbol_encoding() {
        assert_eq!(symbol_char(0xA3), Some('≤'));
        assert_eq!(symbol_char(b'p'), Some('π'));
        assert_eq!(symbol_char(b'-'), Some('−'));
        assert_eq!(symbol_char(0xE6), Some('⎛'));
    }

    #[test]
    fn font_styles() {
        let bold_mono = classify("Courier-Bold");
        assert_eq!(bold_mono.family, Family::Mono);
        assert!(bold_mono.bold);
        let italic = classify("ABCDEF+Times-Italic");
        assert_eq!(italic.family, Family::Serif);
        assert!(italic.italic && !italic.bold);
    }
}
