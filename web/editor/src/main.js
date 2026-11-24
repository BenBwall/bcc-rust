// Monaco glue for the bcc site. The Leptos page (web/src/editor.rs) imports
// this module at run time through wasm-bindgen and drives it with a small,
// plain-data API: create an editor in a host element, set its text,
// language and decorations, and listen for edits, line hovers and cursor
// moves. Everything else (state, semantics, what to highlight) lives in Rust.
//
// Only the editor core and the features the site uses are bundled; language
// support is the Monarch grammars in ./languages.js.

import * as monaco from 'monaco-editor/editor/editor.api.js';
import 'monaco-editor/features/find/register.js';
import 'monaco-editor/features/folding/register.js';
import 'monaco-editor/features/bracketMatching/register.js';
import 'monaco-editor/features/wordHighlighter/register.js';
import 'monaco-editor/features/clipboard/register.js';
import 'monaco-editor/features/hover/register.js';
import 'monaco-editor/features/comment/register.js';
import 'monaco-editor/features/multicursor/register.js';
import 'monaco-editor/features/wordOperations/register.js';
import { ResizableContentWidget } from 'monaco-editor/editor/contrib/hover/browser/resizableContentWidget.js';
import { registerLanguages } from './languages.js';
import { defineThemes } from './themes.js';

// Where a hover has room: between its line and the edge of the editor's
// frame (.code-editor clips what overflows it) or of the window, whichever
// is nearer. Monaco 0.57 measures the room below as the window's height
// less the line's place on the page, scroll included, so on a page
// scrolled further than its hover is tall every hover opened above its
// line even with `hover: { above: false }`, and line 1's was cut off by the
// frame's top edge, under the reading over the output. Measured here, a
// hover opens below when it fits there, and else is sized to the side with
// more room.
const HOVER_MARGIN = 2;
function hoverRoom(widget, position) {
  const node = widget._editor.getDomNode();
  const line = widget._editor.getScrolledVisiblePosition(position);
  if (!node || !line) return undefined;
  const frame = node.getBoundingClientRect();
  const view = node.ownerDocument.defaultView;
  const top = frame.top + line.top;
  return {
    above: top - Math.max(frame.top, 0) - HOVER_MARGIN,
    below: Math.min(frame.bottom, view.innerHeight) - (top + line.height) - HOVER_MARGIN,
  };
}
ResizableContentWidget.prototype._availableVerticalSpaceAbove = function (position) {
  return hoverRoom(this, position)?.above;
};
ResizableContentWidget.prototype._availableVerticalSpaceBelow = function (position) {
  return hoverRoom(this, position)?.below;
};

// The editor worker runs Monaco's text-model computations off the page.
self.MonacoEnvironment = {
  getWorker() {
    return new Worker(new URL('./editor.worker.js', import.meta.url), { type: 'module' });
  },
};

const stylesheet = document.createElement('link');
stylesheet.rel = 'stylesheet';
stylesheet.href = new URL('./main.css', import.meta.url).href;
document.head.append(stylesheet);

registerLanguages(monaco);
defineThemes(monaco);
// Monaco measures glyphs when an editor is created; measure again once the
// site's web font has arrived.
document.fonts?.ready.then(() => monaco.editor.remeasureFonts());

// How the page was last used: by keyboard or by pointer. The Tab hint shows
// only when the keyboard brought the focus into an editor.
let usingKeyboard = false;
addEventListener('keydown', () => { usingKeyboard = true; }, true);
addEventListener('pointerdown', () => { usingKeyboard = false; }, true);

// Whether the reader asked for reduced motion: the editors then jump to a
// line rather than glide there.
const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
// Where a wrapped line may break: after these characters, as Monaco's
// default list for Latin text has it, but not after a full stop.
const WRAP_BREAK_AFTER = ' \t})]?|/&,;';
const editors = new Set();
reducedMotion.addEventListener?.('change', () => {
  for (const editor of editors) editor.applyMotion();
});

function motionOptions() {
  return {
    smoothScrolling: !reducedMotion.matches,
    cursorSmoothCaretAnimation: reducedMotion.matches ? 'off' : 'on',
  };
}

