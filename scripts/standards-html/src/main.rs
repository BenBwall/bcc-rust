//! Converts a WG14 C standard draft PDF into one static HTML document with
//! an element per PDF page.
//!
//! ```text
//! standards-html <input.pdf> <output.html> [--title <title>] [--diff-marks]
//! standards-html <input.pdf> --lines <page,page,...>
//! ```

mod analyze;
mod chrome;
mod html;
mod layout;
mod pdf;

use std::{
    collections::HashMap,
    process::ExitCode,
};

use lopdf::Document;

use crate::analyze::Kind;

fn usage() -> ExitCode {
    eprintln!(
        "usage: standards-html <input.pdf> <output.html> [--title <title>] [--diff-marks]\n       \
         standards-html <input.pdf> --lines <page,...>"
    );
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `--diff-marks`: the document shows changes as pure blue insertions and
    // pure red deletions (N2310, a diff of C2x against C17).
    let diff_marks = args.iter().any(|a| a == "--diff-marks");
    args.retain(|a| a != "--diff-marks");
    layout::set_metrics(layout::Metrics {
        diff_marks,
        ..layout::metrics()
    });
    let [input, rest @ ..] = args.as_slice() else {
        return usage();
    };
    let doc = match Document::load(input) {
        | Ok(doc) => doc,
        | Err(err) => {
            eprintln!("standards-html: cannot read {input}: {err}");
            return ExitCode::FAILURE;
        },
    };
    let mut unmapped = HashMap::new();
    let contents: Vec<pdf::PageContent> = doc
        .get_pages()
        .into_values()
        .map(|id| pdf::page_content(&doc, id, &mut unmapped))
        .collect();
    if !unmapped.is_empty() {
        let mut codes: Vec<_> = unmapped.into_iter().collect();
        codes.sort();
        for ((font, code), count) in codes {
            eprintln!(
                "standards-html: warning: {count} glyph(s) with unmapped code {code} in {font}"
            );
        }
    }
    let geometry = analyze::geometry(&contents);

    if let [flag, page] = rest
        && flag == "--fonts"
    {
        let id = doc.get_pages()[&page.parse::<u32>().unwrap_or(1)];
        for (name, font) in doc.get_page_fonts(id).unwrap_or_default() {
            let get = |k: &[u8]| {
                font.get(k)
                    .ok()
                    .map(|o| format!("{o:?}"))
                    .unwrap_or_default()
            };
            println!(
                "{}: {} {} encoding={} tounicode={}",
                String::from_utf8_lossy(&name),
                get(b"Subtype"),
                get(b"BaseFont"),
                get(b"Encoding"),
                font.has(b"ToUnicode")
            );
        }
        return ExitCode::SUCCESS;
    }

    if let [flag, pages] = rest
        && flag == "--lines"
    {
        for p in pages.split(',') {
            let n: usize = p.parse().unwrap_or(1);
            let page = analyze::page(&contents[n - 1], n, &geometry, false);
            println!(
                "=== page {n} printed {:?} left {} right {}",
                page.printed, page.text_left, page.text_right
            );
            for line in page.header.iter().chain(&page.footer) {
                println!(
                    "  furniture {:6.1} {:6.1} {:4.1} | {}",
                    line.y,
                    line.x0,
                    line.size,
                    line.text()
                );
            }
            if std::env::var_os("RULES").is_some() {
                for r in &contents[n - 1].rules {
                    println!("  rule {:.1},{:.1} - {:.1},{:.1}", r.x0, r.y0, r.x1, r.y1);
                }
            }
            for block in &page.blocks {
                println!(
                    "--- {:?} number={:?} gap={:.1}",
                    block.kind, block.number, block.gap
                );
                for line in &block.lines {
                    println!(
                        "  {:6.1} {:6.1} {:4.1} | {}",
                        line.y,
                        line.x0,
                        line.size,
                        line.text()
                    );
                    if std::env::var_os("SPANS").is_some() {
                        for s in &line.spans {
                            println!(
                                "        {:6.1}-{:6.1} gap {:4.1} {:?} {:?} {:?}",
                                s.x0, s.x1, s.gap, s.style, s.mark, s.text
                            );
                        }
                    }
                }
            }
            for note in &page.footnotes {
                println!("--- footnote {:?}", note.number);
                for line in &note.lines {
                    println!(
                        "  {:6.1} {:6.1} {:4.1} | {}",
                        line.y,
                        line.x0,
                        line.size,
                        line.text()
                    );
                }
            }
        }
        return ExitCode::SUCCESS;
    }

    let (output, title) = match rest {
        | [output] => (output, None),
        | [output, flag, title] if flag == "--title" => (output, Some(title.clone())),
        | _ => return usage(),
    };

    let mut pages: Vec<analyze::Page> = contents
        .iter()
        .enumerate()
        .map(|(i, c)| analyze::page(c, i + 1, &geometry, false))
        .collect();
    // The index at the end is set in two columns.
    if let Some(start) = pages.iter().rposition(|p| {
        p.blocks.first().is_some_and(|b| {
            b.kind == Kind::Heading && b.lines.iter().any(|l| l.text().trim() == "Index")
        })
    }) {
        for (i, content) in contents.iter().enumerate().skip(start) {
            let mut page = analyze::page(content, i + 1, &geometry, true);
            if i == start {
                // Keep the heading itself as a heading.
                if let Some(first) = page.blocks.first_mut()
                    && first.lines.iter().any(|l| l.text().trim() == "Index")
                {
                    first.kind = Kind::Heading;
                }
            }
            pages[i] = page;
        }
    }

    let title = title.unwrap_or_else(|| {
        std::path::Path::new(input).file_stem().map_or_else(
            || "C standard".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        )
    });
    // The PDF's bookmarks, if any, drive the sidebar outline.
    let bookmarks = doc
        .get_toc()
        .map(|toc| {
            toc.toc
                .into_iter()
                .map(|t| html::Bookmark {
                    level: t.level,
                    title: t.title,
                    page:  t.page,
                })
                .collect()
        })
        .unwrap_or_default();
    // The faces the document is mostly set in choose the page's CSS fonts.
    let mut usage: HashMap<&str, usize> = HashMap::new();
    for (name, count) in contents.iter().flat_map(|c| &c.fonts) {
        *usage.entry(name.as_str()).or_default() += count;
    }
    let fonts = html::Fonts::for_names(usage.into_iter());
    let html = html::Document::new(title, pages, bookmarks, geometry.number_offset, fonts).render();
    if let Err(err) = std::fs::write(output, html) {
        eprintln!("standards-html: cannot write {output}: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
