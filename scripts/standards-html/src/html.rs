//! Renders analysed pages as one static HTML document with an element per
//! PDF page.

use std::{
    collections::{
        HashMap,
        HashSet,
    },
    fmt::Write as _,
};

use crate::{
    analyze::{
        Block,
        Kind,
        Page,
        clause_number,
        has_leaders,
        split_page_number,
    },
    chrome,
    layout::{
        self,
        Line,
        Mark,
        Shift,
        Span,
    },
    pdf::Family,
};

/// Times metrics: ascent and descent as fractions of the font size.
const ASCENT: f64 = 0.891;
const DESCENT: f64 = 0.216;

/// Distance from the top of a line box to its baseline.
fn above(size: f64, line_height: f64) -> f64 {
    (line_height - (ASCENT + DESCENT) * size) / 2.0 + ASCENT * size
}

/// Distance from a baseline to the bottom of its line box.
fn below(size: f64, line_height: f64) -> f64 {
    line_height - above(size, line_height)
}

/// Line height used for text of `size` points: the document's own leading
/// for body-sized text, and 1.2 for footnotes and other small text.
fn line_height(size: f64) -> f64 {
    let metrics = layout::metrics();
    if size < 0.9 * metrics.body {
        size * 1.2
    } else {
        size * metrics.lead
    }
}

/// The most common value, rounded to half a point.
fn most_common(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for v in values {
        let v = (v * 2.0).round() / 2.0;
        match counts.iter_mut().find(|(k, _)| (*k - v).abs() < 0.1) {
            | Some((_, n)) => *n += 1,
            | None => counts.push((v, 1)),
        }
    }
    counts.into_iter().max_by_key(|&(_, n)| n).map(|(v, _)| v)
}

fn pt(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r == r.trunc() {
        format!("{r:.0}pt")
    } else {
        format!("{r:.1}pt")
    }
}

pub fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            | '&' => out.push_str("&amp;"),
            | '<' => out.push_str("&lt;"),
            | '>' => out.push_str("&gt;"),
            | '"' => out.push_str("&quot;"),
            | c => out.push(c),
        }
    }
}

/// CSS font stacks standing in for the PDF's own faces.
pub struct Fonts {
    pub serif:       &'static str,
    pub mono:        &'static str,
    /// Code is mostly set bold (Courier-Bold in the troff drafts) or not.
    pub mono_weight: &'static str,
}

impl Fonts {
    /// Picks the stacks closest in design and metrics to the most used text
    /// and code fonts, by base font name.
    pub fn for_names<'a>(names: impl Iterator<Item = (&'a str, usize)>) -> Self {
        let mut serif = ("", 0);
        let mut mono = ("", 0);
        let (mut mono_bold, mut mono_total) = (0, 0);
        for (name, count) in names {
            let lower = name.to_ascii_lowercase();
            let is_mono =
                lower.contains("mono") || lower.contains("courier") || lower.contains("typewriter");
            if is_mono {
                mono_total += count;
                if lower.contains("bold") {
                    mono_bold += count;
                }
            }
            let slot = if is_mono { &mut mono } else { &mut serif };
            if count > slot.1 {
                *slot = (name, count);
            }
        }
        let serif_name = serif.0.to_ascii_lowercase();
        let mono_name = mono.0.to_ascii_lowercase();
        Self {
            serif:       if serif_name.contains("palladio")
                || serif_name.contains("palatino")
                || serif_name.contains("pagella")
            {
                // Metric clones of Palladio first; lines are set as printed, so
                // a wider stand-in such as Palatino Linotype
                // would overrun the margin where narrower Times
                // only spaces out.
                r#""URW Palladio L", P052, "TeX Gyre Pagella", "Times New Roman", Times, serif"#
            } else {
                r#""Times New Roman", Times, "Liberation Serif", "Nimbus Roman", serif"#
            },
            mono:        if mono_name.contains("bera")
                || mono_name.contains("dejavu")
                || mono_name.contains("vera")
            {
                r#""DejaVu Sans Mono", "Bitstream Vera Sans Mono", Menlo, Consolas, "Liberation Mono", monospace"#
            } else {
                r#""Courier New", Courier, "Liberation Mono", "Nimbus Mono PS", monospace"#
            },
            mono_weight: if mono_bold * 2 > mono_total {
                "bold"
            } else {
                "normal"
            },
        }
    }
}

/// Everything the renderer learns about the document before writing pages.
pub struct Document {
    pub title:     String,
    pub pages:     Vec<Page>,
    /// Ids assigned to headings, so cross-references can link to them.
    ids:           HashSet<String>,
    /// Printed page number to PDF page number.
    printed:       HashMap<String, usize>,
    /// Word frequencies, for deciding whether a line-end hyphen is part of
    /// the word.
    words:         HashMap<String, usize>,
    /// Contents tab stops: indent of an entry to the width of its number.
    tab_stops:     Vec<(f64, f64)>,
    /// Headings in document order, for the outline.
    outline:       Vec<chrome::Entry>,
    /// The PDF's own bookmarks, which the outline follows when present.
    bookmarks:     Vec<Bookmark>,
    /// How far left of the text paragraph numbers start.
    number_offset: f64,
    /// CSS font stacks for the document's text and code faces.
    fonts:         Fonts,
}

/// One entry of the PDF's bookmark tree.
pub struct Bookmark {
    /// 1 for top-level bookmarks.
    pub level: usize,
    pub title: String,
    /// 1-based PDF page the bookmark opens.
    pub page:  usize,
}

/// Per-block identity decided by the structure pass.
#[derive(Clone, Debug, Default)]
struct Ident {
    id:        Option<String>,
    /// The numbered paragraph this block belongs to.
    container: Option<String>,
    /// The block continues a paragraph from the previous page.
    continues: bool,
}