// What the reader last did, so the page never scrolls against them: when
// they last scrolled themselves (a wheel, a touch, a scroll key outside the
// editors, the scroll bar), and whether the pointer or the keyboard moved
// last, and where the pointer is.
let userScrolled = -Infinity;
let lastKey = -Infinity;
let pointer = null;
let pointerHeld = false;
const SCROLL_KEYS = new Set(['PageUp', 'PageDown', 'Home', 'End', ' ', 'ArrowUp', 'ArrowDown']);
const noteUserScroll = () => { userScrolled = performance.now(); };
addEventListener('wheel', noteUserScroll, { capture: true, passive: true });
addEventListener('touchmove', noteUserScroll, { capture: true, passive: true });
addEventListener('keydown', (event) => {
  lastKey = performance.now();
  // The arrow keys in an editor move its cursor, which is how a reader
  // selects a line, not a scroll.
  if (SCROLL_KEYS.has(event.key) && !event.target?.closest?.('.monaco-editor')) noteUserScroll();
}, true);
addEventListener('pointermove', (event) => {
  // A finger leaves nothing under the pointer once it lifts.
  pointer = { x: event.clientX, y: event.clientY, time: performance.now(), hovers: event.pointerType === 'mouse' };
}, { capture: true, passive: true });
addEventListener('pointerdown', (event) => {
  pointerHeld = true;
  pointer = { x: event.clientX, y: event.clientY, time: performance.now(), hovers: event.pointerType === 'mouse' };
}, true);
addEventListener('pointerup', () => { pointerHeld = false; }, true);
addEventListener('pointercancel', () => { pointerHeld = false; }, true);
// Dragging the page's scroll bar, or the page under a finger.
addEventListener('scroll', () => { if (pointerHeld) noteUserScroll(); }, { passive: true });

/** How long after the reader's own scrolling the page leaves the scrolling
 * to them. */
const USER_SCROLL_GRACE_MS = 1500;
/** How long the pointer or the cursor must rest on a line before the page
 * scrolls to what it links to, so sweeping across lines moves nothing. */
const LINK_REST_MS = 400;

/**
 * How far the page may scroll by `by` without moving what the reader points
 * at out of reach: all of it when that is in a sticky pane stuck to the
 * window (beside the output on a wide screen), which scrolling leaves where
 * it is; else, for a line the mouse hovers, none, since the page would slide
 * another line under the pointer; and for the cursor's line, chosen with the
 * keyboard or a tap, as far as keeps it in view (the editor's text area
 * sits on that line).
 */
function allowedScroll(by) {
  const byPointer = pointer && pointer.time >= lastKey && pointer.hovers;
  const node = byPointer ? document.elementFromPoint(pointer.x, pointer.y) : document.activeElement;
  if (!node || node === document.body || node === document.documentElement) {
    return 0;
  }
  if (isStuck(node)) {
    return by;
  }
  if (byPointer || !node.closest('.monaco-editor')) {
    return 0;
  }
  const { clear, bottom } = visibleBand();
  const line = node.getBoundingClientRect();
  return by > 0 ? Math.max(0, Math.min(by, line.top - clear)) : Math.min(0, Math.max(by, line.bottom - bottom));
}

/** The key that switches what Tab does in an editor: Alt+M (Option+M on a
 * Mac). Monaco's own Ctrl+M (Ctrl+Shift+M on a Mac) cannot undo the
 * `tabFocusMode` option, and Firefox takes Ctrl+M to mute the tab. */
const TAB_KEY_TOGGLE = monaco.KeyMod.Alt | monaco.KeyCode.KeyM;
const TAB_KEY_TOGGLE_LABEL = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌥M' : 'Alt+M';

/** The part of the window that nothing sticky covers. */
function visibleBand() {
  const clear = parseFloat(getComputedStyle(document.documentElement).scrollPaddingTop) || 0;
  return { clear, bottom: innerHeight - 8 };
}

/** Whether `node` sits in a sticky box that is stuck to the window's top. */
function isStuck(node) {
  for (let box = node.parentElement; box; box = box.parentElement) {
    const style = getComputedStyle(box);
    if (style.position === 'sticky') {
      return Math.abs(box.getBoundingClientRect().top - (parseFloat(style.top) || 0)) < 1;
    }
  }
  return false;
}

