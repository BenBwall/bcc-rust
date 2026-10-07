//! The reader's navigation around the pages: a top bar, a sidebar with page
//! thumbnails and the document outline, and two-page spreads on wide screens.
//!
//! Everything works without scripts: the sidebar, its tabs, and the spread
//! pairing are CSS toggles and the outline is nested `<details>`. A short
//! script then draws live thumbnails, cloning each page only when its tile
//! scrolls into view, and marks the current page and outline entry.

use std::fmt::Write as _;

use crate::html::escape;

/// One heading in the outline.
pub struct Entry {
    pub id:    String,
    pub text:  String,
    /// 2 for top-level clauses, deeper clauses count up from there.
    pub level: usize,
    /// PDF page number the heading is on.
    pub page:  usize,
}

/// Page facts the thumbnail strip needs.
pub struct Thumb<'a> {
    pub number:  usize,
    pub printed: Option<&'a str>,
}

const MENU_ICON: &str =
    r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h18M3 12h18M3 18h18"/></svg>"#;
const PAGES_ICON: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="3" width="16" height="18" rx="2"/><path d="M8 15l3-3 2 2 3-4"/></svg>"#;
const SPREAD_ICON: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2" y="5" width="9" height="14" rx="1"/><rect x="13" y="5" width="9" height="14" rx="1"/></svg>"#;
const OUTLINE_ICON: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="3" width="16" height="18" rx="2"/><path d="M8 8h8M8 12h8M8 16h5"/></svg>"#;