fn heading_id(text: &str) -> String {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("Annex ") {
        return rest.split_whitespace().next().unwrap_or(rest).to_owned();
    }
    if let Some(n) = clause_number(text) {
        return n.to_owned();
    }
    let mut slug = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

fn paragraph_id(clause: &str, number: &str) -> String {
    if clause.starts_with(|c: char| c.is_ascii_digit())
        || clause.len() == 1
        || clause.as_bytes().get(1) == Some(&b'.')
    {
        format!("{clause}p{number}")
    } else {
        format!("{clause}-p{number}")
    }
}

fn block_text(block: &Block) -> String {
    block
        .lines
        .iter()
        .map(Line::text)
        .collect::<Vec<_>>()
        .join(" ")
}

impl Document {
    pub fn new(
        title: String,
        pages: Vec<Page>,
        bookmarks: Vec<Bookmark>,
        number_offset: f64,
        fonts: Fonts,
    ) -> Self {
        let mut words: HashMap<String, usize> = HashMap::new();
        for page in &pages {
            for block in &page.blocks {
                for line in &block.lines {
                    for word in line.text().split(|c: char| !c.is_alphabetic() && c != '-') {
                        let word = word.trim_matches('-');
                        if !word.is_empty() {
                            *words.entry(word.to_lowercase()).or_default() += 1;
                        }
                    }
                }
            }
        }
        let mut printed = HashMap::new();
        for (i, page) in pages.iter().enumerate() {
            if let Some(p) = &page.printed {
                printed.entry(p.clone()).or_insert(i + 1);
            }
        }
        let mut tab_stops: Vec<(f64, f64)> = Vec::new();
        for page in &pages {
            for line in page
                .blocks
                .iter()
                .filter(|b| b.kind == Kind::Toc)
                .flat_map(|b| &b.lines)
            {
                // A number, its title after a tab, and the page number last.
                if let [number, title, _, ..] = line.spans.as_slice()
                    && title.gap > 0.0
                    && !title.style.bold
                {
                    let indent = line.x0 - page.text_left;
                    if !tab_stops.iter().any(|&(i, _)| (i - indent).abs() < 1.0) {
                        tab_stops.push((indent, title.x0 - number.x0));
                    }
                }
            }
        }
        Self {
            title,
            pages,
            ids: HashSet::new(),
            printed,
            words,
            tab_stops,
            outline: Vec::new(),
            bookmarks,
            number_offset,
            fonts,
        }
    }

    /// The sidebar outline: the PDF's bookmarks when it has them, each linked
    /// to its clause heading or else to its page; otherwise the headings.
    fn outline_entries(&self) -> Vec<chrome::Entry> {
        if self.bookmarks.is_empty() {
            return self
                .outline
                .iter()
                .map(|e| chrome::Entry {
                    id:    e.id.clone(),
                    text:  e.text.clone(),
                    level: e.level,
                    page:  e.page,
                })
                .collect();
        }
        self.bookmarks
            .iter()
            .map(|b| {
                let text = b.title.split_whitespace().collect::<Vec<_>>().join(" ");
                let wanted = heading_id(&text);
                let heading = self
                    .outline
                    .iter()
                    .find(|h| h.id == wanted && h.page.abs_diff(b.page) <= 1);
                chrome::Entry {
                    id: heading.map_or_else(|| format!("page-{}", b.page), |h| h.id.clone()),
                    page: heading.map_or(b.page, |h| h.page),
                    text,
                    level: b.level,
                }
            })
            .collect()
    }

    /// Sizes measured from this document, used by the style sheet.
    fn variables(&self, width: f64, height: f64, out: &mut String) {
        let metrics = layout::metrics();
        let lines = |kind: Kind| {
            self.pages
                .iter()
                .flat_map(move |p| p.blocks.iter().filter(move |b| b.kind == kind))
                .flat_map(|b| &b.lines)
        };
        let note = most_common(
            self.pages
                .iter()
                .flat_map(|p| &p.footnotes)
                .flat_map(|n| &n.lines)
                .map(|l| l.size),
        )
        .unwrap_or(metrics.body * 0.83);
        let index = most_common(lines(Kind::Index).map(|l| l.size)).unwrap_or(note);
        // Dot leaders are spaced dots; the pitch is the run's width over its
        // gaps, less one dot (a period is a quarter em).
        let leader = most_common(lines(Kind::Toc).flat_map(|l| &l.spans).filter_map(|s| {
            let dots = s.text.matches('.').count();
            (s.style.bold && dots >= 5 && s.text.trim().chars().all(|c| c == '.' || c == ' '))
                .then(|| (s.x1 - s.x0 - 0.25 * s.size) / (dots - 1) as f64)
        }))
        // Leaders trailing a title (the LaTeX drafts) are not measurable
        // alone; their dots sit about four fifths of an em apart.
        .unwrap_or(0.8 * metrics.body);
        let thumb = 122.4 / (width * 4.0 / 3.0);
        let _ = writeln!(
            out,
            ":root {{ --serif: {}; --mono: {}; --mono-weight: {}; --mono-other: {}; --body: {}; \
             --lead: {}; --pn: {}; --note: {}; --note-lead: {}; --index: {}; --index-lead: {}; \
             --leader: {}; --page-h: {}; --thumb-scale: {thumb:.4}; --thumb-h: {:.1}px; }}",
            self.fonts.serif,
            self.fonts.mono,
            self.fonts.mono_weight,
            if self.fonts.mono_weight == "bold" {
                "normal"
            } else {
                "bold"
            },
            pt(metrics.body),
            pt(metrics.body * metrics.lead),
            pt(self.number_offset),
            pt(note),
            pt(line_height(note)),
            pt(index),
            pt(line_height(index)),
            pt(leader),
            pt(height),
            122.4 * height / width,
        );
    }

    /// The width of a contents entry's number column at `indent`.
    fn tab_stop(&self, indent: f64) -> Option<f64> {
        self.tab_stops
            .iter()
            .find(|&&(i, _)| (i - indent).abs() < 1.0)
            .map(|&(_, w)| w)
    }

    /// Assigns heading, paragraph, and continuation identity to every block.
    fn structure(&mut self) -> Vec<Vec<Ident>> {
        let mut out = Vec::new();
        let mut clause = String::from("front");
        let mut open: Option<String> = None;
        let mut used: HashSet<String> = HashSet::new();
        let unique = |id: String, used: &mut HashSet<String>| {
            let mut candidate = id.clone();
            let mut n = 2;
            while !used.insert(candidate.clone()) {
                candidate = format!("{id}-{n}");
                n += 1;
            }
            candidate
        };
        for (page_index, page) in self.pages.iter().enumerate() {
            let mut idents = Vec::new();
            for (i, block) in page.blocks.iter().enumerate() {
                let mut ident = Ident::default();
                match block.kind {
                    | Kind::Heading => {
                        let text = block_text(block);
                        let id = unique(heading_id(&text), &mut used);
                        clause = id.clone();
                        self.ids.insert(id.clone());
                        self.outline.push(chrome::Entry {
                            id:    id.clone(),
                            text:  text.split_whitespace().collect::<Vec<_>>().join(" "),
                            level: heading_level(&text),
                            page:  page_index + 1,
                        });
                        ident.id = Some(id);
                        open = None;
                    },
                    | Kind::Sub | Kind::Toc | Kind::Index => open = None,
                    | _ => {
                        let run_in = block.lines[0]
                            .spans
                            .first()
                            .is_some_and(|s| s.style.bold && s.text.trim_end().ends_with(':'));
                        if let Some(n) = &block.number {
                            let id = unique(paragraph_id(&clause, n), &mut used);
                            ident.id = Some(id.clone());
                            open = Some(id);
                        } else if run_in {
                            open = None;
                        } else if i == 0 && open.is_some() {
                            ident.continues = true;
                        }
                        ident.container.clone_from(&open);
                    },
                }
                idents.push(ident);
            }
            out.push(idents);
        }
        out
    }

    pub fn render(mut self) -> String {
        let idents = self.structure();
        let mut out = String::with_capacity(4 << 20);
        out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
        out.push_str(
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>",
        );
        escape(&self.title, &mut out);
        out.push_str("</title>\n<style>\n");
        out.push_str(STYLE);
        out.push_str(chrome::STYLE);
        let (w, h) = self
            .pages
            .first()
            .map_or((612.0, 792.0), |p| (p.width, p.height));
        let _ = writeln!(out, "@page {{ size: {} {}; margin: 0; }}", pt(w), pt(h));
        chrome::spreads(
            self.pages.iter().map(|p| p.width).fold(w, f64::max),
            &mut out,
        );
        self.variables(w, h, &mut out);
        out.push_str("</style>\n</head>\n<body>\n");
        let thumbs: Vec<chrome::Thumb<'_>> = self
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| chrome::Thumb {
                number:  i + 1,
                printed: p.printed.as_deref(),
            })
            .collect();
        let outline = self.outline_entries();
        chrome::sidebar(&self.title, &thumbs, &outline, &mut out);
        out.push_str("<main class=\"doc\">\n");
        let mut prev_last_kind: Option<Kind> = None;
        for (index, page) in self.pages.iter().enumerate() {
            let renderer = PageRenderer {
                doc: &self,
                page,
                number: index + 1,
            };
            renderer.render(&idents[index], prev_last_kind, &mut out);
            prev_last_kind = page.blocks.last().map(|b| b.kind);
        }
        out.push_str("</main>\n<script>\n");
        out.push_str(chrome::SCRIPT);
        out.push_str("</script>\n</body>\n</html>\n");
        out
    }
}

