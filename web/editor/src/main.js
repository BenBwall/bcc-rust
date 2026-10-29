// Monaco glue for the bcc site. The Leptos page (web/src/editor.rs) imports
// this module at run time through wasm-bindgen and drives it with a small,
// plain-data API: create an editor in a host element, set its text,
// language and decorations, and listen for edits and line hovers. Everything
// else (state, semantics, what to highlight) lives in Rust.
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
import { registerLanguages } from './languages.js';
import { defineThemes } from './themes.js';

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

/** The key that switches what Tab does in an editor: Alt+M (Option+M on a
 * Mac). Monaco's own Ctrl+M (Ctrl+Shift+M on a Mac) cannot undo the
 * `tabFocusMode` option, and Firefox takes Ctrl+M to mute the tab. */
const TAB_KEY_TOGGLE = monaco.KeyMod.Alt | monaco.KeyCode.KeyM;
const TAB_KEY_TOGGLE_LABEL = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌥M' : 'Alt+M';

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
      fontFamily: "'JetBrains Mono Variable', 'JetBrains Mono', ui-monospace, 'SFMono-Regular', Menlo, Consolas, monospace",
      fontSize: 13.5,
      lineHeight: 22,
      fontLigatures: false,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      renderLineHighlight: options.readOnly ? 'none' : 'line',
      // Clicking a read-only view's line number selects the line; keep the
      // spaces of the selection blank.
      renderWhitespace: options.readOnly ? 'none' : 'selection',
      glyphMargin: !options.readOnly,
      // The WebAssembly listing runs to hundreds of lines.
      lineNumbersMinChars: options.language === 'wat' ? 4 : 3,
      lineDecorationsWidth: 10,
      folding: !!options.readOnly,
      smoothScrolling: true,
      cursorSmoothCaretAnimation: 'on',
      padding: { top: 14, bottom: 14 },
      scrollbar: { verticalScrollbarSize: 10, horizontalScrollbarSize: 10, useShadows: false },
      overviewRulerBorder: false,
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      guides: { indentation: true },
      bracketPairColorization: { enabled: false },
      tabSize: 4,
      // The source wraps rather than hiding the ends of long lines (its
      // comments, mostly) in the narrow columns of small screens; the
      // listings keep one instruction per line.
      wordWrap: options.readOnly ? 'off' : 'on',
      wrappingIndent: 'indent',
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
  }

  getValue() {
    return this.editor.getValue();
  }

  /** Replace the text, keeping the undo stack and the view when unchanged. */
  setValue(text) {
    if (this.editor.getValue() !== text) {
      this.editor.setValue(text);
    }
  }

  setLanguage(language) {
    monaco.editor.setModelLanguage(this.editor.getModel(), language);
    // The machine-code listing numbers its lines with code offsets.
    this.editor.updateOptions({ lineNumbers: language === 'objdump' ? 'off' : 'on' });
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

  /** Call `callback(line)` when the cursor moves to another line. */
  onCursorLine(callback) {
    let current = 0;
    this.listeners.push(this.editor.onDidChangeCursorPosition((event) => {
      if (event.position.lineNumber !== current) {
        current = event.position.lineNumber;
        callback(current);
      }
    }));
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
   * padding) and above the window's bottom, provided part of the editor is
   * already on screen: a reader who scrolled away to the memory diagram
   * stays there.
   */
  revealLine(line, onPage) {
    if (line <= 0) {
      return;
    }
    const editor = this.editor;
    const lineHeight = editor.getOption(monaco.editor.EditorOption.lineHeight);
    const height = editor.getLayoutInfo().height;
    const scrollTop = editor.getScrollTop();
    const top = editor.getTopForLineNumber(line);
    const inside = top >= scrollTop && top + lineHeight <= scrollTop + height;
    editor.revealLineInCenterIfOutsideViewport(line, monaco.editor.ScrollType.Smooth);
    if (!onPage) {
      return;
    }
    // Where the line ends up in the editor: where it is, or where the
    // (smooth) reveal is taking it, centred as far as the text allows.
    const centred = Math.min(Math.max(0, top - (height - lineHeight) / 2), Math.max(0, editor.getContentHeight() - height));
    const offset = top - (inside ? scrollTop : centred);
    const host = editor.getContainerDomNode().getBoundingClientRect();
    const clear = parseFloat(getComputedStyle(document.documentElement).scrollPaddingTop) || 0;
    const bottom = innerHeight - 8;
    if (host.bottom <= clear || host.top >= bottom) {
      return;
    }
    const lineTop = host.top + offset;
    if (lineTop < clear) {
      scrollBy({ top: lineTop - clear });
    } else if (lineTop + lineHeight > bottom) {
      scrollBy({ top: lineTop + lineHeight - bottom });
    }
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