// The editors that follow execution on the page, each showing its current
// line. A step can move one line (the WebAssembly instruction) or two (and
// the C line); the page scrolls once per frame, as little as shows every
// current line clear of the toolbar when they fit together, and otherwise
// the line just revealed.
const following = new Set();
let latest = null;
let frame = 0;

function followOnPage(editor) {
  following.add(editor);
  latest = editor;
  frame ||= requestAnimationFrame(() => {
    frame = 0;
    const { clear, bottom: limit } = visibleBand();
    const spans = [...following].map((each) => [each, each.followedSpan(clear, limit)]).filter(([, span]) => span);
    for (const each of following) {
      each.revealed = null;
    }
    const own = spans.find(([each]) => each === latest)?.[1];
    if (!own) {
      return;
    }
    const all = { top: Math.min(...spans.map(([, span]) => span.top)), bottom: Math.max(...spans.map(([, span]) => span.bottom)) };
    const span = all.bottom - all.top <= limit - clear ? all : own;
    if (span.top < clear) {
      scrollBy({ top: span.top - clear });
    } else if (span.bottom > limit) {
      scrollBy({ top: span.bottom - limit });
    }
  });
}

/**
 * Scroll the page as little as shows the cursor's line of `editor`, after
 * the arrow keys moved it: Monaco keeps the line inside the editor, not
 * the editor's line on the page. An editor in a stuck sticky pane is
 * already where scrolling cannot move it.
 */
function showCursorOnPage(editor) {
  requestAnimationFrame(() => {
    const node = editor.getContainerDomNode();
    const position = editor.getPosition();
    if (!position || !node.isConnected || isStuck(node)) {
      return;
    }
    const { clear, bottom } = visibleBand();
    const height = lineSpan(editor, position.lineNumber);
    const top = node.getBoundingClientRect().top + editor.getTopForLineNumber(position.lineNumber) - editor.getScrollTop();
    if (top < clear) {
      scrollBy({ top: top - clear });
    } else if (top + height > bottom) {
      scrollBy({ top: top + height - bottom });
    }
  });
}

/**
 * The height of `line` of `editor` in pixels: a line that wraps takes a
 * row of the line height for each piece.
 */
function lineSpan(editor, line) {
  return editor.getBottomForLineNumber(line) - editor.getTopForLineNumber(line);
}

/** Switch every editor between the site's `light` and `dark` themes. */
export function setTheme(theme) {
  monaco.editor.setTheme(theme === 'light' ? 'bcc-light' : 'bcc-dark');
}

/**
 * Create an editor in `host`. `options`: `value`, `language` (`c`, `bcc-ir`,
 * `llvm`, `asm`, `objdump`, `wat` or `plaintext`), `readOnly`, `ariaLabel`, `theme`.
 */
export function createEditor(host, options) {
  return new Editor(host, options);
}

/** Whether a view in `language` wraps long rows: all but the machine code. */
function wrapsRows(language) {
  return language === 'objdump' ? 'off' : 'on';
}

