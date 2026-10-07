//! Groups positioned glyphs into lines of styled spans.

use std::sync::{
    PoisonError,
    RwLock,
};

use crate::pdf::{
    Family,
    FontStyle,
    Glyph,
};

/// Vertical placement of a span relative to its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Base,
    Super,
    Sub,
}

/// Whether a span is text the document marks as inserted or deleted, as in
/// N2310's diff against C17 (blue underlined, red struck through).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    None,
    Inserted,
    Deleted,
}

/// A run of text sharing one style, size, and baseline shift.
#[derive(Clone, Debug)]
pub struct Span {
    pub text:  String,
    pub style: FontStyle,
    pub size:  f64,
    pub shift: Shift,
    pub mark:  Mark,
    pub x0:    f64,
    pub x1:    f64,
    /// Baseline of the span's first glyph.
    pub y:     f64,
    /// Horizontal distance from the previous span's last glyph.
    pub gap:   f64,
}

/// One printed line.
#[derive(Clone, Debug)]
pub struct Line {
    pub spans:         Vec<Span>,
    /// Baseline of the line's main text.
    pub y:             f64,
    pub x0:            f64,
    pub x1:            f64,
    /// Size of the line's main text.
    pub size:          f64,
    /// Widest gap between neighbouring glyphs that is not code alignment.
    pub max_gap:       f64,
    /// A deletion mark was printed in the margin beside this line.
    pub deleted:       bool,
    /// A paragraph number printed beside this line on its own baseline.
    pub margin_number: Option<String>,
}

impl Line {
    pub fn text(&self) -> String {
        let mut out = String::new();
        for s in &self.spans {
            if s.gap > 0.0 && !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(&s.text);
        }
        out
    }

    /// Refreshes the extent and widest gap after spans were removed.
    pub fn recompute_extent(&mut self) {
        self.x0 = self.spans.first().map_or(self.x0, |s| s.x0);
        self.x1 = self.spans.last().map_or(self.x1, |s| s.x1);
        if let Some(first) = self.spans.first_mut() {
            first.gap = 0.0;
        }
        self.max_gap = self.spans.iter().map(|s| s.gap).fold(0.0, f64::max);
    }
}

/// Typographic facts of one document, measured before it is analysed: the
/// size of its body text and the ratio of its line spacing to that size.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub body:       f64,
    pub lead:       f64,
    /// Pure blue and pure red text are diff marks (`--diff-marks`).
    pub diff_marks: bool,
}

static METRICS: RwLock<Metrics> = RwLock::new(Metrics {
    body:       12.0,
    lead:       1.25,
    diff_marks: false,
});

/// The diff mark a glyph's colour carries, when the document has them.
fn mark_of(g: &Glyph) -> Mark {
    match g.color {
        | [0, 0, 255] if metrics().diff_marks => Mark::Inserted,
        | [255, 0, 0] if metrics().diff_marks => Mark::Deleted,
        | _ => Mark::None,
    }
}

pub fn metrics() -> Metrics {
    *METRICS.read().unwrap_or_else(PoisonError::into_inner)
}

pub fn set_metrics(metrics: Metrics) {
    *METRICS.write().unwrap_or_else(PoisonError::into_inner) = metrics;
}

/// Gaps at least this wide separate table cells or formula parts.
pub fn wide_gap() -> f64 {
    metrics().body * 4.0 / 3.0
}

