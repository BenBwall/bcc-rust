// Editor themes matching the site's palette (web/style.css). Each token kind
// has one colour in every language: types are blue, values and variables
// light blue, labels pink, instructions and keywords violet, numbers amber.

const dark = {
  background: '#0f141d',
  foreground: '#dfe6f0',
  muted: '#9fa7b6',
  keyword: '#c4a7ff',
  type: '#7cc7f7',
  number: '#f5b86a',
  string: '#8fdca3',
  function: '#5fd8c4',
  variable: '#a5c8ff',
  slot: '#e3b8ff',
  label: '#f59fc8',
  register: '#ff9ea9',
  operator: '#b6c2d4',
};

const light = {
  background: '#ffffff',
  foreground: '#1b2230',
  muted: '#58606d',
  keyword: '#7036d6',
  type: '#0b62a3',
  number: '#964a09',
  string: '#156d35',
  function: '#0a6b60',
  variable: '#1f4fbf',
  slot: '#8a2fb8',
  label: '#b0185f',
  register: '#b4233a',
  operator: '#475266',
};

function rules(palette) {
  const strip = (color) => color.slice(1);
  return [
    { token: '', foreground: strip(palette.foreground) },
    { token: 'comment', foreground: strip(palette.muted), fontStyle: 'italic' },
    { token: 'comment.address', foreground: strip(palette.muted), fontStyle: '' },
    { token: 'keyword', foreground: strip(palette.keyword) },
    { token: 'keyword.instruction', foreground: strip(palette.keyword), fontStyle: 'bold' },
    { token: 'keyword.directive', foreground: strip(palette.muted) },
    { token: 'keyword.condition', foreground: strip(palette.label) },
    { token: 'keyword.metadata', foreground: strip(palette.muted) },
    { token: 'type', foreground: strip(palette.type) },
    { token: 'number', foreground: strip(palette.number) },
    { token: 'number.immediate', foreground: strip(palette.number) },
    { token: 'number.bytes', foreground: strip(palette.muted) },
    { token: 'string', foreground: strip(palette.string) },
    { token: 'string.escape', foreground: strip(palette.number) },
    { token: 'function', foreground: strip(palette.function) },
    { token: 'variable', foreground: strip(palette.variable) },
    { token: 'variable.slot', foreground: strip(palette.slot) },
    { token: 'label', foreground: strip(palette.label) },
    { token: 'register', foreground: strip(palette.register) },
    { token: 'operator', foreground: strip(palette.operator) },
    { token: 'delimiter', foreground: strip(palette.operator) },
    { token: 'identifier', foreground: strip(palette.foreground) },
  ];
}

function colors(palette, isDark) {
  return {
    'editor.background': palette.background,
    'editor.foreground': palette.foreground,
    'editorGutter.background': palette.background,
    'editorLineNumber.foreground': isDark ? '#768296' : '#6b7484',
    // The number of the empty line after a final new-line, which Monaco
    // otherwise dims to 1.7:1.
    'editorLineNumber.dimmedForeground': isDark ? '#768296' : '#6b7484',
    'editorLineNumber.activeForeground': isDark ? '#c3cddb' : '#2b3445',
    'editor.lineHighlightBackground': isDark ? '#ffffff08' : '#0b62a308',
    'editor.lineHighlightBorder': '#00000000',
    // Other places of the word under the cursor. Monaco's light default
    // drops tokens below 4.5:1 where it meets the cursor's line; a light
    // fill with an outline keeps every token readable.
    'editor.wordHighlightBackground': isDark ? '#ffffff12' : '#0b62a30d',
    'editor.wordHighlightBorder': isDark ? '#ffffff30' : '#0b62a340',
    'editor.wordHighlightStrongBackground': isDark ? '#ffffff12' : '#0b62a30d',
    'editor.wordHighlightStrongBorder': isDark ? '#ffffff30' : '#0b62a340',
    'editor.wordHighlightTextBackground': isDark ? '#ffffff12' : '#0b62a30d',
    'editor.wordHighlightTextBorder': isDark ? '#ffffff30' : '#0b62a340',
    'editor.selectionBackground': isDark ? '#3b82f640' : '#3b82f62e',
    'editor.inactiveSelectionBackground': isDark ? '#3b82f626' : '#3b82f61c',
    'editorCursor.foreground': isDark ? '#5eead4' : '#0b7a6d',
    'editorIndentGuide.background1': isDark ? '#ffffff0c' : '#0000000c',
    'editorIndentGuide.activeBackground1': isDark ? '#ffffff22' : '#00000022',
    'editorBracketMatch.background': isDark ? '#5eead422' : '#0b7a6d18',
    'editorBracketMatch.border': isDark ? '#5eead466' : '#0b7a6d55',
    'editorWidget.background': isDark ? '#171e2b' : '#ffffff',
    'editorWidget.border': isDark ? '#2a3446' : '#dfe4ec',
    'editorHoverWidget.background': isDark ? '#171e2b' : '#ffffff',
    'editorHoverWidget.border': isDark ? '#2a3446' : '#dfe4ec',
    'scrollbarSlider.background': isDark ? '#ffffff14' : '#0000001a',
    'scrollbarSlider.hoverBackground': isDark ? '#ffffff26' : '#0000002b',
    'focusBorder': isDark ? '#5eead4' : '#0b7a6d',
    // Monaco still tags brackets with its pair-colouring classes when that
    // feature is off; give every level the delimiter colour so brackets
    // match the rest of the palette instead of Monaco's rainbow.
    ...Object.fromEntries(
      [1, 2, 3, 4, 5, 6].map((level) => [`editorBracketHighlight.foreground${level}`, palette.operator]),
    ),
    'editorBracketHighlight.unexpectedBracket.foreground': palette.operator,
  };
}

export function defineThemes(monaco) {
  monaco.editor.defineTheme('bcc-dark', {
    base: 'vs-dark',
    inherit: false,
    rules: rules(dark),
    colors: colors(dark, true),
  });
  monaco.editor.defineTheme('bcc-light', {
    base: 'vs',
    inherit: false,
    rules: rules(light),
    colors: colors(light, false),
  });
}