class Editor {
  constructor(host, options) {
    this.decorations = new Map();
    // Listener registrations, released by dispose().
    this.listeners = [];
    this.editor = monaco.editor.create(host, {
      value: options.value ?? '',
      language: options.language ?? 'plaintext',
      readOnly: !!options.readOnly,
      lineNumbers: options.language === 'objdump' ? 'off' : 'on',
      domReadOnly: !!options.readOnly,
      ariaLabel: options.ariaLabel ?? 'Code',
      theme: options.theme === 'light' ? 'bcc-light' : 'bcc-dark',
      automaticLayout: true,
      fontFamily: "'Space Mono', ui-monospace, 'SFMono-Regular', Menlo, Consolas, monospace",
      fontSize: 13,
      lineHeight: 22,
      fontLigatures: false,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      // A read-only view marks the cursor's line only while it has the
      // focus, for the reader moving through it with the arrow keys.
      // The gutter too, where the focused editor's bar marks the line.
      renderLineHighlight: 'all',
      renderLineHighlightOnlyWhenFocus: !!options.readOnly,
      // Clicking a read-only view's line number selects the line; keep the
      // spaces of the selection blank.
      renderWhitespace: options.readOnly ? 'none' : 'selection',
      glyphMargin: !options.readOnly,
      // The WebAssembly listing runs to hundreds of lines.
      lineNumbersMinChars: options.language === 'wat' ? 4 : 3,
      lineDecorationsWidth: 10,
      folding: !!options.readOnly,
      ...motionOptions(),
      padding: { top: 14, bottom: 14 },
      scrollbar: { verticalScrollbarSize: 10, horizontalScrollbarSize: 10, useShadows: false },
      // A hover opens below its line, so the first line's (a folded line's
      // explanation) is not cut off by the editor's top edge (with
      // hoverRoom, which measures where it fits).
      hover: { above: false },
      overviewRulerBorder: false,
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      guides: { indentation: true },
      bracketPairColorization: { enabled: false },
      // Monaco guesses a model's tab size from its text by default, and
      // a wrapped row hangs by tab stops, so the listings (LLVM IR indents
      // by 2) hung 4 columns in some and 8 in others. A fixed size keeps
      // every listing's hang the same: a wrapped row goes on two tab stops
      // past its line's indent, at column 8 for an unindented line and for
      // LLVM IR's 2-space instructions alike (6 columns past those).
      // (Monaco keeps model options in one configuration for all editors,
      // so the source takes the same fixed size; its lessons indent by 4.)
      tabSize: 4,
      detectIndentation: false,
      // Every view wraps rather than hiding the ends of long lines: the
      // source's comments in the narrow columns of small screens, and the
      // listings' calls, string constants and LLVM's `; preds` comments
      // beside the stage's notes. A wrapped piece hangs a level further in
      // than its line, a listing's two levels, past the instructions'
      // operands, and only a line's first row has a number, so it reads
      // as one line; the line links and their marks cover all its rows.
      // The machine code is the exception: a row is an offset, its bytes
      // and their instruction side by side, and a row broken in two put an
      // instruction's operands under its bytes, so the listing scrolls
      // sideways in its frame instead (wrapsRows).
      wordWrap: wrapsRows(options.language),
      wrappingIndent: options.readOnly ? 'deepIndent' : 'indent',
      // Rows break at spaces and punctuation, but not at a full stop, so
      // a number (`0.2`, `3.000000e-01`) or a name (`@.str.5`) stays
      // whole; a piece too long for a row still breaks.
      wordWrapBreakAfterCharacters: WRAP_BREAK_AFTER,
      stickyScroll: { enabled: false },
      accessibilitySupport: 'auto',
      quickSuggestions: false,
      suggestOnTriggerCharacters: false,
      contextmenu: false,
      // Tab moves the keyboard focus on, as everywhere else on the page,
      // instead of trapping it in the editor (and editing the lesson);
      // Alt+M switches Tab to indenting while editing (see TabHint).
      tabFocusMode: true,
    });
    this.tabHint = new TabHint(this.editor);
    editors.add(this);
    // The time the reader last turned the wheel over this editor.
    this.wheeled = -Infinity;
    this.onWheel = () => { this.wheeled = performance.now(); };
    host.addEventListener('wheel', this.onWheel, { capture: true, passive: true });
    this.listeners.push(this.editor.onDidChangeCursorPosition((event) => {
      if (event.source === 'keyboard') showCursorOnPage(this.editor);
    }));
    // A read-only view the reader scrolled keeps its cursor where it was,
    // often far off screen; Tab into it starts at the first line in view
    // instead, so its links and announcement are about what is shown.
    if (options.readOnly) {
      this.listeners.push(this.editor.onDidFocusEditorText(() => {
        const line = this.editor.getPosition()?.lineNumber ?? 1;
        const visible = this.editor.getVisibleRanges();
        if (visible.length && !visible.some((range) => line >= range.startLineNumber && line <= range.endLineNumber)) {
          // Past the padding line Monaco half-shows at the top.
          const top = visible[0].startLineNumber;
          const first = this.editor.getTopForLineNumber(top) < this.editor.getScrollTop() ? top + 1 : top;
          this.editor.setPosition({ lineNumber: Math.min(first, visible[0].endLineNumber), column: 1 });
        }
      }));
    }
  }

  getValue() {
    return this.editor.getValue();
  }