/// The most common glyph size, by glyph count: the body text size.
pub fn body_size(glyphs: impl Iterator<Item = f64>) -> f64 {
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for size in glyphs {
        let size = (size * 2.0).round() / 2.0;
        match counts.iter_mut().find(|(s, _)| (*s - size).abs() < 0.1) {
            | Some((_, n)) => *n += 1,
            | None => counts.push((size, 1)),
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map_or(12.0, |(s, _)| s)
}

/// Text to emit for a glyph: ligatures are decomposed so the output is
/// searchable, and code (monospace) text uses the ASCII characters a
/// programmer would type.
fn glyph_text(g: &Glyph, out: &mut String) {
    let mono = g.style.family == Family::Mono;
    match g.text {
        | 'ﬁ' => out.push_str("fi"),
        | 'ﬂ' => out.push_str("fl"),
        | 'ﬀ' => out.push_str("ff"),
        | 'ﬃ' => out.push_str("ffi"),
        | 'ﬄ' => out.push_str("ffl"),
        | '’' if mono => out.push('\''),
        | '‘' if mono => out.push('`'),
        | '−' | '‐' if mono => out.push('-'),
        | '“' | '”' if mono => out.push('"'),
        | c => out.push(c),
    }
}

struct Row {
    y:      f64,
    size:   f64,
    glyphs: Vec<Glyph>,
}

/// Builds lines from glyphs, top to bottom.
pub fn lines(glyphs: &[Glyph]) -> Vec<Line> {
    let mut sorted: Vec<&Glyph> = glyphs.iter().collect();
    sorted.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
    let mut rows: Vec<Row> = Vec::new();
    for g in sorted {
        match rows.last_mut() {
            | Some(row) if (row.y - g.y).abs() < 0.8 => row.glyphs.push(g.clone()),
            | _ => rows.push(Row {
                y:      g.y,
                size:   0.0,
                glyphs: vec![g.clone()],
            }),
        }
    }
    for row in &mut rows {
        row.size = dominant_size(&row.glyphs);
    }
    // Rows of small text that sit just above or below a row of larger text
    // and overlap it horizontally are superscripts and subscripts of it.
    let mut main: Vec<(Row, Vec<Shift>)> = Vec::new();
    let mut minor: Vec<Row> = Vec::new();
    let mut underscores: Vec<Row> = Vec::new();
    for row in rows {
        if row.glyphs.iter().all(|g| g.text == ' ') {
            continue;
        }
        if row.glyphs.iter().all(|g| g.text == '_' || g.text == ' ') {
            underscores.push(row);
        } else if row.size < 0.8 * metrics().body || short(&row) {
            minor.push(row);
        } else {
            let n = row.glyphs.len();
            main.push((row, vec![Shift::Base; n]));
        }
    }
    for row in minor {
        let (x0, x1) = extent(&row.glyphs);
        // Small text drawn over or under a larger glyph (the limits of a
        // summation) is stacked, not a superscript or subscript.
        let stacked = |m: &Row| {
            row.glyphs.iter().filter(|g| g.text != ' ').any(|g| {
                let centre = g.x + g.width / 2.0;
                m.glyphs
                    .iter()
                    .any(|b| b.text != ' ' && centre > b.x + 0.5 && centre < b.x + b.width - 0.5)
            })
        };
        // A few glyphs at the line's own size, nudged just off its baseline
        // into a gap in it (a code font's lowered `*`), are part of the line.
        let nudged = short(&row).then(|| {
            main.iter().position(|(m, _)| {
                let (mx0, mx1) = extent(&m.glyphs);
                (row.y - m.y).abs() < 0.25 * m.size
                    && row.size >= 0.9 * m.size
                    && x0 > mx0 - 2.0
                    && x1 < mx1 + 2.0
                    && !stacked(m)
            })
        });
        if let Some(Some(i)) = nudged {
            let (m, shifts) = &mut main[i];
            shifts.extend(std::iter::repeat_n(Shift::Base, row.glyphs.len()));
            let y = m.y;
            m.glyphs
                .extend(row.glyphs.into_iter().map(|g| Glyph { y, ..g }));
            continue;
        }
        let target = main
            .iter()
            .enumerate()
            .filter(|(_, (m, _))| {
                let dy = row.y - m.y;
                let (mx0, mx1) = extent(&m.glyphs);
                dy.abs() < 0.6 * m.size
                    && dy.abs() > 0.8
                    && (row.size < 0.9 * m.size || (short(&row) && row.size < m.size - 0.4))
                    && x0 < mx1 + 2.0
                    && x1 > mx0 - 2.0
                    && !stacked(m)
            })
            .min_by(|(_, (a, _)), (_, (b, _))| (row.y - a.y).abs().total_cmp(&(row.y - b.y).abs()))
            .map(|(i, _)| i);
        match target {
            | Some(i) => {
                let shift = if row.y > main[i].0.y {
                    Shift::Super
                } else {
                    Shift::Sub
                };
                let (m, shifts) = &mut main[i];
                shifts.extend(std::iter::repeat_n(shift, row.glyphs.len()));
                m.glyphs.extend(row.glyphs);
            },
            | None => {
                let n = row.glyphs.len();
                main.push((row, vec![Shift::Base; n]));
            },
        }
    }
    // TeX draws an underscore as a rule from another font, off the baseline;
    // it belongs on the baseline of the word it joins.
    for row in underscores {
        let (x0, x1) = extent(&row.glyphs);
        let target = main
            .iter()
            .enumerate()
            .filter(|(_, (m, _))| {
                let (mx0, mx1) = extent(&m.glyphs);
                (row.y - m.y).abs() < 0.6 * m.size && x0 < mx1 + 2.0 && x1 > mx0 - 2.0
            })
            .min_by(|(_, (a, _)), (_, (b, _))| (row.y - a.y).abs().total_cmp(&(row.y - b.y).abs()))
            .map(|(i, _)| i);
        match target {
            | Some(i) => {
                let (m, shifts) = &mut main[i];
                shifts.extend(std::iter::repeat_n(Shift::Base, row.glyphs.len()));
                let y = m.y;
                m.glyphs
                    .extend(row.glyphs.into_iter().map(|g| Glyph { y, ..g }));
            },
            | None => {
                let n = row.glyphs.len();
                main.push((row, vec![Shift::Base; n]));
            },
        }
    }
    let mut out: Vec<Line> = main
        .into_iter()
        .map(|(row, shifts)| build(row, &shifts))
        .collect();
    out.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x0.total_cmp(&b.x0)));
    out
}