struct PageRenderer<'a> {
    doc:    &'a Document,
    page:   &'a Page,
    number: usize,
}

impl PageRenderer<'_> {
    fn render(&self, idents: &[Ident], prev_last_kind: Option<Kind>, out: &mut String) {
        let page = self.page;
        let _ = write!(
            out,
            "<section class=\"page\" id=\"page-{}\" data-page=\"{}\"",
            self.number, self.number
        );
        if let Some(p) = &page.printed {
            let _ = write!(out, " data-printed=\"{p}\"");
        }
        if let Some(section) = &page.section {
            out.push_str(" data-section=\"");
            escape(section, out);
            out.push('"');
        }
        let _ = writeln!(
            out,
            " style=\"width:{};height:{}\">",
            pt(page.width),
            pt(page.height)
        );
        for header in &page.header {
            self.running(header, "head", out);
        }
        let body_width = page.text_right - page.text_left;
        let first_size = page
            .blocks
            .first()
            .map_or(layout::metrics().body, |b| b.lines[0].size);
        let top = page.height - page.top - above(first_size, line_height(first_size));
        let _ = write!(
            out,
            "<div class=\"body\" style=\"left:{};top:{};width:{}\"",
            pt(page.text_left),
            pt(top),
            pt(body_width)
        );
        if std::env::var_os("STANDARDS_HTML_DEBUG").is_some() {
            // Where the PDF's last body line box ends, for layout checks.
            if let Some(last) = page.blocks.last().and_then(|b| b.lines.last()) {
                let bottom = page.height - last.y + below(last.size, line_height(last.size));
                let _ = write!(out, " data-expect-bottom=\"{bottom:.1}\"");
            }
        }
        out.push_str(
            ">
",
        );
        if let Some(gutter) = page.two_column {
            // The index's own heading spans both columns.
            for (block, ident) in page.blocks.iter().zip(idents) {
                if block.kind == Kind::Heading {
                    let level = heading_level(&block_text(block));
                    let _ = write!(out, "<h{level}");
                    if let Some(id) = &ident.id {
                        let _ = write!(out, " id=\"{id}\"");
                    }
                    let _ = write!(out, " style=\"font-size:{}\">", pt(block.lines[0].size));
                    escape(block_text(block).trim(), out);
                    let _ = writeln!(out, "</h{level}>");
                }
            }
            self.index_columns(gutter, out);
        } else {
            self.blocks(idents, prev_last_kind, out);
        }
        out.push_str("</div>\n");
        if let Some(rule) = &page.separator {
            let _ = writeln!(
                out,
                "<div class=\"sep\" style=\"left:{};top:{};width:{}\"></div>",
                pt(rule.x0),
                pt(page.height - rule.y1),
                pt(rule.x1 - rule.x0)
            );
        }
        if !page.footnotes.is_empty() {
            self.footnotes(out);
        }
        for footer in &page.footer {
            self.running(footer, "foot", out);
        }
        out.push_str("</section>\n");
    }

    fn running(&self, line: &Line, class: &str, out: &mut String) {
        let top = self.page.height - line.y - above(line.size, line_height(line.size));
        let _ = write!(
            out,
            "<div class=\"run {class}\" style=\"left:{};top:{};width:{}\">",
            pt(self.page.text_left),
            pt(top),
            pt(self.page.text_right - self.page.text_left)
        );
        // Each part sits where it was printed.
        let mut parts: Vec<&Span> = line.spans.iter().collect();
        parts.retain(|s| !s.text.trim().is_empty());
        for group in split_groups(&parts) {
            let _ = write!(
                out,
                "<span style=\"left:{}\">",
                pt(group[0].x0 - self.page.text_left)
            );
            for (n, s) in group.iter().enumerate() {
                let mut s = (*s).clone();
                if n + 1 == group.len() {
                    s.text = s.text.trim_end().to_owned();
                }
                self.inline_span(&s, false, out);
            }
            out.push_str("</span> ");
        }
        out.push_str("</div>\n");
    }

    fn margin(&self, gap: f64, prev: Option<(f64, f64)>, this: (f64, f64)) -> f64 {
        match prev {
            | Some((ps, plh)) => gap - below(ps, plh) - above(this.0, this.1),
            | None => 0.0,
        }
    }

    fn blocks(&self, idents: &[Ident], prev_last_kind: Option<Kind>, out: &mut String) {
        let blocks = &self.page.blocks;
        let mut open_container: Option<&str> = None;
        let mut prev_metrics: Option<(f64, f64)> = None;
        let mut i = 0;
        while i < blocks.len() {
            let block = &blocks[i];
            let ident = &idents[i];
            // Close or open the numbered-paragraph container.
            let wants = ident.container.as_deref();
            if open_container.is_some() && (wants != open_container || block.number.is_some()) {
                out.push_str("</div>\n");
                open_container = None;
            }
            if open_container.is_none()
                && let Some(id) = wants
            {
                if block.number.is_some() {
                    let _ = writeln!(
                        out,
                        "<div class=\"para\" id=\"{id}\"><span class=\"pn\">{}</span>",
                        block.number.as_deref().unwrap_or_default()
                    );
                } else {
                    let _ = writeln!(out, "<div class=\"para cont\" data-of=\"{id}\">");
                }
                open_container = Some(id);
            }
            let size = block.lines[0].size;
            let metrics = (size, line_height(size));
            let margin = self.margin(block.gap, prev_metrics, metrics);
            let continues_text = i == 0
                && ident.continues
                && block.kind == Kind::Para
                && prev_last_kind == Some(Kind::Para);
            match block.kind {
                | Kind::ListItem => {
                    // Consecutive items form one list.
                    out.push_str("<ul class=\"dash\">\n");
                    let mut j = i;
                    let mut m = margin;
                    while j < blocks.len()
                        && blocks[j].kind == Kind::ListItem
                        && (j == i || blocks[j].number.is_none())
                        && idents[j].container == ident.container
                    {
                        let b = &blocks[j];
                        if j > i {
                            let s = b.lines[0].size;
                            m = self.margin(b.gap, prev_metrics, (s, line_height(s)));
                        }
                        out.push_str("<li");
                        self.block_attrs(b, m, b.lines[0].x0 - self.page.text_left, out);
                        self.open_end(&b.lines, out);
                        let base = b.lines.get(1).map_or(b.lines[0].x0 + 18.0, |l| l.x0);
                        self.prose(b, true, base, out);
                        out.push_str("</li>\n");
                        let last = b.lines.last().expect("blocks have lines");
                        prev_metrics = Some((last.size, line_height(last.size)));
                        j += 1;
                    }
                    out.push_str("</ul>\n");
                    i = j;
                    continue;
                },
                | Kind::Heading => {
                    let level = heading_level(&block_text(block));
                    let _ = write!(out, "<h{level}");
                    self.block_attrs(block, 6.0, 0.0, out);
                    if let Some(id) = &ident.id {
                        let _ = write!(out, " id=\"{id}\"");
                    }
                    let mut style = format!("font-size:{}", pt(size));
                    if margin.abs() > 0.05 {
                        let _ = write!(style, ";margin-top:{}", pt(margin));
                    }
                    let centered = block.lines.iter().all(|l| {
                        let left = l.x0 - self.page.text_left;
                        let right = self.page.text_right - l.x1;
                        left > 20.0 && (left - right).abs() < 12.0
                    });
                    if centered {
                        style.push_str(";text-align:center");
                    }
                    let _ = write!(out, " style=\"{style}\">");
                    for (k, line) in block.lines.iter().enumerate() {
                        if k > 0 {
                            out.push_str("<br>");
                        }
                        for s in &line.spans {
                            // A heading's roman parts, such as an annex's
                            // "(informative)", keep their printed weight.
                            if s.style.bold {
                                self.inline_span(s, true, out);
                            } else {
                                out.push_str("<span class=\"rm\">");
                                self.inline_span(s, true, out);
                                out.push_str("</span>");
                            }
                        }
                    }
                    let _ = writeln!(out, "</h{level}>");
                },
                | Kind::Sub => {
                    out.push_str("<h6 class=\"sub\"");
                    self.block_attrs(block, margin, 0.0, out);
                    out.push('>');
                    escape(block_text(block).trim(), out);
                    out.push_str("</h6>\n");
                },
                | Kind::Para => {
                    out.push_str("<p");
                    if continues_text {
                        out.push_str(" class=\"cont\"");
                    }
                    // The body of the paragraph sets the left margin; each
                    // line is placed against it, so a first line that hangs
                    // (a term before its definition) or is indented keeps it.
                    let rest = block
                        .lines
                        .get(1)
                        .unwrap_or(&block.lines[0])
                        .x0
                        .max(self.page.text_left);
                    self.block_attrs(block, margin, rest - self.page.text_left, out);
                    self.open_end(&block.lines, out);
                    self.prose(block, false, rest, out);
                    out.push_str("</p>\n");
                },
                | Kind::Code => self.code(block, margin, out),
                | Kind::Syntax => self.syntax(block, margin, out),
                | Kind::Layout => self.layout(block, margin, out),
                | Kind::Toc => self.toc(block, margin, out),
                | Kind::Index => self.index_line(&block.lines[0], margin, out),
            }
            let last = block.lines.last().expect("blocks have lines");
            prev_metrics = Some((last.size, line_height(last.size)));
            i += 1;
        }
        if open_container.is_some() {
            out.push_str("</div>\n");
        }
    }

    fn block_attrs(&self, block: &Block, margin: f64, indent: f64, out: &mut String) {
        if std::env::var_os("STANDARDS_HTML_DEBUG").is_some() {
            let first = &block.lines[0];
            let top = self.page.height - first.y - above(first.size, line_height(first.size));
            let _ = write!(out, " data-ey=\"{top:.1}\"");
        }
        let mut style = String::new();
        if (margin - 6.0).abs() > 0.3 {
            let _ = write!(style, "margin-top:{}", pt(margin));
        }
        if indent > 1.0 {
            if !style.is_empty() {
                style.push(';');
            }
            let _ = write!(style, "margin-left:{}", pt(indent));
        }
        let size = block.lines[0].size;
        // Prose is set line for line, so its lines are spaced as printed: the
        // block's median baseline pitch, where that differs from the default.
        let mut lead = line_height(size);
        if matches!(block.kind, Kind::Para | Kind::ListItem) && block.lines.len() > 1 {
            let mut pitches: Vec<f64> = block.lines.windows(2).map(|w| w[0].y - w[1].y).collect();
            pitches.sort_by(f64::total_cmp);
            let pitch = pitches[pitches.len() / 2];
            if pitch > 0.5 * size && (pitch - lead).abs() > 0.5 {
                lead = pitch;
            }
        }
        if block.kind != Kind::Heading
            && ((size - layout::metrics().body).abs() > 0.3
                || (lead - line_height(size)).abs() > 0.05)
        {
            if !style.is_empty() {
                style.push(';');
            }
            let _ = write!(style, "font-size:{};line-height:{}", pt(size), pt(lead));
        }
        if !style.is_empty() {
            let _ = write!(out, " style=\"{style}\"");
        }
    }

    /// Closes an opening tag and adds the change bars and deletion marks
    /// that fall beside `lines`, positioned from the element's top.
    fn open_end(&self, lines: &[Line], out: &mut String) {
        out.push('>');
        let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
            return;
        };
        let top_y = first.y + above(first.size, line_height(first.size));
        let bottom_y = last.y - below(last.size, line_height(last.size));
        for &(y0, y1) in &self.page.change_bars {
            let (y0, y1) = (y0.max(bottom_y), y1.min(top_y));
            if y1 > y0 {
                let _ = write!(
                    out,
                    "<i class=\"cb\" style=\"top:{};height:{}\"></i>",
                    pt(top_y - y1),
                    pt(y1 - y0)
                );
            }
        }
        for line in lines.iter().filter(|l| l.deleted) {
            let top = top_y - line.y - above(line.size, line_height(line.size));
            let _ = write!(out, "<i class=\"dm\" style=\"top:{}\"></i>", pt(top));
        }
    }

    /// Flowing text: lines joined with spaces, line-end hyphens resolved,
    /// and code lines kept on their own lines.
    /// Flowing text set line for line as printed: each line is a justified
    /// block, so pages break where the PDF's do whatever the font. Lines are
    /// joined by a space inside the earlier line, so the text reads on; a
    /// hyphen dropped at a line break is drawn by CSS rather than written.
    /// `base` is where an unindented line starts, in PDF coordinates.
    fn prose(&self, block: &Block, list_item: bool, base: f64, out: &mut String) {
        let count = block.lines.len();
        for (k, line) in block.lines.iter().enumerate() {
            let mut spans: Vec<Span> = line.spans.clone();
            if k == 0
                && list_item
                && let Some(first) = spans.first_mut()
            {
                let t = first.text.trim_start();
                first.text = t.strip_prefix('—').unwrap_or(t).trim_start().to_owned();
            }
            let code_line = k > 0 && is_code_line(line);
            let next = block.lines.get(k + 1);
            let dropped = !code_line && self.hyphen_dropped(block, k);
            let kept = !dropped
                && next.is_some()
                && !code_line
                && self.hyphen_joins(&line.text(), "", &mut String::new());
            // Justified text fills every line but the last, so a line that
            // stops short ended there on purpose and is not stretched.
            let end = k + 1 == count
                || code_line
                || next.is_some_and(is_code_line)
                || line.x1 < self.page.text_right - 24.0;
            let mut class = String::from("l");
            if code_line {
                class.push_str(" cl");
            }
            if end {
                class.push_str(" e");
            }
            if dropped {
                class.push_str(" hy");
            }
            let _ = write!(out, "<span class=\"{class}\"");
            let offset = if k == 0 && list_item {
                0.0
            } else {
                line.x0 - base
            };
            if offset.abs() > 1.5 {
                let _ = write!(out, " style=\"margin-left:{}\"", pt(offset));
            }
            out.push('>');
            for (n, s) in spans.iter().enumerate() {
                let mut s = s.clone();
                if n + 1 == spans.len() && !code_line {
                    s.text = s.text.trim_end().to_owned();
                    if dropped {
                        s.text.pop();
                    }
                }
                if s.gap > 0.0 && n > 0 {
                    out.push(' ');
                }
                self.inline_span(&s, false, out);
            }
            if next.is_some() && !dropped && !kept {
                out.push(' ');
            }
            out.push_str("</span>");
        }
    }

    /// Writes nothing; reports whether line `prev` ends in a hyphen that
    /// joins directly to the next line (so no space is wanted).
    fn hyphen_joins(&self, prev: &str, _next: &str, _out: &mut String) -> bool {
        let prev = prev.trim_end();
        prev.ends_with('-') && prev[..prev.len() - 1].ends_with(|c: char| c.is_alphabetic())
    }

    /// A line-end hyphen is dropped when the joined word occurs elsewhere
    /// unhyphenated and the hyphenated compound does not.
    fn hyphen_dropped(&self, block: &Block, k: usize) -> bool {
        let Some(next) = block.lines.get(k + 1) else {
            return false;
        };
        let prev = block.lines[k].text();
        let prev = prev.trim_end();
        if !self.hyphen_joins(prev, "", &mut String::new()) {
            return false;
        }
        let head: String = prev[..prev.len() - 1]
            .chars()
            .rev()
            .take_while(|c| c.is_alphabetic())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let next_text = next.text();
        let tail: String = next_text
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphabetic())
            .collect();
        if head.is_empty() || tail.is_empty() || !tail.starts_with(|c: char| c.is_lowercase()) {
            return false;
        }
        let joined = format!("{head}{tail}").to_lowercase();
        let compound = format!("{head}-{tail}").to_lowercase();
        self.doc.words.get(&joined).copied().unwrap_or(0) > 0
            && self.doc.words.get(&compound).copied().unwrap_or(0) <= 1
    }

    /// Writes a span, inside `<ins>` or `<del>` when it is a diff mark.
    fn inline_span(&self, span: &Span, heading: bool, out: &mut String) {
        let (open, close) = match span.mark {
            | Mark::None => ("", ""),
            | Mark::Inserted => ("<ins>", "</ins>"),
            | Mark::Deleted => ("<del>", "</del>"),
        };
        out.push_str(open);
        self.unmarked_span(span, heading, out);
        out.push_str(close);
    }

    fn unmarked_span(&self, span: &Span, heading: bool, out: &mut String) {
        let text = span.text.as_str();
        if text.is_empty() {
            return;
        }
        let mono = span.style.family == Family::Mono;
        let footnote_ref = span.shift == Shift::Super
            && text
                .trim()
                .strip_suffix(')')
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        let (open, close): (&str, &str) = match span.shift {
            | Shift::Super => ("<sup>", "</sup>"),
            | Shift::Sub => ("<sub>", "</sub>"),
            | Shift::Base => ("", ""),
        };
        out.push_str(open);
        if footnote_ref {
            let n = text.trim().trim_end_matches(')');
            let _ = write!(out, "<a href=\"#fn{n}\">{n})</a>");
            if text.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(close);
            return;
        }
        let trailing = text.len() - text.trim_end().len();
        let body = &text[..text.len() - trailing];
        if mono {
            // Code is set in the document's usual code weight; a span in the
            // other weight (a bold keyword in C23's synopses) says so.
            let usual_bold = self.doc.fonts.mono_weight == "bold";
            out.push_str(if span.style.bold == usual_bold {
                "<code>"
            } else {
                "<code class=\"w\">"
            });
        }
        if span.style.bold && !mono && !heading {
            out.push_str("<b>");
        }
        if span.style.italic {
            out.push_str("<i>");
        }
        if mono || heading {
            escape(body, out);
        } else {
            self.linked(body, out);
        }
        if span.style.italic {
            out.push_str("</i>");
        }
        if span.style.bold && !mono && !heading {
            out.push_str("</b>");
        }
        if mono {
            out.push_str("</code>");
        }
        out.push_str(close);
        out.push_str(&text[text.len() - trailing..]);
    }

    /// Escapes prose and links clause references such as `6.7.2` or `A.1`.
    fn linked(&self, text: &str, out: &mut String) {
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let boundary = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '.');
            if boundary && (chars[i].is_ascii_digit() || chars[i].is_ascii_uppercase()) {
                let mut j = i;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.') {
                    j += 1;
                }
                let mut token: String = chars[i..j].iter().collect();
                while token.ends_with('.') {
                    token.pop();
                }
                let len = token.chars().count();
                let after_ok = chars.get(i + len).is_none_or(|c| !c.is_alphanumeric());
                if token.contains('.') && after_ok && self.doc.ids.contains(&token) {
                    let _ = write!(out, "<a href=\"#{token}\">{token}</a>");
                    i += len;
                    continue;
                }
            }
            escape(&chars[i].to_string(), out);
            i += 1;
        }
    }

    fn code(&self, block: &Block, margin: f64, out: &mut String) {
        let left = block
            .lines
            .iter()
            .map(|l| l.x0)
            .fold(f64::INFINITY, f64::min);
        out.push_str("<pre");
        self.block_attrs(block, margin, left - self.page.text_left, out);
        self.open_end(&block.lines, out);
        for (k, line) in block.lines.iter().enumerate() {
            // Whole blank lines stay blank lines; leftover spacing (code set
            // at one and a half line spacing) becomes a margin on the line.
            let mut extra = 0.0;
            if k > 0 {
                let pitch = line_height(line.size);
                let space = block.lines[k - 1].y - line.y - pitch;
                let blank = ((space + 0.5) / pitch).floor().max(0.0);
                extra = space - blank * pitch;
                for _ in 0..=(blank as usize) {
                    out.push('\n');
                }
            }
            if extra > 0.5 {
                let _ = write!(
                    out,
                    "<span class=\"pl\" style=\"margin-top:{}\">",
                    pt(extra)
                );
            }
            let cell = 0.6 * line.size;
            let indent = ((line.x0 - left) / cell).round().max(0.0) as usize;
            out.extend(std::iter::repeat_n(' ', indent));
            for (n, s) in line.spans.iter().enumerate() {
                if s.gap > 0.0 && n > 0 {
                    let spaces = (s.gap / cell).round().max(1.0) as usize;
                    out.extend(std::iter::repeat_n(' ', spaces));
                }
                let mut s = s.clone();
                if n + 1 == line.spans.len() {
                    s.text = s.text.trim_end().to_owned();
                }
                self.inline_span(&s, false, out);
            }
            if extra > 0.5 {
                out.push_str("</span>");
            }
        }
        out.push_str("</pre>\n");
    }

    fn syntax(&self, block: &Block, margin: f64, out: &mut String) {
        out.push_str("<div class=\"syn\"");
        self.block_attrs(block, margin, 0.0, out);
        self.open_end(&block.lines, out);
        out.push('\n');
        for (k, line) in block.lines.iter().enumerate() {
            let gap = if k == 0 {
                0.0
            } else {
                block.lines[k - 1].y - line.y - line_height(line.size)
            };
            let indent = line.x0 - self.page.text_left;
            if line.max_gap > 0.0 {
                self.positioned_line(line, gap, out);
                continue;
            }
            out.push_str("<div");
            let mut style = String::new();
            if indent > 1.0 {
                let _ = write!(style, "margin-left:{}", pt(indent));
            }
            if gap > 0.5 {
                if !style.is_empty() {
                    style.push(';');
                }
                let _ = write!(style, "margin-top:{}", pt(gap));
            }
            if !style.is_empty() {
                let _ = write!(out, " style=\"{style}\"");
            }
            out.push('>');
            for s in &line.spans {
                self.inline_span(s, false, out);
            }
            out.push_str("</div>\n");
        }
        out.push_str("</div>\n");
    }

    /// One line whose spans are placed at their printed horizontal offsets.
    fn positioned_line(&self, line: &Line, gap: f64, out: &mut String) {
        let lh = line_height(line.size);
        let _ = write!(out, "<div class=\"ln\" style=\"height:{}", pt(lh));
        if gap > 0.5 {
            let _ = write!(out, ";margin-top:{}", pt(gap));
        }
        out.push_str("\">");
        let top_y = line.y + above(line.size, lh);
        for group in split_groups(&line.spans.iter().collect::<Vec<_>>()) {
            let first = group[0];
            let top = top_y - first.y - above(first.size, first.size);
            let _ = write!(
                out,
                "<span style=\"left:{};top:{}\">",
                pt(first.x0 - self.page.text_left),
                pt(top)
            );
            for s in group {
                self.inline_span(s, false, out);
            }
            out.push_str("</span> ");
        }
        out.push_str("</div>\n");
    }

    /// Tables and formulas: every span placed where it was printed, relative
    /// to the block's top left corner, plus the block's rules.
    fn layout(&self, block: &Block, margin: f64, out: &mut String) {
        let first = &block.lines[0];
        let last = block.lines.last().expect("blocks have lines");
        let top_y = first.y + above(first.size, line_height(first.size));
        let bottom_y = last.y - below(last.size, line_height(last.size));
        let height = top_y - bottom_y;
        out.push_str("<div class=\"lay\"");
        // Height is required because every child is absolutely positioned.
        let mut attrs = String::new();
        self.block_attrs(block, margin, 0.0, &mut attrs);
        match attrs.find(" style=\"") {
            | Some(at) => {
                let insert = at + " style=\"".len();
                attrs.insert_str(insert, &format!("height:{};", pt(height)));
            },
            | None => {
                let _ = write!(attrs, " style=\"height:{}\"", pt(height));
            },
        }
        out.push_str(&attrs);
        self.open_end(&block.lines, out);
        for line in &block.lines {
            for span in &line.spans {
                let mut s = span.clone();
                s.text = s.text.trim_end().to_owned();
                if s.text.is_empty() {
                    continue;
                }
                // Each span carries its own position, so shifts are explicit.
                s.shift = Shift::Base;
                let top = top_y - s.y - above(s.size, s.size);
                let _ = write!(
                    out,
                    "<span style=\"left:{};top:{}",
                    pt(s.x0 - self.page.text_left),
                    pt(top)
                );
                if (s.size - layout::metrics().body).abs() > 0.3 {
                    let _ = write!(out, ";font-size:{}", pt(s.size));
                }
                out.push_str("\">");
                self.inline_span(&s, false, out);
                // The space keeps neighbouring words apart in extracted text.
                out.push_str("</span> ");
            }
            // The line break separates the last span from the next line's.
            out.truncate(out.trim_end_matches(' ').len());
            out.push('\n');
        }
        for r in &block.rules {
            let _ = write!(
                out,
                "<i class=\"rule\" style=\"left:{};top:{};width:{};height:{}\"></i>",
                pt(r.x0 - self.page.text_left),
                pt(top_y - r.y1),
                pt((r.x1 - r.x0).max(0.5)),
                pt((r.y1 - r.y0).max(0.5))
            );
        }
        out.push_str("</div>\n");
    }

    fn toc(&self, block: &Block, margin: f64, out: &mut String) {
        for (k, line) in block.lines.iter().enumerate() {
            // The page number ends the line; leaders, alone in a span or
            // trailing the title, end where the number's column begins.
            let mut title: Vec<Span> = line.spans.clone();
            let mut page_no = None;
            if let Some(last) = title.last_mut()
                && let Some((before, number)) = split_page_number(&last.text)
            {
                let number = number.to_owned();
                let before = before.to_owned();
                page_no = Some(number.clone());
                if before.is_empty() {
                    title.pop();
                } else {
                    // The number ran into the leaders; the leaders end a
                    // number's width before the margin.
                    last.text = before;
                    last.x1 -= 0.5 * last.size * (number.len() as f64 + 1.0);
                }
            }
            let mut leader_end = None;
            title.retain(|s| {
                let leaders_only = s.text.trim().chars().all(|c| c == '.' || c == ' ');
                if has_leaders(s) {
                    leader_end = Some(s.x1);
                }
                !(leaders_only && has_leaders(s))
            });
            if let Some(last) = title.last_mut() {
                last.text = last.text.trim_end_matches(['.', ' ']).to_owned();
            }
            let indent = line.x0 - self.page.text_left;
            // A number long enough to narrow the tab gap was set with a plain
            // space; give it the tab stop that shorter numbers at this level
            // use.
            if title.get(1).is_none_or(|s| s.gap == 0.0)
                && let Some(first) = title.first()
                && let Some(number) = clause_number(first.text.trim_start())
                && first.text.trim_start()[number.len()..].starts_with(' ')
                && let Some(width) = self.doc.tab_stop(indent)
            {
                let number = number.to_owned();
                let mut rest = first.clone();
                rest.text = first.text.trim_start()[number.len()..]
                    .trim_start()
                    .to_owned();
                rest.x0 = first.x0 + width;
                rest.gap = 1.0;
                title[0].text = number;
                title.insert(1, rest);
            }
            let title: Vec<&Span> = title.iter().collect();
            let text: String = title
                .iter()
                .enumerate()
                .map(|(n, s)| {
                    if n > 0 && s.gap > 0.0 {
                        format!(" {}", s.text)
                    } else {
                        s.text.clone()
                    }
                })
                .collect();
            out.push_str("<div class=\"toc\"");
            let m = if k == 0 { margin } else { 6.0 };
            self.block_attrs(block, m, indent, out);
            out.push('>');
            let target = heading_id(text.trim());
            if self.doc.ids.contains(&target) {
                let _ = write!(out, "<a href=\"#{target}\">");
            } else {
                out.push_str("<span>");
            }
            for (n, s) in title.iter().enumerate() {
                let mut s = (*s).clone();
                if n + 1 == title.len() {
                    s.text = s.text.trim_end().to_owned();
                }
                // A clause number set apart from its title by a tab stop keeps
                // that width, so titles at one level line up as printed.
                let tabbed = n == 0 && title.get(1).is_some_and(|next| next.gap > 0.0);
                if tabbed {
                    let width = title[1].x0 - s.x0;
                    let _ = write!(out, "<span class=\"no\" style=\"width:{}\">", pt(width));
                    s.text = s.text.trim_end().to_owned();
                }
                self.inline_span(&s, true, out);
                if tabbed {
                    out.push_str("</span> ");
                }
            }
            out.push_str(if self.doc.ids.contains(&target) {
                "</a>"
            } else {
                "</span>"
            });
            // Entries printed without leaders (C23's chapters) get a plain gap.
            out.push_str(if leader_end.is_some() {
                " <span class=\"dots\"></span> "
            } else {
                " <span class=\"dots none\"></span> "
            });
            if let Some(p) = page_no {
                // The page number column starts where the printed leaders end,
                // so the dot grid lines up from the right as printed.
                let width = leader_end.map_or(18.0, |end| self.page.text_right - end);
                let style = format!(" style=\"width:{}\"", pt(width));
                match self.doc.printed.get(&p) {
                    | Some(n) => {
                        let _ = write!(out, "<a class=\"pg\" href=\"#page-{n}\"{style}>{p}</a>");
                    },
                    | None => {
                        let _ = write!(out, "<span class=\"pg\"{style}>{p}</span>");
                    },
                }
            }
            out.push_str("</div>\n");
        }
    }

    fn index_line(&self, line: &Line, margin: f64, out: &mut String) {
        out.push_str("<div class=\"ix\"");
        let mut style = String::new();
        if (margin).abs() > 0.3 {
            let _ = write!(style, "margin-top:{}", pt(margin));
        }
        let _ = write!(
            out,
            "{}",
            if style.is_empty() {
                String::new()
            } else {
                format!(" style=\"{style}\"")
            }
        );
        out.push('>');
        for (n, s) in line.spans.iter().enumerate() {
            let mut s = s.clone();
            if n + 1 == line.spans.len() {
                s.text = s.text.trim_end().to_owned();
            }
            if s.gap > 0.0 && n > 0 {
                out.push(' ');
            }
            self.inline_span(&s, false, out);
        }
        out.push_str("</div>\n");
    }

    fn index_columns(&self, gutter: f64, out: &mut String) {
        let page = self.page;
        out.push_str("<div class=\"cols\">\n");
        for side in [true, false] {
            let lines: Vec<&Line> = page
                .blocks
                .iter()
                .filter(|b| b.kind != Kind::Heading)
                .flat_map(|b| b.lines.iter())
                .filter(|l| (l.x0 < gutter) == side)
                .collect();
            let col_left = if side {
                page.text_left
            } else {
                lines.iter().map(|l| l.x0).fold(f64::INFINITY, f64::min)
            };
            let col_top = lines.first().map_or(page.top, |l| l.y);
            out.push_str("<div class=\"col\"");
            let first_offset = page.top - col_top;
            if first_offset.abs() > 0.3 {
                let _ = write!(out, " style=\"padding-top:{}\"", pt(first_offset));
            }
            out.push_str(">\n");
            let mut prev: Option<&Line> = None;
            for line in lines {
                let margin = prev.map_or(0.0, |p| p.y - line.y - line_height(line.size));
                out.push_str("<div class=\"ix\"");
                let mut style = String::new();
                if margin.abs() > 0.3 {
                    let _ = write!(style, "margin-top:{}", pt(margin));
                }
                let indent = line.x0 - col_left;
                if indent > 1.0 {
                    if !style.is_empty() {
                        style.push(';');
                    }
                    let _ = write!(style, "padding-left:{}", pt(indent));
                }
                if !style.is_empty() {
                    let _ = write!(out, " style=\"{style}\"");
                }
                out.push('>');
                for (n, s) in line.spans.iter().enumerate() {
                    let mut s = s.clone();
                    if n + 1 == line.spans.len() {
                        s.text = s.text.trim_end().to_owned();
                    }
                    self.inline_span(&s, false, out);
                }
                out.push_str("</div>\n");
                prev = Some(line);
            }
            out.push_str("</div>\n");
        }
        out.push_str("</div>\n");
    }

    fn footnotes(&self, out: &mut String) {
        let page = self.page;
        let first = &page.footnotes[0].lines[0];
        let top = page.height - first.y - above(first.size, line_height(first.size));
        let _ = writeln!(
            out,
            "<div class=\"notes\" style=\"left:{};top:{};width:{}\">",
            pt(page.text_left),
            pt(top),
            pt(page.text_right - page.text_left)
        );
        let mut prev_y: Option<f64> = None;
        for note in &page.footnotes {
            out.push_str("<p class=\"fn\"");
            if let Some(n) = &note.number {
                let _ = write!(out, " id=\"fn{n}\"");
            } else {
                out.push_str(" data-cont=\"\"");
            }

            let first = &note.lines[0];
            let rest = note.lines.get(1).unwrap_or(first).x0.max(page.text_left);
            let mut style = Vec::new();
            if let Some(py) = prev_y {
                let m = py - first.y - line_height(first.size);
                if m.abs() > 0.3 {
                    style.push(format!("margin-top:{}", pt(m)));
                }
            }
            if rest - page.text_left > 1.0 {
                style.push(format!("margin-left:{}", pt(rest - page.text_left)));
            }
            if !style.is_empty() {
                let _ = write!(out, " style=\"{}\"", style.join(";"));
            }
            self.open_end(&note.lines, out);
            let block = Block {
                kind:   Kind::Para,
                lines:  note.lines.clone(),
                number: None,
                gap:    0.0,
                rules:  Vec::new(),
            };
            self.prose(&block, false, rest, out);
            out.push_str("</p>\n");
            prev_y = note.lines.last().map(|l| l.y);
        }
        out.push_str("</div>\n");
    }
}