  /** Follow the reader's reduced-motion preference, which can change. */
  applyMotion() {
    this.editor.updateOptions(motionOptions());
  }

  /**
   * Replace the text, keeping the undo stack and the view when unchanged.
   * With `top`, the new text opens with that line first in view (1: at its
   * start), so a view that changes under the reader keeps their place in
   * the code rather than their scroll offset in pixels.
   */
  setValue(text, top = 0) {
    if (this.editor.getValue() === text) {
      return;
    }
    this.placing = true;
    try {
      this.editor.setValue(text);
      this.scrollToTop(top);
    } finally {
      this.placing = false;
    }
    this.reportTop?.();
  }

  /** Scroll, at once, so `line` is the first line in view (0: stay). */
  placeTop(line) {
    if (line <= 0) {
      return;
    }
    this.placing = true;
    try {
      this.scrollToTop(line);
    } finally {
      this.placing = false;
    }
    this.reportTop?.();
  }

  scrollToTop(line) {
    if (line <= 0) {
      return;
    }
    const top = line <= 1 ? 0 : this.editor.getTopForLineNumber(line);
    this.editor.setScrollTop(top, monaco.editor.ScrollType.Immediate);
  }

  /** The first line wholly in view, past the one Monaco half-shows. */
  topLine() {
    const visible = this.editor.getVisibleRanges();
    if (!visible.length) {
      return 1;
    }
    const top = visible[0].startLineNumber;
    const hidden = this.editor.getTopForLineNumber(top) < this.editor.getScrollTop();
    return hidden ? Math.min(top + 1, visible[0].endLineNumber) : top;
  }

  /**
   * Call `callback(line)` with the first line wholly in view after each
   * scroll, once per frame; a new editor is at line 1 until it scrolls. The
   * page keeps the reader's place in the code with it (see `setValue`).
   */
  onTopLine(callback) {
    let frame = 0;
    let current = 0;
    const report = () => {
      frame = 0;
      const line = this.topLine();
      if (line !== current) {
        current = line;
        callback(line);
      }
    };
    // After new text, the same line number is another line: report it again.
    this.reportTop = () => {
      current = 0;
      frame ||= requestAnimationFrame(report);
    };
    this.listeners.push(this.editor.onDidScrollChange((event) => {
      if (event.scrollTopChanged && !this.placing) frame ||= requestAnimationFrame(report);
    }));
    this.listeners.push({ dispose: () => cancelAnimationFrame(frame) });
  }

  /**
   * Bring `first` to `last`, the run of lines linked to the line the reader
   * points at in the other editor, into view (0 cancels): inside this
   * editor at once, unless the reader is turning the wheel over it; and,
   * once the pointer or the cursor has rested on its line, on the page, as
   * little as shows them clear of the sticky header and above the window's
   * bottom, provided part of this editor is on screen and the reader has not
   * just scrolled themselves, and only as far as keeps the line they point
   * at where they can see it (see `allowedScroll`). Both glide, unless the
   * reader asked for reduced motion.
   */
  showLinked(first, last) {
    clearTimeout(this.linkTimer);
    this.linkTimer = 0;
    if (first <= 0) {
      return;
    }
    last = Math.max(first, last);
    // The same run again, as when the pointer leaves this editor and the
    // cursor's link in the other is back, after the reader turned the wheel
    // here since it was shown: they scrolled away from it on purpose.
    const again = this.linked?.[0] === first && this.linked?.[1] === last;
    if (again && this.wheeled > this.linkedAt) {
      return;
    }
    this.linked = [first, last];
    this.linkedAt = performance.now();
    const editor = this.editor;
    const smooth = monaco.editor.ScrollType.Smooth;
    if (performance.now() - this.wheeled > USER_SCROLL_GRACE_MS) {
      const lineHeight = editor.getOption(monaco.editor.EditorOption.lineHeight);
      const span = editor.getBottomForLineNumber(last) - editor.getTopForLineNumber(first);
      // Every row of the run, the last line's wrapped rows included: a
      // range of whole lines, since revealing lines stops at the last
      // line's first row. A run too tall for the editor shows its first
      // line whole.
      const model = editor.getModel();
      const fits = span <= editor.getLayoutInfo().height - 2 * lineHeight;
      const end = fits ? last : first;
      editor.revealRangeInCenterIfOutsideViewport(new monaco.Range(first, 1, end, model.getLineMaxColumn(end)), smooth);
    }
    this.linkTimer = setTimeout(() => {
      this.linkTimer = 0;
      this.showLinkedOnPage(first, last);
    }, LINK_REST_MS);
  }

