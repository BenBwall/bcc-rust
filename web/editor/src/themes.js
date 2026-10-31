// Editor themes matching the site (web/style.css). Every editor is one of
// the device's black screens, in daylight and under low light alike, so both
// themes are dark; the low-light screen is a shade deeper. Each token kind
// has one colour in every language, few and muted, as on a hardware
// sequencer's screen: keywords warm, types and variables blue, numbers ochre,
// strings green, the rest off-white and grey.

const screen = {
  background: '#0b0b0b',
  foreground: '#ecebe4',
  muted: '#9a9991',
  keyword: '#ff9466',
  type: '#7fb0ff',
  number: '#e8b45c',
  string: '#9fd49a',
  function: '#f2f2ec',
  variable: '#a8c7ff',
  slot: '#c9b3ff',
  label: '#ffb3c8',
  register: '#ff9ea9',
  operator: '#b5b4ac',
};

const dark = { ...screen, background: '#050505' };
const light = screen;

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

function colors(palette) {
  return {
    'editor.background': palette.background,
    'editor.foreground': palette.foreground,
    'editorGutter.background': palette.background,
    'editorLineNumber.foreground': '#7a7a73',
    // The number of the empty line after a final new-line, which Monaco
    // otherwise dims below 4.5:1.
    'editorLineNumber.dimmedForeground': '#7a7a73',
    'editorLineNumber.activeForeground': '#f2f2ec',
    'editor.lineHighlightBackground': '#ffffff08',
    'editor.lineHighlightBorder': '#00000000',
    // Other places of the word under the cursor: a faint fill with an
    // outline keeps every token readable.
    'editor.wordHighlightBackground': '#ffffff12',
    'editor.wordHighlightBorder': '#ffffff30',
    'editor.wordHighlightStrongBackground': '#ffffff12',
    'editor.wordHighlightStrongBorder': '#ffffff30',
    'editor.wordHighlightTextBackground': '#ffffff12',
    'editor.wordHighlightTextBorder': '#ffffff30',
    'editor.selectionBackground': '#ff5a1f55',
    'editor.inactiveSelectionBackground': '#ff5a1f30',
    'editorCursor.foreground': '#ff5a1f',
    'editorIndentGuide.background1': '#ffffff0c',
    'editorIndentGuide.activeBackground1': '#ffffff22',
    'editorBracketMatch.background': '#ff5a1f22',
    'editorBracketMatch.border': '#ff5a1f88',
    'editorWidget.background': '#141413',
    'editorWidget.border': '#3a3a36',
    'editorHoverWidget.background': '#141413',
    'editorHoverWidget.border': '#3a3a36',
    'scrollbarSlider.background': '#ffffff14',
    'scrollbarSlider.hoverBackground': '#ffffff26',
    'focusBorder': '#ff5a1f',
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
    colors: colors(dark),
  });
  monaco.editor.defineTheme('bcc-light', {
    base: 'vs-dark',
    inherit: false,
    rules: rules(light),
    colors: colors(light),
  });
}