/// Writes the top bar and the sidebar, before the pages.
pub fn sidebar(title: &str, thumbs: &[Thumb<'_>], outline: &[Entry], out: &mut String) {
    // Whether wide windows show single pages, spreads from page 1 (odd), or
    // spreads from page 2 (even). These precede the sidebar toggle so the
    // spread rules can select on both.
    let default = if even_spreads(thumbs) { "even" } else { "odd" };
    for layout in ["none", "even", "odd"] {
        let _ = write!(
            out,
            "<input type=\"radio\" name=\"spreads\" id=\"spread-{layout}\" \
             class=\"spread-{layout}\" hidden{}>",
            if layout == default { " checked" } else { "" }
        );
    }
    out.push('\n');
    // Checked hides the sidebar on wide screens and shows it on narrow ones.
    out.push_str("<input type=\"checkbox\" id=\"nav-toggle\" class=\"nav-toggle\" hidden>\n");
    out.push_str(
        "<header class=\"bar\"><label for=\"nav-toggle\" class=\"burger\" title=\"Toggle \
         sidebar\">",
    );
    out.push_str(MENU_ICON);
    out.push_str("</label><span class=\"doc-title\">");
    escape(title, out);
    // The page box needs the script, which shows it.
    let _ = write!(
        out,
        "</span><form class=\"page-nav\" hidden><input type=\"text\" inputmode=\"numeric\" \
         enterkeyhint=\"go\" autocomplete=\"off\" value=\"1\" aria-label=\"PDF page\" title=\"Go \
         to PDF page\"><span>/ {}</span></form>",
        thumbs.len()
    );
    let _ = write!(
        out,
        "<div class=\"spreads\" role=\"group\" aria-label=\"Spreads\" \
         title=\"Spreads\">{SPREAD_ICON}<span><label for=\"spread-none\" title=\"Single \
         pages\">None</label><label for=\"spread-even\" title=\"Even spreads: page 1 alone, then \
         2–3, 4–5, …\">Even</label><label for=\"spread-odd\" title=\"Odd spreads: pages 1–2, 3–4, \
         …\">Odd</label></span></div>"
    );
    out.push_str("</header>\n<nav class=\"side\" aria-label=\"Document navigation\">\n");
    out.push_str("<input type=\"radio\" name=\"side-tab\" id=\"tab-pages\" checked hidden>");
    out.push_str(
        "<input type=\"radio\" name=\"side-tab\" id=\"tab-outline\" hidden>\n<div class=\"tabs\">",
    );
    let _ = write!(
        out,
        "<label for=\"tab-pages\" title=\"Pages\">{PAGES_ICON}</label>"
    );
    let _ = write!(
        out,
        "<label for=\"tab-outline\" title=\"Outline\">{OUTLINE_ICON}</label>"
    );
    out.push_str("</div>\n<div class=\"panel thumbs\">\n");
    for thumb in thumbs {
        let _ = write!(
            out,
            "<a class=\"thumb\" href=\"#page-{0}\" data-page=\"{0}\"",
            thumb.number
        );
        if let Some(p) = thumb.printed {
            let _ = write!(out, " title=\"Printed page {p}\"");
        }
        let _ = writeln!(
            out,
            "><span class=\"sheet\"></span><span class=\"num\">{}</span></a>",
            thumb.number
        );
    }
    out.push_str("</div>\n<div class=\"panel outline\">\n");
    tree(outline, out);
    out.push_str("</div>\n</nav>\n");
}

/// Whether spreads start from page 2, so the printed page numbers put odd
/// pages on the right as in a bound book. Most pages decide, since OCR
/// misreads some numbers and front matter numbers in roman; with no arabic
/// numbers, spreads start from page 1.
fn even_spreads(thumbs: &[Thumb<'_>]) -> bool {
    let (mut even, mut odd) = (0, 0);
    for thumb in thumbs {
        if let Some(n) = thumb.printed.and_then(|p| p.parse::<usize>().ok()) {
            if (thumb.number + n) % 2 == 0 {
                even += 1;
            } else {
                odd += 1;
            }
        }
    }
    even > odd
}

/// Width of the sidebar in CSS pixels, `--side` in [`STYLE`].
const SIDE: f64 = 236.0;

/// Gap between the pages of a spread, and their least margin from the window
/// edges, in points.
const GUTTER: f64 = 8.0;

/// Writes the rules that set pages side by side in spreads wherever the window
/// fits two of the widest, `page_width` points, beside the sidebar or with it
/// closed, unless `#spread-none` is checked. `#spread-odd` puts odd pages on
/// the left and `#spread-even` even ones. The spreads fit by construction, so
/// they clip instead of overflowing while the sidebar slides.
pub fn spreads(page_width: f64, out: &mut String) {
    // Points to CSS pixels, with room for a scrollbar.
    let closed = ((2.0 * page_width + 3.0 * GUTTER) * 4.0 / 3.0 + 18.0).ceil();
    for (min, state) in [
        (closed, ".nav-toggle:checked ~ "),
        (closed + SIDE, ".nav-toggle:not(:checked) ~ "),
    ] {
        let rules = SPREAD_RULES
            .replace("STATE", state)
            .replace("GUTTER", &GUTTER.to_string());
        let _ = writeln!(out, "@media screen and (min-width: {min}px) {{{rules}}}");
    }
}

const SPREAD_RULES: &str = r#"
  STATE.bar .spreads { display: flex; }
  :is(.spread-odd, .spread-even):checked ~ STATE.doc { display: grid; grid-template-columns: repeat(2, max-content); justify-content: center; column-gap: GUTTERpt; overflow-x: clip; }
  :is(.spread-odd, .spread-even):checked ~ STATE.doc > .page { margin: 0 0 16pt; }
  .spread-odd:checked ~ STATE.doc > .page:nth-child(odd), .spread-even:checked ~ STATE.doc > .page:nth-child(even) { grid-column: 1; justify-self: end; }
  .spread-odd:checked ~ STATE.doc > .page:nth-child(even), .spread-even:checked ~ STATE.doc > .page:nth-child(odd) { grid-column: 2; justify-self: start; }
"#;

/// Nested lists of the headings; entries with children collapse.
fn tree(entries: &[Entry], out: &mut String) {
    out.push_str("<ul>\n");
    let mut i = 0;
    while i < entries.len() {
        let entry = &entries[i];
        let end = entries[i + 1..]
            .iter()
            .position(|e| e.level <= entry.level)
            .map_or(entries.len(), |n| i + 1 + n);
        let link = |out: &mut String| {
            let _ = write!(
                out,
                "<a href=\"#{}\" data-page=\"{}\">",
                entry.id, entry.page
            );
            escape(&entry.text, out);
            out.push_str("</a>");
        };
        if end > i + 1 {
            out.push_str("<li><details><summary>");
            link(out);
            out.push_str("</summary>\n");
            tree(&entries[i + 1..end], out);
            out.push_str("</details></li>\n");
        } else {
            out.push_str("<li>");
            link(out);
            out.push_str("</li>\n");
        }
        i = end;
    }
    out.push_str("</ul>\n");
}

pub const STYLE: &str = r#"
/* Reader chrome: top bar and sidebar. */
:root { --bar: 48px; --side: 236px; --chrome: #2b2b2b; --chrome-2: #383838; --chrome-ink: #e8e8e8; --accent: #7aa7ff; }
body { padding-top: calc(var(--bar) + 16pt); }
.bar { position: fixed; inset: 0 0 auto 0; height: var(--bar); z-index: 3; display: flex; align-items: center; gap: 12px; padding: 0 12px; background: var(--chrome); color: var(--chrome-ink); font: 600 15px/1 system-ui, sans-serif; box-shadow: 0 1px 3px rgb(0 0 0 / 40%); }
.burger { display: grid; place-items: center; width: 36px; height: 36px; border-radius: 6px; cursor: pointer; }
.burger:hover, .tabs label:hover { background: var(--chrome-2); }
.bar svg, .tabs svg { width: 22px; height: 22px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
.doc-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* Shown where spreads fit: single pages, or spreads from page 1 or 2. */
.page-nav { display: flex; flex: none; align-items: center; gap: 6px; margin-left: auto; font-weight: 500; }
.page-nav[hidden] { display: none; }
.page-nav input { width: 5ch; height: 28px; padding: 0 4px; border: 1px solid #555; border-radius: 4px; background: #1e1e1e; color: inherit; font: inherit; text-align: center; }
.page-nav input:focus { outline: 2px solid var(--accent); outline-offset: -1px; border-color: transparent; }
.spreads { display: none; flex: none; align-items: center; gap: 8px; font-weight: 500; white-space: nowrap; }
.spreads > span { display: flex; border: 1px solid #555; border-radius: 6px; overflow: hidden; }
.spreads label { display: grid; place-items: center; height: 28px; padding: 0 10px; color: #bbb; cursor: pointer; }
.spreads label + label { border-left: 1px solid #555; }
.spreads label:hover { background: var(--chrome-2); color: var(--chrome-ink); }
.spread-none:checked ~ .bar label[for="spread-none"], .spread-even:checked ~ .bar label[for="spread-even"], .spread-odd:checked ~ .bar label[for="spread-odd"] { background: #4a4a4a; color: var(--accent); }
.side { position: fixed; top: var(--bar); bottom: 0; left: 0; width: var(--side); z-index: 2; display: grid; grid-template-columns: 44px 1fr; background: var(--chrome); color: var(--chrome-ink); font: 13px/1.35 system-ui, sans-serif; }
.tabs { display: flex; flex-direction: column; align-items: center; gap: 6px; padding-top: 10px; border-right: 1px solid var(--chrome-2); }
.tabs label { display: grid; place-items: center; width: 34px; height: 34px; border-radius: 6px; cursor: pointer; color: #bbb; }
#tab-pages:checked ~ .tabs label[for="tab-pages"], #tab-outline:checked ~ .tabs label[for="tab-outline"] { color: var(--accent); box-shadow: inset 3px 0 var(--accent); }
.panel { display: none; overflow-y: auto; overscroll-behavior: contain; padding: 12px 8px 24px; }
#tab-pages:checked ~ .thumbs, #tab-outline:checked ~ .outline { display: block; }
.thumbs { text-align: center; }
.thumb { display: block; width: max-content; margin: 0 auto 14px; color: var(--chrome-ink); text-decoration: none; }
.thumb .sheet { display: block; position: relative; overflow: hidden; width: 122.4px; height: var(--thumb-h); background: var(--paper); border-radius: 2px; outline: 4px solid transparent; }
.thumb[aria-current="page"] .sheet, .thumb:hover .sheet { outline-color: var(--accent); }
.thumb .num { display: block; margin-top: 6px; }
.thumb .mini { position: absolute; top: 0; left: 0; margin: 0; transform: scale(var(--thumb-scale)); transform-origin: 0 0; box-shadow: none; pointer-events: none; }
.outline ul { list-style: none; margin: 0; padding: 0 0 0 12px; }
.outline > ul { padding-left: 0; }
.outline li { margin: 0; }
.outline a { display: block; padding: 4px 6px; border-radius: 4px; color: var(--chrome-ink); text-decoration: none; }
.outline a:hover { background: var(--chrome-2); text-decoration: none; }
.outline a[aria-current="true"] { background: #4a4a4a; }
.outline summary { display: flex; align-items: baseline; list-style: none; cursor: pointer; }
.outline summary::-webkit-details-marker { display: none; }
.outline summary::before { content: "▸"; flex: none; width: 12px; margin-left: -12px; color: #aaa; }
.outline details[open] > summary::before { content: "▾"; }
.outline summary a { flex: 1; }
.outline > ul { padding-left: 12px; }
/* The sidebar slides; once it is out of view it also leaves the tab order. */
.side { transition: transform 0.25s ease, visibility 0s 0s; }
.doc { margin-left: var(--side); transition: margin-left 0.25s ease; }
/* Off-screen pages skip layout, so moving the column stays cheap. */
.doc > .page { content-visibility: auto; contain-intrinsic-size: auto var(--page-h); }
.nav-toggle:checked ~ .side { transform: translateX(-100%); visibility: hidden; transition: transform 0.25s ease, visibility 0s 0.25s; }
.nav-toggle:checked ~ .doc { margin-left: 0; }
@media (prefers-reduced-motion: reduce) {
  .side, .doc, .nav-toggle:checked ~ .side { transition: none; }
}
[id] { scroll-margin-top: calc(var(--bar) + 8px); }
@media screen and (max-width: 860px) {
  /* Narrow screens start with the sidebar closed and open it over the text. */
  .side { transform: translateX(-100%); visibility: hidden; box-shadow: 4px 0 12px rgb(0 0 0 / 40%); transition: transform 0.25s ease, visibility 0s 0.25s; }
  .doc, .nav-toggle:checked ~ .doc { margin-left: 0; }
  .nav-toggle:checked ~ .side { transform: none; visibility: visible; transition: transform 0.25s ease, visibility 0s 0s; }
}
@media print {
  .bar, .side { display: none !important; }
  body { padding-top: 0; }
  .doc { margin-left: 0 !important; }
}
"#;

/// Progressive enhancement: live thumbnails and the current position.
pub const SCRIPT: &str = r#"(() => {
  const pages = [...document.querySelectorAll(".doc > .page")];
  const thumbs = [...document.querySelectorAll(".thumb")];
  const links = [...document.querySelectorAll(".outline a")];
  const side = document.querySelector(".side");
  const bar = document.querySelector(".bar");
  const doc = document.querySelector(".doc");
  // Pages set side by side in one spread.
  const beside = (a, b) => !!a && !!b && a.offsetTop === b.offsetTop;
  const shown = (el) => el.offsetParent && getComputedStyle(side).visibility === "visible";
  // Clone a page into its tile only when the tile comes into view.
  const fill = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      fill.unobserve(e.target);
      const page = document.getElementById("page-" + e.target.dataset.page);
      const mini = page.cloneNode(true);
      mini.removeAttribute("id");
      for (const n of mini.querySelectorAll("[id]")) n.removeAttribute("id");
      mini.classList.add("mini");
      mini.inert = true;
      e.target.querySelector(".sheet").append(mini);
    }
  }, { root: document.querySelector(".thumbs"), rootMargin: "600px 0px" });
  thumbs.forEach((t) => fill.observe(t));

  // Outline targets, in document order. A target's offset within its page is
  // measured once, so scrolling reads only page positions, which stay laid
  // out while off-screen pages skip their contents. In a spread, the reading
  // position runs down the left page over the first half of the spread's
  // height and down the right page over the second, so positions keep to
  // document order.
  const targets = links.map((a) => {
    const el = document.getElementById(decodeURIComponent(a.hash.slice(1)));
    return { el, page: el && el.closest(".page"), offset: null, base: 0, scale: 1 };
  });
  const top = (t) => {
    if (!t.el) return Infinity;
    const pageTop = t.page.getBoundingClientRect().top;
    if (t.offset === null) {
      t.offset = t.el.getBoundingClientRect().top - pageTop;
      const right = beside(t.page, t.page.previousElementSibling);
      t.scale = right || beside(t.page, t.page.nextElementSibling) ? 0.5 : 1;
      t.base = right ? t.page.offsetHeight / 2 : 0;
    }
    return pageTop + t.base + t.offset * t.scale;
  };
  // Last index in `items` whose position is at or above `line`.
  const lastAbove = (items, position, line) => {
    let lo = -1, hi = items.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (position(items[mid]) <= line) lo = mid; else hi = mid - 1;
    }
    return lo;
  };

  // A clicked outline entry stays current until the reader scrolls on, even
  // where the document cannot scroll its heading up to the top.
  let pinned = null, pinnedY = null;
  document.querySelector(".outline").addEventListener("click", (e) => {
    const a = e.target.closest("a");
    if (a) { pinned = a; pinnedY = null; }
  });

  // The page a relayout keeps in place, and where its top was. Only the
  // reader's scrolling moves it on, not a relayout's own scroll, so switching
  // the layout back and forth returns to the same page.
  let held = 0, heldTop = 0, heldY = null;
  // A page typed into the page box stays current until the reader scrolls on,
  // even beside another in a spread or where it cannot scroll to the top.
  const form = document.querySelector(".page-nav");
  const box = form.querySelector("input");
  let typed = 0, typedY = null;
  let page = 0, entry = null, queued = false;
  const update = () => {
    queued = false;
    if (typed) {
      if (typedY === null) typedY = scrollY;
      else if (Math.abs(scrollY - typedY) > 4) typed = 0;
    }
    // The current page is the last one whose top has passed a third of the
    // way down the window.
    let n = typed || lastAbove(pages, (p) => p.getBoundingClientRect().top, innerHeight / 3) + 1 || 1;
    // A spread is current from its left page.
    if (!typed && beside(pages[n - 1], pages[n - 2])) n--;
    if (heldY === null || Math.abs(scrollY - heldY) > 1) {
      held = n;
      heldTop = pages[n - 1].getBoundingClientRect().top;
      heldY = null;
    }
    if (n !== page) {
      page = n;
      for (const t of thumbs) t.removeAttribute("aria-current");
      thumbs[n - 1].setAttribute("aria-current", "page");
      if (document.activeElement !== box) box.value = n;
      if (shown(thumbs[n - 1])) thumbs[n - 1].scrollIntoView({ block: "nearest" });
    }
    // The current entry is the last heading that has reached the top bar.
    if (pinned) {
      if (pinnedY === null) pinnedY = scrollY;
      else if (Math.abs(scrollY - pinnedY) > 4) pinned = null;
    }
    const here = pinned || links[lastAbove(targets, top, bar.offsetHeight + 16)] || null;
    if (here === entry) return;
    if (entry) entry.removeAttribute("aria-current");
    entry = here;
    if (!here) return;
    here.setAttribute("aria-current", "true");
    // Open the outline down to the current entry and keep it in view.
    for (let d = here.closest("details"); d; d = d.parentElement.closest("details")) d.open = true;
    if (shown(here)) here.scrollIntoView({ block: "nearest" });
  };
  const schedule = () => { if (!queued) { queued = true; requestAnimationFrame(update); } };
  addEventListener("scroll", schedule, { passive: true });
  // Spreads come and go with the window width, the sidebar, and the spread
  // control, which also pairs the pages differently. Each moves every page,
  // so the held page is put back where it was.
  let spread = getComputedStyle(doc).display === "grid";
  const relayout = (moved) => {
    targets.forEach((t) => { t.offset = null; });
    const now = getComputedStyle(doc).display === "grid";
    if (!moved && now === spread) return;
    spread = now;
    if (held) {
      scrollBy(0, pages[held - 1].getBoundingClientRect().top - heldTop);
      heldY = scrollY;
    }
    schedule();
  };
  addEventListener("resize", () => relayout(false));
  document.getElementById("nav-toggle").addEventListener("change", () => relayout(false));
  for (const input of document.querySelectorAll("[name=spreads]")) {
    input.addEventListener("change", () => relayout(true));
  }
  // A panel that was hidden could not scroll to the current entry. Catch up
  // once it shows, after the slide, so the slide's frames stay free.
  for (const input of document.querySelectorAll("[name=side-tab], #nav-toggle")) {
    input.addEventListener("change", () => setTimeout(() => { page = 0; entry = null; schedule(); }, 300));
  }
  form.hidden = false;
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const n = Math.min(Math.max(parseInt(box.value, 10), 1), pages.length);
    box.blur();
    box.value = n || page;
    if (!n) return;
    history.pushState(null, "", `#page-${n}`);
    pages[n - 1].scrollIntoView();
    typed = n;
    typedY = null;
    schedule();
  });
  box.addEventListener("focus", () => box.select());
  box.addEventListener("blur", () => { box.value = page; });
  box.addEventListener("keydown", (e) => { if (e.key === "Escape") box.blur(); });
  update();
})();
"#;

#[cfg(test)]
mod tests {
    use super::{
        SIDE,
        STYLE,
        Thumb,
        even_spreads,
    };

    #[test]
    fn side_width_matches_style() {
        assert!(STYLE.contains(&format!("--side: {SIDE}px;")));
    }

    #[test]
    fn spread_parity_follows_printed_numbers() {
        let thumbs = |printed: &[Option<&'static str>]| -> Vec<Thumb<'static>> {
            printed
                .iter()
                .enumerate()
                .map(|(i, &printed)| Thumb {
                    number: i + 1,
                    printed,
                })
                .collect()
        };
        // Printed page 1 on PDF page 3 sits on the right of 2–3.
        assert!(even_spreads(&thumbs(&[
            Some("i"),
            Some("ii"),
            Some("1"),
            Some("2"),
            Some("3")
        ])));
        // Printed page 1 on PDF page 2 sits on the right of 1–2, and a misread
        // number is outvoted.
        assert!(!even_spreads(&thumbs(&[
            Some("i"),
            Some("1"),
            Some("2"),
            Some("9"),
            Some("4")
        ])));
        assert!(!even_spreads(&thumbs(&[None, Some("ii"), None])));
    }
}