  showLinkedOnPage(first, last) {
    const node = this.editor.getContainerDomNode();
    if (!node.isConnected || isStuck(node) || performance.now() - userScrolled < USER_SCROLL_GRACE_MS) {
      return;
    }
    const { clear, bottom } = visibleBand();
    const host = node.getBoundingClientRect();
    if (host.bottom <= clear || host.top >= bottom) {
      return;
    }
    // The run as it sits in the window, cut to what the editor shows.
    const editor = this.editor;
    const scrollTop = editor.getScrollTop();
    const top = Math.max(host.top, host.top + editor.getTopForLineNumber(first) - scrollTop);
    const end = Math.min(host.bottom, host.top + editor.getBottomForLineNumber(last) - scrollTop);
    if (end <= top) {
      return;
    }
    let by = 0;
    if (top < clear) {
      by = top - clear;
    } else if (end > bottom) {
      // As much as shows the run's end, never pushing its start under the
      // header.
      by = Math.min(end - bottom, top - clear);
    }
    by = allowedScroll(by);
    if (Math.abs(by) >= 1) {
      scrollBy({ top: by, behavior: reducedMotion.matches ? 'instant' : 'smooth' });
    }
  }

  setLanguage(language) {
    monaco.editor.setModelLanguage(this.editor.getModel(), language);
    // The machine-code listing numbers its lines with code offsets.
    this.editor.updateOptions({
      lineNumbers: language === 'objdump' ? 'off' : 'on',
      wordWrap: wrapsRows(language),
    });
  }

  /**
   * Label each line in the gutter with `labels[line - 1]` instead of its
   * number, as the logical-lines view does with the physical lines a line
   * came from; an empty list numbers the lines again.
   */
  setLineNumbers(labels) {
    if (!labels || labels.length === 0) {
      this.editor.updateOptions({
        lineNumbers: this.editor.getModel()?.getLanguageId() === 'objdump' ? 'off' : 'on',
        lineNumbersMinChars: 3,
      });
      return;
    }
    const width = Math.max(3, ...labels.map((label) => label.length));
    this.editor.updateOptions({
      lineNumbers: (line) => labels[line - 1] ?? '',
      lineNumbersMinChars: width,
    });
  }

  setReadOnly(readOnly) {
    this.editor.updateOptions({ readOnly, domReadOnly: readOnly });
  }

  /** Call `callback(text)` after every edit. */
  onChange(callback) {
    this.listeners.push(this.editor.onDidChangeModelContent(() => callback(this.editor.getValue())));
  }

  /** Call `callback(line)` (0 when leaving) as the pointer moves over lines. */
  onHoverLine(callback) {
    let current = 0;
    this.listeners.push(this.editor.onMouseMove((event) => {
      const line = event.target.position?.lineNumber ?? 0;
      if (line !== current) {
        current = line;
        callback(line);
      }
    }));
    this.listeners.push(this.editor.onMouseLeave(() => {
      if (current !== 0) {
        current = 0;
        callback(0);
      }
    }));
  }

  /** Call `callback(line)` when the gutter (line number or glyph) is clicked. */
  onGutterClick(callback) {
    this.listeners.push(this.editor.onMouseDown((event) => {
      const type = event.target.type;
      if (
        type === monaco.editor.MouseTargetType.GUTTER_LINE_NUMBERS ||
        type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN
      ) {
        callback(event.target.position.lineNumber);
      }
    }));
  }