/// A line is code when its first visible span is monospace and most of its
/// text is monospace.
fn is_code_line(line: &Line) -> bool {
    let first = line.spans.iter().find(|s| !s.text.trim().is_empty());
    let (mono, total) = line.spans.iter().fold((0, 0), |(m, t), s| {
        let n = s.text.trim().chars().count();
        (
            m + if s.style.family == Family::Mono { n } else { 0 },
            t + n,
        )
    });
    first.is_some_and(|s| s.style.family == Family::Mono) && mono * 10 >= total * 6
}

/// Splits spans into groups separated by wide gaps (table cells).
fn split_groups<'s>(spans: &[&'s Span]) -> Vec<Vec<&'s Span>> {
    let mut groups: Vec<Vec<&Span>> = Vec::new();
    for s in spans {
        if groups.is_empty() || s.gap > 0.0 {
            groups.push(vec![s]);
        } else {
            groups.last_mut().expect("nonempty").push(s);
        }
    }
    groups
}

fn heading_level(text: &str) -> usize {
    match clause_number(text.trim()) {
        | Some(n) => (n.split('.').count() + 1).min(6),
        | None => 2,
    }
}

const STYLE: &str = r#":root {
  --desk: #d8d8d8;
  --paper: #fff;
  --ink: #000;
  --muted: #444;
  color-scheme: light;
}
@media (prefers-color-scheme: dark) {
  :root { --desk: #2b2b2b; }
}
* { box-sizing: border-box; }
html { background: var(--desk); }
body { margin: 0; padding: 16pt 0; }
.page {
  position: relative;
  margin: 0 auto 16pt;
  overflow: hidden;
  background: var(--paper);
  color: var(--ink);
  box-shadow: 0 1pt 4pt rgb(0 0 0 / 35%);
  break-after: page;
  font: var(--body)/var(--lead) var(--serif);
  font-kerning: none;
}
.run, .body, .notes { position: absolute; }
.run { height: 15pt; white-space: nowrap; }
.run > span { position: absolute; top: 0; }
.foot .pg { font-weight: normal; }
.body p, .body h2, .body h3, .body h4, .body h5, .body h6, .body li, .body pre,
.body .syn, .body .lay, .body .toc { margin: 6pt 0 0; }
.body ul { margin: 0; }
.body > :first-child, .body > .para:first-child > :nth-child(2) { margin-top: 0; }
p { text-align: justify; }
p.cont { text-indent: 0; }
h2, h3, h4, h5, h6 { font-weight: bold; line-height: 1.25; }
h6.sub { font-size: var(--body); }
.rm { font-weight: normal; }
/* Diff marks, as N2310 prints them against C17. */
ins { color: #00f; text-decoration: underline; }
del { color: #e00; text-decoration: line-through; }
b { font-weight: bold; }
code, pre { font-family: var(--mono); font-size: 1em; font-weight: var(--mono-weight); }
code.w { font-weight: var(--mono-other); }
code { line-height: 0; }
pre { white-space: pre; line-height: var(--lead); }
pre .pl { display: inline-block; }
/* Prose is set line for line as printed: each line a justified block that
   keeps to one line, a short or final line left as it is. */
.l { display: block; white-space: nowrap; text-align-last: justify; }
.l.e { text-align-last: auto; }
.l.hy::after { content: "-"; }
.l.cl { white-space: pre; }
sup, sub { font-size: 9pt; line-height: 0; }
a { color: inherit; text-decoration: none; }
a:hover, a:hover .no { text-decoration: underline; }
.para { position: relative; }
.pn { position: absolute; left: calc(-1 * var(--pn)); top: 0; }
ul.dash { list-style: none; padding: 0; }
ul.dash > li { position: relative; padding-left: 18pt; text-align: justify; }
ul.dash > li::before { content: "—"; position: absolute; left: 0; }
.syn > div { text-align: left; }
.ln, .lay { position: relative; }
.ln > span, .lay > span { position: absolute; white-space: pre; line-height: 1; }
.lay .rule { position: absolute; background: var(--ink); }
.toc { display: flex; align-items: baseline; }
/* Leaders: bold dots on the printed 12pt grid, anchored at the right edge so
   they line up from entry to entry. */
.toc .dots {
  flex: 1;
  align-self: stretch;
  margin-left: 9pt;
  background: radial-gradient(circle at calc(var(--leader) - 1.5pt) 50%, var(--ink) 0.8pt, transparent 1pt) right bottom 2.6pt / var(--leader) 3pt repeat-x;
  print-color-adjust: exact;
  -webkit-print-color-adjust: exact;
}
.toc .dots.none { background: none; }
.toc .pg { flex: none; text-align: right; }
.toc .no { display: inline-block; }
.cols { display: grid; grid-template-columns: 1fr 1fr; column-gap: 12pt; }
.ix { font-size: var(--index); line-height: var(--index-lead); text-indent: 0; }
.notes { font-size: var(--note); line-height: var(--note-lead); }
.sep { position: absolute; border-top: 0.4pt solid var(--ink); }
.notes .fn { margin: 0; text-align: justify; }
.notes sup, .notes sub { font-size: 7.5pt; }
p, li, pre, h6, .syn, .toc, .ix { position: relative; }
.cb { position: absolute; right: -10pt; border-right: 0.5pt solid var(--ink); }
.dm { position: absolute; right: -18pt; font-style: normal; }
.dm::before { content: "∗"; }
@media print {
  html, body { background: none; padding: 0; }
  .page { margin: 0; box-shadow: none; }
}
@media screen and (max-width: 660px) {
  body { padding: 0; }
  .doc > .page { width: auto !important; height: auto !important; margin: 0 0 12px; padding: 16px; box-shadow: none; overflow: visible; }
  .doc .run, .doc .body, .doc .notes { position: static; width: auto !important; }
  .doc .run { font-size: 10pt; color: var(--muted); height: auto; display: flex; flex-wrap: wrap; justify-content: space-between; gap: 0 8px; white-space: normal; }
  .doc .run > span { position: static; }
  .doc .body { margin: 8px 0; }
  .doc .pn { position: static; font-weight: bold; margin-right: 6pt; }
  .doc .l { display: inline; white-space: normal; margin-left: 0 !important; }
  .doc .l.hy::after { content: none; }
  .doc .l.cl { display: block; white-space: pre; }
  .doc .lay, .doc .ln, .doc pre, .doc .syn { overflow-x: auto; }
  .doc .notes { margin-top: 12px; border-top: 1px dashed var(--muted); padding-top: 6px; }
  .doc .sep { display: none; }
  .doc .cols { grid-template-columns: 1fr; }
}
"#;

#[cfg(test)]
mod tests {
    use super::{
        heading_id,
        heading_level,
        paragraph_id,
    };

    #[test]
    fn heading_ids() {
        assert_eq!(heading_id("6.7.2 Type specifiers"), "6.7.2");
        assert_eq!(
            heading_id("Annex A (informative) Language syntax summary"),
            "A"
        );
        assert_eq!(
            heading_id("Programming languages — C"),
            "programming-languages-c"
        );
        assert_eq!(heading_id("Foreword"), "foreword");
    }

    #[test]
    fn paragraph_ids() {
        assert_eq!(paragraph_id("6.7.2", "2"), "6.7.2p2");
        assert_eq!(paragraph_id("A.1", "1"), "A.1p1");
        assert_eq!(paragraph_id("foreword", "3"), "foreword-p3");
    }

    #[test]
    fn font_stacks() {
        let troff = super::Fonts::for_names(
            [
                ("Times-Roman", 900),
                ("Courier-Bold", 300),
                ("Times-Bold", 40),
            ]
            .into_iter(),
        );
        assert!(troff.serif.starts_with("\"Times New Roman\""));
        assert!(troff.mono.starts_with("\"Courier New\""));
        assert_eq!(troff.mono_weight, "bold");
        let latex = super::Fonts::for_names(
            [
                ("URWPalladioL-Roma", 900),
                ("BeraSansMono-Roman", 300),
                ("BeraSansMono-Bold", 200),
            ]
            .into_iter(),
        );
        assert!(latex.serif.starts_with("\"URW Palladio L\""));
        assert!(latex.mono.starts_with("\"DejaVu Sans Mono\""));
        assert_eq!(latex.mono_weight, "normal");
    }

    #[test]
    fn heading_levels() {
        assert_eq!(heading_level("6 Language"), 2);
        assert_eq!(heading_level("6.7.2 Type specifiers"), 4);
        assert_eq!(heading_level("7.19.6.1.2.3 Deep"), 6);
    }
}
