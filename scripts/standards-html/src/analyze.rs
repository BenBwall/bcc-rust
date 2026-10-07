//! Turns a page's lines into blocks: headings, numbered paragraphs, lists,
//! code, grammar, tables, contents entries, index entries, and footnotes.

use crate::{
    layout::{
        self,
        Line,
        Span,
    },
    pdf::{
        Family,
        Glyph,
        PageContent,
        Rule,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A clause or unnumbered top-level heading.
    Heading,
    /// A bold run-in subheading such as "Constraints" or "Syntax".
    Sub,
    Para,
    ListItem,
    Code,
    Syntax,
    /// Lines whose horizontal arrangement matters: tables and formulas.
    Layout,
    Toc,
    Index,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub kind:   Kind,
    pub lines:  Vec<Line>,
    /// The paragraph number printed in the left margin, if any.
    pub number: Option<String>,
    /// Distance from the previous block's last baseline to this block's
    /// first baseline, or from the text top for the first block.
    pub gap:    f64,
    /// Rules that fall inside a layout block.
    pub rules:  Vec<Rule>,
}

#[derive(Debug)]
pub struct Footnote {
    pub number: Option<String>,
    pub lines:  Vec<Line>,
}

#[derive(Debug)]
pub struct Page {
    pub width:       f64,
    pub height:      f64,
    /// Running header and footer lines, top to bottom.
    pub header:      Vec<Line>,
    pub footer:      Vec<Line>,
    /// The page number printed in the footer.
    pub printed:     Option<String>,
    /// The section title printed in the footer, such as "Environment".
    pub section:     Option<String>,
    pub text_left:   f64,
    pub text_right:  f64,
    /// Baseline of the first body line.
    pub top:         f64,
    pub blocks:      Vec<Block>,
    pub footnotes:   Vec<Footnote>,
    pub two_column:  Option<f64>,
    /// Vertical extents of change bars in the right margin.
    pub change_bars: Vec<(f64, f64)>,
    /// The rule printed above the footnotes.
    pub separator:   Option<Rule>,
}

/// Layout facts shared by all pages of one document.
pub struct Geometry {
    /// Body text left edge for odd and even PDF pages.
    pub left:          [f64; 2],
    pub right:         [f64; 2],
    /// How far left of the text paragraph numbers start.
    pub number_offset: f64,
    /// Margin numbers count lines (every fifth, as in ANSI C89) rather than
    /// number paragraphs.
    pub line_numbers:  bool,
    /// The text carries font styles; a scan's OCR layer does not, so its
    /// headings are known by size alone.
    pub styled:        bool,
}

fn round_half(v: f64) -> f64 {
    (v * 2.0).round() / 2.0
}

fn mode(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for v in values {
        let v = round_half(v);
        match counts.iter_mut().find(|(k, _)| (*k - v).abs() < 0.6) {
            | Some((_, n)) => *n += 1,
            | None => counts.push((v, 1)),
        }
    }
    counts.into_iter().max_by_key(|&(_, n)| n).map(|(v, _)| v)
}

/// The centre of the densest window `width` wide: a mode that tolerates the
/// jitter of OCR positions, where exact values scatter across neighbours.
fn dense(values: impl Iterator<Item = f64>, width: f64) -> Option<f64> {
    let mut v: Vec<f64> = values.collect();
    v.sort_by(f64::total_cmp);
    let (mut best, mut best_len, mut lo) = (0, 0, 0);
    for hi in 0..v.len() {
        while v[hi] - v[lo] > width {
            lo += 1;
        }
        if hi + 1 - lo > best_len {
            best_len = hi + 1 - lo;
            best = lo;
        }
    }
    (best_len > 0).then(|| round_half(v[best + best_len / 2]))
}

/// Learns the document's body text size and line spacing (recording them as
/// the layout metrics), and the body text block's edges.
pub fn geometry(pages: &[PageContent]) -> Geometry {
    let body = layout::body_size(
        pages
            .iter()
            .flat_map(|p| p.glyphs.iter().filter(|g| g.text != ' ').map(|g| g.size)),
    );
    layout::set_metrics(layout::Metrics {
        body,
        lead: 1.25,
        ..layout::metrics()
    });
    let mut left = [0.0; 2];
    let mut right = [0.0; 2];
    let mut pitches = Vec::new();
    let mut offsets = Vec::new();
    let mut numbers: Vec<u32> = Vec::new();
    for parity in 0..2 {
        let lines: Vec<Line> = pages
            .iter()
            .enumerate()
            .filter(|(i, _)| (i + 1) % 2 == parity)
            .flat_map(|(_, p)| {
                let (lo, hi) = (0.08 * p.height, 0.92 * p.height);
                layout::lines(&p.glyphs)
                    .into_iter()
                    .filter(move |l| (l.size - body).abs() < 0.6 && l.y > lo && l.y < hi)
            })
            .collect();
        left[parity] = dense(lines.iter().map(|l| l.x0), 3.0).unwrap_or(90.0);
        right[parity] = dense(lines.iter().map(|l| l.x1), 3.0).unwrap_or(520.0);
        pitches.extend(
            lines
                .windows(2)
                .map(|w| w[0].y - w[1].y)
                .filter(|&d| d > 0.9 * body && d < 2.0 * body),
        );
        offsets.extend(lines.iter().filter_map(|l| {
            let first = l.spans.first()?;
            let digits = first.text.trim_start().split(' ').next()?;
            let number = (is_digits(digits) && first.x0 < left[parity] - 3.0).then_some(digits)?;
            numbers.extend(number.parse::<u32>().ok());
            Some(left[parity] - first.x0)
        }));
    }
    let pitch = mode(pitches.into_iter()).unwrap_or(1.25 * body);
    layout::set_metrics(layout::Metrics {
        body,
        lead: pitch / body,
        ..layout::metrics()
    });
    let fifths = numbers.iter().filter(|&&n| n % 5 == 0).count();
    Geometry {
        left,
        right,
        number_offset: mode(offsets.into_iter()).unwrap_or(25.0),
        line_numbers: numbers.len() >= 20 && fifths * 10 >= numbers.len() * 9,
        styled: pages
            .iter()
            .any(|p| p.glyphs.iter().any(|g| g.style.bold || g.style.italic)),
    }
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_roman(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| matches!(b, b'i' | b'v' | b'x' | b'l' | b'c'))
}

/// A clause number at the start of heading text: `6.7.2`, `1.`, `A.1`.
pub fn clause_number(text: &str) -> Option<&str> {
    let token = text.split_whitespace().next()?;
    let token = token.strip_suffix('.').unwrap_or(token);
    let mut parts = token.split('.');
    let first = parts.next()?;
    let first_ok =
        is_digits(first) || (first.len() == 1 && first.as_bytes()[0].is_ascii_uppercase());
    let rest_ok = parts.all(is_digits);
    // A lone capital letter is only a clause number in "Annex A".
    (first_ok && rest_ok && !(first.len() == 1 && !token.contains('.') && !is_digits(first)))
        .then_some(token)
}

/// Unnumbered headings recognised by name where font styles are missing.
const TITLES: &[&str] = &[
    "Foreword",
    "Introduction",
    "Contents",
    "Index",
    "Bibliography",
];

const SUBHEADINGS: &[&str] = &[
    "Examples",
    "Syntax",
    "Constraints",
    "Semantics",
    "Description",
    "Returns",
    "Synopsis",
    "Environmental limits",
    "Recommended practice",
    "Implementation limits",
];

/// Every visible span is bold, and the line starts in the text face (a bold
/// monospace line is code, not a heading).
fn all_bold(line: &Line) -> bool {
    line.spans
        .iter()
        .all(|s| s.style.bold || s.text.trim().is_empty())
        && line
            .spans
            .first()
            .is_some_and(|s| s.style.family != Family::Mono)
}

/// The span holds contents leaders: a run of at least five spaced dots, set
/// alone (N1256) or after the entry's title (the LaTeX drafts).
pub fn has_leaders(span: &Span) -> bool {
    span.text.contains(". . . . .")
}

/// The page number that ends a contents entry's last span, after its
/// leaders when the two run together (`. . . 192`), with the text before it.
pub fn split_page_number(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_end();
    let (before, number) = text.rsplit_once(' ').unwrap_or(("", text));
    (is_digits(number) || is_roman(number)).then_some((before, number))
}

/// The line ends in a page number set apart from the text before it: a
/// span of its own, or the end of a run of leaders.
fn ends_in_page_number(line: &Line) -> bool {
    line.spans
        .last()
        .is_some_and(|s| match split_page_number(&s.text) {
            | Some(("", _)) => line.spans.len() > 1,
            | Some((before, _)) => before.ends_with('.'),
            | None => false,
        })
}

/// A contents entry: a title, leaders or a wide gap, and a page number.
fn contents_line(line: &Line, contents_page: bool) -> bool {
    ends_in_page_number(line)
        && (line.spans.iter().any(has_leaders)
            || (contents_page
                && line
                    .spans
                    .last()
                    .is_some_and(|s| s.gap >= layout::wide_gap())))
}

fn mono_dominant(line: &Line) -> bool {
    let first = line.spans.iter().find(|s| !s.text.trim().is_empty());
    let (mono, total) = line.spans.iter().fold((0, 0), |(m, t), s| {
        let n = s.text.trim().chars().count();
        (
            m + if s.style.family == Family::Mono { n } else { 0 },
            t + n,
        )
    });
    first.is_some_and(|s| s.style.family == Family::Mono) && mono * 10 >= total * 4
}

/// A grammar production's left-hand side, `type-specifier:`, optionally
/// preceded by the clause that defines it, as in Annex A's `(6.5.1)`.
fn production_header(line: &Line) -> bool {
    let text = line.text();
    let text = text.trim_end();
    let mut spans = line.spans.iter().peekable();
    if let Some(first) = spans.peek()
        && !first.style.italic
        && first.text.trim().starts_with('(')
        && first.text.trim().ends_with(')')
    {
        spans.next();
    }
    text.ends_with(':')
        && spans.all(|s| s.style.italic || s.text.trim() == ":" || s.text.trim().is_empty())
}

/// Finds an empty vertical band in the middle of the text block, the gutter
/// between two columns.
fn gutter(glyphs: &[Glyph], left: f64, right: f64, (bottom, top): (f64, f64)) -> Option<f64> {
    let lo = left + 0.3 * (right - left);
    let hi = left + 0.7 * (right - left);
    let bins = (hi - lo).ceil() as usize;
    let mut used = vec![false; bins.max(1)];
    for g in glyphs
        .iter()
        .filter(|g| g.text != ' ' && g.y > bottom && g.y < top)
    {
        let a = ((g.x - lo).floor().max(0.0) as usize).min(bins);
        let b = ((g.x + g.width - lo).ceil().max(0.0) as usize).min(bins);
        for slot in used.iter_mut().take(b).skip(a) {
            *slot = true;
        }
    }
    let (mut best, mut best_len, mut run_start, mut run) = (0, 0, 0, 0);
    for (i, u) in used.iter().enumerate() {
        if *u {
            run = 0;
        } else {
            if run == 0 {
                run_start = i;
            }
            run += 1;
            if run > best_len {
                best_len = run;
                best = run_start;
            }
        }
    }
    (best_len >= 6).then(|| lo + best as f64 + best_len as f64 / 2.0)
}

/// Analyses one page. `index` is true for pages of the two-column index.
pub fn page(content: &PageContent, pdf_number: usize, geometry: &Geometry, index: bool) -> Page {
    let parity = pdf_number % 2;
    let mut text_left = geometry.left[parity];
    let mut text_right = geometry.right[parity];
    // A scanned page sits wherever it was laid on the scanner, so its own
    // lines locate its text block when there are enough of them.
    if !geometry.styled {
        let body = layout::metrics().body;
        let own: Vec<Line> = layout::lines(&content.glyphs)
            .into_iter()
            .filter(|l| {
                (l.size - body).abs() < 0.6
                    && l.y > 0.08 * content.height
                    && l.y < 0.92 * content.height
            })
            .collect();
        // Measure where text starts after any line number, so the numbers in
        // the margin cannot pull the edge left.
        let starts: Vec<f64> = own
            .iter()
            .filter_map(|l| match l.spans.as_slice() {
                | [first, second, ..] if is_digits(first.text.trim()) => Some(second.x0),
                | [first, ..]
                    if first
                        .text
                        .trim_start()
                        .starts_with(|c: char| c.is_ascii_digit()) =>
                    None,
                | _ => Some(l.x0),
            })
            .collect();
        if starts.len() >= 3 {
            text_left = dense(starts.into_iter(), 3.0).unwrap_or(text_left);
            // Short lines (code, synopses) can outnumber full ones, so the
            // right edge is where nearly all lines have ended.
            let mut ends: Vec<f64> = own.iter().map(|l| l.x1).collect();
            ends.sort_by(f64::total_cmp);
            text_right = round_half(ends[(ends.len() * 9 / 10).min(ends.len() - 1)]);
        }
    }

    // The running header is the top line of the page, with any rows printed
    // beside it a little off its baseline, set apart from the text below. The
    // footer is the bottom line likewise, with any further lines that are
    // still in the page's bottom margin.
    let whole = layout::lines(&content.glyphs);
    let height = content.height;
    let pitch = layout::metrics().body * layout::metrics().lead;
    let cluster = |ys: &mut dyn Iterator<Item = f64>, joins: &dyn Fn(f64, f64) -> bool| {
        let Some(first) = ys.next() else {
            return (0, None);
        };
        let (mut n, mut last) = (1, first);
        for y in ys {
            if !joins(last, y) {
                return (n, Some((last - y).abs()));
            }
            n += 1;
            last = y;
        }
        (n, None)
    };
    let set_apart = |gap: Option<f64>| gap.is_none_or(|g| g > 1.2 * pitch);
    let (head, gap) = cluster(&mut whole.iter().map(|l| l.y), &|last, y| {
        last - y < 0.5 * pitch
    });
    let head = if whole.first().is_some_and(|l| l.y > 0.8 * height)
        && set_apart(gap)
        && head < whole.len()
    {
        head
    } else {
        0
    };
    let (foot, gap) = cluster(&mut whole.iter().rev().map(|l| l.y), &|last, y| {
        y - last < 0.5 * pitch || (y < 0.075 * height && y - last < 1.5 * pitch)
    });
    let foot = if whole
        .last()
        .is_some_and(|l| l.y < 0.075 * height || (l.y < 0.12 * height && set_apart(gap)))
        && foot < whole.len()
    {
        foot
    } else {
        0
    };
    let foot = foot.min(whole.len() - head);
    let header: Vec<Line> = whole[..head].to_vec();
    let footer: Vec<Line> = whole[whole.len() - foot..].to_vec();
    let band = (
        footer.first().map_or(0.0, |l| l.y + 1.0),
        header.last().map_or(height, |l| l.y - 1.0),
    );

    let two_column = if index {
        gutter(&content.glyphs, text_left, text_right, band)
    } else {
        None
    };
    let mut lines = match two_column {
        | Some(split) => {
            let (l, r): (Vec<Glyph>, Vec<Glyph>) =
                content.glyphs.iter().cloned().partition(|g| g.x < split);
            let inside = |l: &Line| l.y > band.0 && l.y < band.1;
            let mut all = layout::lines(&l);
            all.retain(inside);
            all.extend(layout::lines(&r).into_iter().filter(inside));
            all
        },
        | None => whole[head..whole.len() - foot].to_vec(),
    };

    // The printed page number starts or ends a footer or header line, or
    // stands short in one (ANSI C89 centres it in the header); the section
    // title is printed beside it.
    let page_number = |line: &Line| -> Option<String> {
        let text = line.text();
        let words: Vec<&str> = text.split_whitespace().collect();
        let number = |w: &&&str| is_digits(w) || is_roman(w);
        [words.first(), words.last()]
            .into_iter()
            .flatten()
            .find(number)
            .or_else(|| words.iter().filter(|w| w.len() <= 3).find(number))
            .map(|w| (*w).to_owned())
    };
    let numbered = footer
        .iter()
        .chain(&header)
        .find_map(|line| page_number(line).map(|n| (n, line)));
    let printed = numbered.as_ref().map(|(n, _)| n.clone());
    let section = numbered
        .and_then(|(_, line)| running_title(line))
        .or_else(|| footer.iter().find_map(running_title));

    // Footnotes sit below a short separator rule, drawn as one line or a row
    // of dashes, that starts at the text's left edge with a footnote, smaller
    // than the body, directly below it.
    let thin: Vec<&Rule> = content
        .rules
        .iter()
        .filter(|r| (r.y1 - r.y0) < 2.0 && r.x0 > text_left - 25.0 && r.x0 < text_left + 120.0)
        .collect();
    let row = |y: f64| -> Rule {
        thin.iter().filter(|r| (r.y0 - y).abs() < 1.0).fold(
            Rule {
                x0: f64::INFINITY,
                y0: y,
                x1: f64::NEG_INFINITY,
                y1: y,
            },
            |acc, r| Rule {
                x0: acc.x0.min(r.x0),
                y0: acc.y0,
                x1: acc.x1.max(r.x1),
                y1: acc.y1.max(r.y1),
            },
        )
    };
    let body = layout::metrics().body;
    let separator_line = thin
        .iter()
        .map(|r| row(r.y0))
        .filter(|r| {
            let width = r.x1 - r.x0;
            r.x0 < text_left + 5.0
                && width >= 30.0
                && width < 0.6 * (text_right - text_left)
                && lines
                    .iter()
                    .filter(|l| l.y < r.y0)
                    .max_by(|a, b| a.y.total_cmp(&b.y))
                    .is_some_and(|first| first.size < 0.95 * body)
        })
        .max_by(|a, b| a.y0.total_cmp(&b.y0));
    let separator = separator_line.map(|r| r.y0);
    let footnote_lines: Vec<Line> = match separator {
        | Some(sep) if !index => {
            let (below, above): (Vec<Line>, Vec<Line>) = lines.into_iter().partition(|l| l.y < sep);
            lines = above;
            below
        },
        | _ => Vec::new(),
    };
    let mut footnotes: Vec<Footnote> = Vec::new();
    for mut line in footnote_lines {
        strip_deletion_mark(&mut line, text_right);
        let text = line.text();
        let num = text
            .split_once(')')
            .map(|(n, _)| n)
            .filter(|n| is_digits(n) && line.x0 < text_left + 15.0);
        match (num, footnotes.last_mut()) {
            | (None, Some(last)) => last.lines.push(line),
            | (num, _) => footnotes.push(Footnote {
                number: num.map(str::to_owned),
                lines:  vec![line],
            }),
        }
    }

    let change_bars: Vec<(f64, f64)> = content
        .rules
        .iter()
        .filter(|r| (r.x1 - r.x0) < 2.0 && (r.y1 - r.y0) > 4.0 && r.x0 > text_right + 3.0)
        .map(|r| (r.y0, r.y1))
        .collect();

    attach_margin_numbers(&mut lines, text_left);

    // A contents page is one where most lines end in a page number at the
    // right margin.
    // The page's own right edge: front-matter pages need not alternate
    // margins in step with the body.
    let page_right = dense(lines.iter().map(|l| l.x1), 3.0).unwrap_or(text_right);
    let numbered = lines
        .iter()
        .filter(|l| ends_in_page_number(l) && l.x1 > page_right - 4.0)
        .count();
    let contents_page = numbered >= 8 && numbered * 2 >= lines.len();

    let mut blocks: Vec<Block> = Vec::new();
    let top = lines.first().map_or(content.height - 72.0, |l| l.y);
    let mut prev_y = top + layout::metrics().body * layout::metrics().lead;
    let mut prev_line: Option<Line> = None;
    for mut line in lines {
        strip_deletion_mark(&mut line, text_right);
        if line.spans.is_empty() {
            continue;
        }

        let mut number = line.margin_number.take();
        if number.is_none()
            && let Some(first) = line.spans.first()
            && first.x0 < text_left - 3.0
        {
            let text = first.text.trim_start();
            let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
            let rest = text[digits.len()..].to_owned();
            if !digits.is_empty() && (rest.is_empty() || rest.starts_with(' ')) {
                number = Some(digits);
                if rest.trim().is_empty() {
                    line.spans.remove(0);
                } else {
                    // The number shares a span with the text that follows it.
                    let first = &mut line.spans[0];
                    first.text = rest.trim_start().to_owned();
                    first.x0 = text_left;
                }
                line.recompute_extent();
            }
        }

        if geometry.line_numbers {
            // Line numbers locate lines, not paragraphs.
            number = None;
        }
        let gap = prev_y - line.y;
        let pitch = line.size * layout::metrics().lead;
        let text = line.text();
        let trimmed = text.trim();
        let indent = line.x0 - text_left;

        let kind = if index {
            Kind::Index
        } else if contents_line(&line, contents_page) {
            Kind::Toc
        } else if number.is_none()
            // A contents page's only heading is its title; a bold entry
            // without a page number (an annex's first line) is not one.
            && (!contents_page || trimmed == "Contents")
            && ((all_bold(&line)
                && (line.size >= 1.08 * body
                    || clause_number(trimmed).is_some()
                    || trimmed.starts_with("Annex")))
                || (!geometry.styled
                    && line.size >= 1.15 * body
                    && (clause_number(trimmed).is_some() || TITLES.contains(&trimmed))))
        {
            Kind::Heading
        } else if (all_bold(&line) || (!geometry.styled && line.size > 1.05 * body))
            && SUBHEADINGS.contains(&trimmed)
        {
            Kind::Sub
        } else if line.max_gap >= layout::wide_gap() {
            Kind::Layout
        } else if production_header(&line) {
            Kind::Syntax
        } else if trimmed.starts_with("— ") || trimmed == "—" {
            Kind::ListItem
        } else if (mono_dominant(&line)
            && (indent >= 10.0 || prev_block_kind(&blocks) == Some(Kind::Code)))
            || continues_code(&blocks, &line, gap)
        {
            Kind::Code
        } else {
            Kind::Para
        };

        let prev = blocks.last();
        let prev_pitch = prev_line.as_ref().map_or(pitch, |l| l.size * 1.25);
        let tight = gap < 0.85 * pitch.max(prev_pitch)
            && prev.is_some_and(|b| {
                !matches!(b.kind, Kind::Heading | Kind::Sub | Kind::Toc | Kind::Index)
            })
            && !matches!(kind, Kind::Heading | Kind::Sub | Kind::Toc | Kind::Index);
        let continues = match (prev, kind) {
            | _ if number.is_some() => false,
            | (Some(_), _) if tight => true,
            | (Some(b), Kind::Para) => match b.kind {
                | Kind::Para => gap <= pitch * 1.15 && !starts_run_in(&line),
                | Kind::ListItem => gap <= pitch * 1.15 && line.x0 > b.lines[0].x0 + 4.0,
                | Kind::Syntax => gap <= pitch * 2.2 && line.x0 > b.lines[0].x0 + 4.0,
                | Kind::Layout =>
                    gap <= pitch * 1.1
                        && line.x1 < text_right - 40.0
                        && prev_line
                            .as_ref()
                            .is_some_and(|p| p.max_gap >= layout::wide_gap()),
                | _ => false,
            },
            | (Some(b), Kind::Code) => match b.kind {
                | Kind::Code => gap <= pitch * 4.5,
                | Kind::ListItem | Kind::Para => gap <= pitch * 1.15,
                | Kind::Syntax => gap <= pitch * 2.2 && line.x0 > b.lines[0].x0 + 4.0,
                | _ => false,
            },
            | (Some(b), Kind::Syntax) => b.kind == Kind::Syntax && gap <= pitch * 2.2,
            | (Some(b), Kind::Layout) => b.kind == Kind::Layout && gap <= pitch * 2.5,
            | (Some(b), Kind::Heading) =>
                b.kind == Kind::Heading
                    && gap <= line.size * 1.6
                    && b.lines
                        .last()
                        .is_some_and(|l| l.size >= 1.08 * layout::metrics().body)
                    && clause_number(trimmed).is_none(),
            | _ => false,
        };
        if continues {
            let block = blocks.last_mut().expect("continues implies a block");
            if tight {
                block.kind = Kind::Layout;
            }
            block.lines.push(line.clone());
        } else {
            blocks.push(Block {
                kind,
                lines: vec![line.clone()],
                number,
                gap,
                rules: Vec::new(),
            });
        }
        prev_y = line.y;
        prev_line = Some(line);
    }

    merge_title_lines(&mut blocks, text_left);

    // Rules drawn within a table belong to its layout block.
    for block in &mut blocks {
        if block.kind != Kind::Layout {
            continue;
        }
        let top = block.lines.first().map_or(0.0, |l| l.y + l.size);
        let bottom = block.lines.last().map_or(0.0, |l| l.y - l.size * 0.4);
        block.rules = content
            .rules
            .iter()
            .filter(|r| {
                r.y1 <= top + 4.0
                    && r.y0 >= bottom - 4.0
                    && r.x0 >= text_left - 30.0
                    && r.x1 <= text_right + 3.0
            })
            .copied()
            .collect();
    }

    let has_footnotes = !footnotes.is_empty();
    Page {
        width: content.width,
        height: content.height,
        header,
        footer,
        printed,
        section,
        text_left,
        text_right,
        top,
        blocks,
        footnotes,
        two_column,
        change_bars,
        separator: separator_line.filter(|_| has_footnotes),
    }
}

/// An annex title is three centred display lines: "Annex A", "(informative)",
/// and the title itself. The lines after the first join its heading.
fn merge_title_lines(blocks: &mut Vec<Block>, text_left: f64) {
    let mut i = 0;
    while i + 1 < blocks.len() {
        let joins = {
            let (head, next) = (&blocks[i], &blocks[i + 1]);
            let first = &head.lines[0];
            let line = &next.lines[0];
            head.kind == Kind::Heading
                && first.text().trim_start().starts_with("Annex ")
                && next.lines.len() == 1
                && next.number.is_none()
                && (line.size - first.size).abs() < 0.5
                && line.x0 > text_left + 40.0
                && next.gap <= first.size * 2.2
        };
        if joins {
            let next = blocks.remove(i + 1);
            blocks[i].lines.extend(next.lines);
        } else {
            i += 1;
        }
    }
}

/// Paragraph numbers are set at the body size even beside smaller text, so
/// their baseline can differ from the line they number. A lone number in the
/// left margin is moved onto the nearest line beside it.
fn attach_margin_numbers(lines: &mut Vec<Line>, text_left: f64) {
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        let lone = line.spans.len() == 1
            && is_digits(line.spans[0].text.trim())
            && line.x1 < text_left - 3.0;
        let target = lone
            .then(|| {
                lines
                    .iter()
                    .enumerate()
                    .filter(|(j, l)| {
                        *j != i && (l.y - line.y).abs() < 5.0 && l.x0 >= text_left - 1.0
                    })
                    .min_by(|(_, a), (_, b)| (a.y - line.y).abs().total_cmp(&(b.y - line.y).abs()))
                    .map(|(j, _)| j)
            })
            .flatten();
        match target {
            | Some(j) => {
                let number = lines[i].spans[0].text.trim().to_owned();
                lines[j].margin_number = Some(number);
                lines.remove(i);
            },
            | None => i += 1,
        }
    }
}