  /**
   * Call `callback(line)` with the cursor's line while the editor has the
   * keyboard focus: as the focus arrives and whenever the cursor moves to
   * another line. Call `callback(0)` when the focus leaves.
   */
  onCursorLine(callback) {
    let current = 0;
    const report = (line) => {
      if (line !== current) {
        current = line;
        callback(line);
      }
    };
    this.listeners.push(this.editor.onDidChangeCursorPosition((event) => {
      if (this.editor.hasTextFocus()) report(event.position.lineNumber);
    }));
    this.listeners.push(this.editor.onDidFocusEditorText(() => {
      report(this.editor.getPosition()?.lineNumber ?? 0);
    }));
    this.listeners.push(this.editor.onDidBlurEditorText(() => report(0)));
    // The page may have moved the focus in before listening (from the
    // highlighted fallback Monaco replaced).
    if (this.editor.hasTextFocus()) report(this.editor.getPosition()?.lineNumber ?? 0);
  }

  /**
   * Replace the decoration layer `layer` with `items`: each has 1-based
   * `startLine`, `startColumn`, `endLine`, `endColumn` (columns may be 0 for
   * whole-line marks), and any of `inlineClass`, `lineClass`, `marginClass`,
   * `numberClass` (the line number), `glyphClass`, `hover` (Markdown), and
   * `before` with `beforeClass`: text drawn before the range that is not
   * part of the code.
   */
  setDecorations(layer, items) {
    const decorations = items.map((item) => ({
      range: new monaco.Range(
        item.startLine,
        item.startColumn || 1,
        item.endLine || item.startLine,
        item.endColumn || 1,
      ),
      options: {
        isWholeLine: !item.startColumn,
        inlineClassName: item.inlineClass || undefined,
        inlineClassNameAffectsLetterSpacing: false,
        className: item.className || undefined,
        linesDecorationsClassName: item.marginClass || undefined,
        lineNumberClassName: item.numberClass || undefined,
        glyphMarginClassName: item.glyphClass || undefined,
        hoverMessage: item.hover ? { value: item.hover } : undefined,
        before: item.before
          ? { content: item.before, inlineClassName: item.beforeClass || undefined }
          : undefined,
        // A marker sits on an empty range, which Monaco otherwise skips.
        showIfCollapsed: !!item.before,
        stickiness: monaco.editor.TrackedRangeStickiness.NeverGrowsWhenTypingAtEdges,
        zIndex: item.zIndex || 0,
      },
    }));
    let collection = this.decorations.get(layer);
    if (!collection) {
      collection = this.editor.createDecorationsCollection();
      this.decorations.set(layer, collection);
    }
    collection.set(decorations);
  }

  /**
   * Replace bcc's diagnostics on the text with `items`, drawn as squiggles
   * with their message on hover: each has 1-based `startLine`, `startColumn`,
   * `endLine` and `endColumn` (UTF-16, the end just past the range),
   * `severity` (`error`, `warning` or `note`) and `message`.
   */
  setMarkers(items) {
    const severity = {
      error: monaco.MarkerSeverity.Error,
      warning: monaco.MarkerSeverity.Warning,
    };
    monaco.editor.setModelMarkers(this.editor.getModel(), 'bcc', items.map((item) => ({
      startLineNumber: item.startLine,
      startColumn: item.startColumn,
      endLineNumber: item.endLine,
      endColumn: item.endColumn,
      severity: severity[item.severity] ?? monaco.MarkerSeverity.Info,
      message: item.message,
      source: 'bcc',
    })));
  }

  /**
   * Scroll so `line` is visible, centring it only when it is off screen.
   * With `onPage`, also scroll the page as little as needed to bring the
   * line clear of the sticky header and toolbar (the root's scroll
   * padding) and above the window's bottom (see `followOnPage`), provided
   * part of the editor is already on screen, so that a reader who scrolled
   * away to the memory diagram stays there, and the editor is not in a
   * sticky pane that is already stuck, which scrolling cannot move.
   */
  revealLine(line, onPage) {
    if (line <= 0) {
      return;
    }
    const editor = this.editor;
    const height = editor.getLayoutInfo().height;
    const scrollTop = editor.getScrollTop();
    const top = editor.getTopForLineNumber(line);
    // A wrapped line's rows, all of which the reveal brings into view.
    const span = lineSpan(editor, line);
    const inside = top >= scrollTop && top + span <= scrollTop + height;
    const range = new monaco.Range(line, 1, line, editor.getModel().getLineMaxColumn(line));
    editor.revealRangeInCenterIfOutsideViewport(range, monaco.editor.ScrollType.Smooth);
    if (!onPage) {
      return;
    }
    // Where the line ends up in the editor: where it is, or where the
    // (smooth) reveal is taking it, centred as far as the text allows.
    const centred = Math.min(Math.max(0, top - (height - span) / 2), Math.max(0, editor.getContentHeight() - height));
    this.followed = line;
    this.revealed = top - (inside ? scrollTop : centred);
    followOnPage(this);
  }

