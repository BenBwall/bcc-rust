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

/** Switch every editor between the site's `light` and `dark` themes. */
export function setTheme(theme) {
  monaco.editor.setTheme(theme === 'light' ? 'bcc-light' : 'bcc-dark');
}

/**
 * Create an editor in `host`. `options`: `value`, `language` (`c`, `bcc-ir`,
 * `llvm`, `asm`, `objdump` or `plaintext`), `readOnly`, `ariaLabel`, `theme`.
 */
export function createEditor(host, options) {
  return new Editor(host, options);
}

class Editor {
  constructor(host, options) {
    this.decorations = new Map();
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
      glyphMargin: !options.readOnly,
      lineNumbersMinChars: 3,
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
      wordWrap: 'off',
      stickyScroll: { enabled: false },
      accessibilitySupport: 'auto',
      quickSuggestions: false,
      suggestOnTriggerCharacters: false,
      contextmenu: false,
      tabFocusMode: false,
    });
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

  setReadOnly(readOnly) {
    this.editor.updateOptions({ readOnly, domReadOnly: readOnly });
  }

  /** Call `callback(text)` after every edit. */
  onChange(callback) {
    this.editor.onDidChangeModelContent(() => callback(this.editor.getValue()));
  }

  /** Call `callback(line)` (0 when leaving) as the pointer moves over lines. */
  onHoverLine(callback) {
    let current = 0;
    this.editor.onMouseMove((event) => {
      const line = event.target.position?.lineNumber ?? 0;
      if (line !== current) {
        current = line;
        callback(line);
      }
    });
    this.editor.onMouseLeave(() => {
      if (current !== 0) {
        current = 0;
        callback(0);
      }
    });
  }

  /** Call `callback(line)` when the gutter (line number or glyph) is clicked. */
  onGutterClick(callback) {
    this.editor.onMouseDown((event) => {
      const type = event.target.type;
      if (
        type === monaco.editor.MouseTargetType.GUTTER_LINE_NUMBERS ||
        type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN
      ) {
        callback(event.target.position.lineNumber);
      }
    });
  }

  /** Call `callback(line)` when the cursor moves to another line. */
  onCursorLine(callback) {
    let current = 0;
    this.editor.onDidChangeCursorPosition((event) => {
      if (event.position.lineNumber !== current) {
        current = event.position.lineNumber;
        callback(current);
      }
    });
  }

  /**
   * Replace the decoration layer `layer` with `items`: each has 1-based
   * `startLine`, `startColumn`, `endLine`, `endColumn` (columns may be 0 for
   * whole-line marks), and any of `inlineClass`, `lineClass`, `marginClass`,
   * `glyphClass` and `hover` (Markdown).
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
        glyphMarginClassName: item.glyphClass || undefined,
        hoverMessage: item.hover ? { value: item.hover } : undefined,
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

  /** Scroll so `line` is visible, centring it only when it is off screen. */
  revealLine(line) {
    if (line > 0) {
      this.editor.revealLineInCenterIfOutsideViewport(line, monaco.editor.ScrollType.Smooth);
    }
  }

  focus() {
    this.editor.focus();
  }

  layout() {
    this.editor.layout();
  }

  dispose() {
    this.editor.dispose();
  }
}