/// The title in a running footer: the parts that are neither the page number
/// nor the `§` clause reference, as in "§5.2.4.2.2 Environment 27".
fn running_title(footer: &Line) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for span in &footer.spans {
        match parts.last_mut() {
            | Some(last) if span.gap == 0.0 => last.push_str(&span.text),
            | _ => parts.push(span.text.clone()),
        }
    }
    parts.iter().find_map(|part| {
        // A page number may lead or trail the title; numbers inside it, as
        // in "IEC 60559 floating-point arithmetic", are part of it.
        let mut words: Vec<&str> = part.split_whitespace().collect();
        while words.first().is_some_and(|w| is_digits(w) || is_roman(w)) {
            words.remove(0);
        }
        while words.last().is_some_and(|w| is_digits(w) || is_roman(w)) {
            words.pop();
        }
        let title = words.join(" ");
        let title = title.trim_matches(|c: char| c == '—' || c == '-' || c.is_whitespace());
        (!title.is_empty() && !title.starts_with('§')).then(|| title.to_owned())
    })
}

/// Removes a deletion mark, a lone asterisk past the right edge of the text,
/// and records it on the line.
fn strip_deletion_mark(line: &mut Line, text_right: f64) {
    let before = line.spans.len();
    line.spans
        .retain(|s| !(s.x0 > text_right + 3.0 && s.text.trim() == "∗"));
    if line.spans.len() != before {
        line.deleted = true;
        line.recompute_extent();
    }
}