/// A row of a few glyphs, such as a raised `*` slightly smaller than its
/// line, may be a superscript even when it is not much smaller than body
/// text.
fn short(row: &Row) -> bool {
    row.glyphs.iter().filter(|g| g.text != ' ').count() <= 4
}

fn extent(glyphs: &[Glyph]) -> (f64, f64) {
    let x0 = glyphs.iter().map(|g| g.x).fold(f64::INFINITY, f64::min);
    let x1 = glyphs
        .iter()
        .map(|g| g.x + g.width)
        .fold(f64::NEG_INFINITY, f64::max);
    (x0, x1)
}

fn dominant_size(glyphs: &[Glyph]) -> f64 {
    let mut sizes: Vec<(f64, usize)> = Vec::new();
    for g in glyphs.iter().filter(|g| g.text != ' ') {
        let s = (g.size * 10.0).round() / 10.0;
        match sizes.iter_mut().find(|(v, _)| (*v - s).abs() < 0.05) {
            | Some((_, n)) => *n += 1,
            | None => sizes.push((s, 1)),
        }
    }
    sizes
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map_or(0.0, |(s, _)| s)
}

fn build(row: Row, shifts: &[Shift]) -> Line {
    let size = row.size;
    let mut items: Vec<(Glyph, Shift)> =
        row.glyphs.into_iter().zip(shifts.iter().copied()).collect();
    items.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    let mut spans: Vec<Span> = Vec::new();
    let mut prev: Option<(f64, bool)> = None;
    let mut pending_space = false;
    let mut max_gap: f64 = 0.0;
    for (mut g, shift) in items {
        if g.text == ' ' {
            pending_space = prev.is_some();
            continue;
        }
        // TeX draws the underscore in code from a text font; touching code,
        // it takes the code's style so `nullptr_t` stays one run.
        if g.text == '_'
            && let Some(last) = spans.last()
            && g.style.family != Family::Mono
            && last.style.family == Family::Mono
            && prev.is_some_and(|(end, _)| (g.x - end).abs() < 0.08 * g.size)
        {
            g.style = last.style;
            g.size = last.size;
        }
        let mono = g.style.family == Family::Mono;
        let gap = prev.map_or(0.0, |(end, _)| g.x - end);
        let both_mono = mono && prev.is_some_and(|(_, m)| m);
        let wide = prev.is_some() && gap >= wide_gap() && !both_mono;
        if wide {
            max_gap = max_gap.max(gap);
        }
        // Spaces: code keeps its column alignment; prose gets one space for
        // any gap wider than letter spacing.
        let spaces = if prev.is_none() || wide {
            0
        } else if both_mono {
            // Consecutive underscores are nudged apart slightly; only gaps of
            // most of a character cell are spaces.
            let cell = 0.6 * g.size;
            let n = (gap / cell + 0.3).floor();
            // Code spacing comes from the geometry alone: troff overstrikes
            // `__` as underscore, space, and an underscore drawn back over it.
            n.max(0.0) as usize
        } else {
            // A space glyph only counts when it leaves a visible gap: one
            // positioned inside a kerned word is not a word break.
            usize::from(
                (pending_space && gap > 0.08 * g.size) || gap > 0.15 * g.size.max(size * 0.8),
            )
        };
        pending_space = false;
        let mark = mark_of(&g);
        let same = !wide
            && spans.last().is_some_and(|s| {
                s.style == g.style
                    && s.shift == shift
                    && s.mark == mark
                    && (s.size - g.size).abs() < 0.3
            });
        if spaces > 0
            && let Some(last) = spans.last_mut()
        {
            // Keep the space inside the earlier span so styled runs stay tight.
            last.text.extend(std::iter::repeat_n(' ', spaces));
        }
        if same {
            let last = spans.last_mut().expect("checked");
            glyph_text(&g, &mut last.text);
            last.x1 = g.x + g.width;
        } else {
            let mut text = String::new();
            glyph_text(&g, &mut text);
            spans.push(Span {
                text,
                style: g.style,
                size: g.size,
                shift,
                mark,
                x0: g.x,
                x1: g.x + g.width,
                y: g.y,
                gap: if wide { gap } else { 0.0 },
            });
        }
        prev = Some((g.x + g.width, mono));
    }
    let x0 = spans.first().map_or(0.0, |s| s.x0);
    let x1 = spans.last().map_or(0.0, |s| s.x1);
    Line {
        spans,
        y: row.y,
        x0,
        x1,
        size,
        max_gap,
        deleted: false,
        margin_number: None,
    }
}