  /**
   * Where this editor's current line sits in the window, or null when the
   * page should not follow it: the editor is off screen (the reader
   * scrolled away), or in a sticky pane that is stuck, which scrolling
   * cannot move.
   */
  followedSpan(clear, limit) {
    const node = this.editor.getContainerDomNode();
    if (!this.followed || !node.isConnected || isStuck(node)) {
      return null;
    }
    const host = node.getBoundingClientRect();
    if (host.bottom <= clear || host.top >= limit) {
      return null;
    }
    const offset = this.revealed ?? this.editor.getTopForLineNumber(this.followed) - this.editor.getScrollTop();
    const top = host.top + offset;
    return { top, bottom: top + lineSpan(this.editor, this.followed) };
  }

  focus() {
    this.editor.focus();
  }

  layout() {
    this.editor.layout();
  }

  /**
   * Release the editor: its listeners (so no callback runs afterwards), its
   * decorations, its text model, and the editor itself with its layout
   * observer. The page calls this when the view holding the editor unmounts.
   */
  dispose() {
    following.delete(this);
    editors.delete(this);
    clearTimeout(this.linkTimer);
    this.editor.getContainerDomNode().removeEventListener('wheel', this.onWheel, { capture: true });
    for (const listener of this.listeners.splice(0)) {
      listener.dispose();
    }
    this.tabHint.dispose();
    monaco.editor.setModelMarkers(this.editor.getModel(), 'bcc', []);
    for (const collection of this.decorations.values()) {
      collection.clear();
    }
    this.decorations.clear();
    const model = this.editor.getModel();
    this.editor.dispose();
    if (model && !model.isDisposed()) {
      model.dispose();
    }
  }
}

/**
 * What Tab does in an editable editor, and the key that switches it. Tab
 * starts by moving the focus on, as everywhere else on the page; Alt+M
 * makes it indent, and back. A small note in the editor's corner says
 * which, while the keyboard has the focus there, and screen readers hear
 * it when it changes.
 */
class TabHint {
  constructor(editor) {
    this.editor = editor;
    this.node = document.createElement('div');
    this.node.className = 'tab-hint';
    this.node.setAttribute('role', 'status');
    this.node.hidden = true;
    this.widget = {
      getId: () => 'bcc.tabHint',
      getDomNode: () => this.node,
      getPosition: () => ({ preference: monaco.editor.OverlayWidgetPositionPreference.BOTTOM_RIGHT_CORNER }),
    };
    editor.addOverlayWidget(this.widget);
    this.action = editor.addAction({
      id: 'bcc.toggleTabFocus',
      label: 'Toggle Tab Key Moves Focus',
      keybindings: [TAB_KEY_TOGGLE],
      precondition: '!editorReadonly',
      run: () => {
        editor.updateOptions({ tabFocusMode: !this.movesFocus() });
        this.show();
      },
    });
    this.listeners = [
      editor.onDidFocusEditorText(() => {
        if (usingKeyboard) this.show();
      }),
      editor.onDidBlurEditorText(() => this.hide()),
      editor.onMouseDown(() => this.hide()),
    ];
  }

  movesFocus() {
    return this.editor.getOption(monaco.editor.EditorOption.tabFocusMode);
  }

  show() {
    if (this.editor.getOption(monaco.editor.EditorOption.readOnly)) return;
    this.node.textContent = this.movesFocus()
      ? `Tab moves focus · ${TAB_KEY_TOGGLE_LABEL}: Tab indents`
      : `Tab indents · ${TAB_KEY_TOGGLE_LABEL}: Tab moves focus`;
    this.node.hidden = false;
  }

  hide() {
    this.node.hidden = true;
  }

  dispose() {
    for (const listener of this.listeners) listener.dispose();
    this.action.dispose();
    this.editor.removeOverlayWidget(this.widget);
  }
}