/// A line at a code block's indentation, directly below it, is code even
/// when italic placeholders make up most of its text.
fn continues_code(blocks: &[Block], line: &Line, gap: f64) -> bool {
    blocks.last().is_some_and(|b| {
        b.kind == Kind::Code
            && gap <= line.size * 1.25 * 1.3
            && b.lines.iter().any(|l| (l.x0 - line.x0).abs() < 1.0)
            && line.spans.iter().any(|s| s.style.family == Family::Mono)
    })
}

fn prev_block_kind(blocks: &[Block]) -> Option<Kind> {
    blocks.last().map(|b| b.kind)
}

/// A paragraph that opens with a bold run-in such as "Forward references:".
fn starts_run_in(line: &Line) -> bool {
    line.spans
        .first()
        .is_some_and(|s| s.style.bold && s.text.trim_end().ends_with(':'))
}

#[cfg(test)]
mod tests {
    use super::{
        clause_number,
        running_title,
        split_page_number,
    };
    use crate::{
        layout::{
            Line,
            Mark,
            Shift,
            Span,
        },
        pdf::{
            Family,
            FontStyle,
        },
    };

    fn line(parts: &[(&str, f64)]) -> Line {
        let style = FontStyle {
            family: Family::Serif,
            bold:   false,
            italic: false,
        };
        let spans: Vec<Span> = parts
            .iter()
            .map(|&(text, gap)| Span {
                text: text.to_owned(),
                style,
                size: 12.0,
                shift: Shift::Base,
                mark: Mark::None,
                x0: 0.0,
                x1: 0.0,
                y: 45.0,
                gap,
            })
            .collect();
        Line {
            spans,
            y: 45.0,
            x0: 0.0,
            x1: 0.0,
            size: 12.0,
            max_gap: 0.0,
            deleted: false,
            margin_number: None,
        }
    }

    #[test]
    fn running_titles() {
        let odd = line(&[("§5.2.4.2.2", 0.0), ("Environment", 120.0), ("27", 150.0)]);
        assert_eq!(running_title(&odd).as_deref(), Some("Environment"));
        let even = line(&[("100", 0.0), ("Language", 160.0), ("§6.7.2", 150.0)]);
        assert_eq!(running_title(&even).as_deref(), Some("Language"));
        let annex = line(&[
            ("§G.5", 0.0),
            ("IEC 60559-compatible ", 90.0),
            ("complex arithmetic", 0.0),
            ("481", 80.0),
        ]);
        assert_eq!(
            running_title(&annex).as_deref(),
            Some("IEC 60559-compatible complex arithmetic")
        );
        let front = line(&[("Contents", 0.0), ("iii", 200.0)]);
        assert_eq!(running_title(&front).as_deref(), Some("Contents"));
        // ANSI C89 centres the page number between section and subsection.
        let ansi = line(&[("Language 46", 0.0), ("Expressions", 300.0)]);
        assert_eq!(running_title(&ansi).as_deref(), Some("Language"));
        let numbered = line(&[
            ("§F.3", 0.0),
            ("IEC 60559 floating-point arithmetic", 90.0),
            ("452", 80.0),
        ]);
        assert_eq!(
            running_title(&numbered).as_deref(),
            Some("IEC 60559 floating-point arithmetic")
        );
    }

    #[test]
    fn contents_page_numbers() {
        assert_eq!(split_page_number("192"), Some(("", "192")));
        assert_eq!(
            split_page_number("Reserved identifiers . . . . 192"),
            Some(("Reserved identifiers . . . .", "192"))
        );
        assert_eq!(split_page_number("xiii "), Some(("", "xiii")));
        assert_eq!(split_page_number("Scope"), None);
    }

    #[test]
    fn clause_numbers() {
        assert_eq!(clause_number("6.7.2 Type specifiers"), Some("6.7.2"));
        assert_eq!(clause_number("1. Scope"), Some("1"));
        assert_eq!(clause_number("A.2.1 Expressions"), Some("A.2.1"));
        assert_eq!(clause_number("Annex A"), None);
        assert_eq!(clause_number("Constraints"), None);
        assert_eq!(clause_number("C"), None);
    }
}
