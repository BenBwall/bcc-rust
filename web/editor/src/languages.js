// Monarch grammars for the languages the site shows: C source (also the
// logical lines and the preprocessed text, with its line markers), the
// preprocessing-token listing, bcc's own IR (before and after ABI lowering),
// LLVM IR, AT&T-syntax x86-64 assembly, and the objdump-style machine-code
// listing. Token names map to the colours in
// themes.js, so the same kind of thing (a type, a value, a label) has the
// same colour in every view.

const C_KEYWORDS = [
  'auto', 'break', 'case', 'const', 'continue', 'default', 'do', 'else', 'enum', 'extern',
  'for', 'goto', 'if', 'inline', 'register', 'restrict', 'return', 'sizeof', 'static',
  'struct', 'switch', 'typedef', 'union', 'volatile', 'while', '_Alignas', '_Alignof',
  '_Atomic', '_Generic', '_Noreturn', '_Static_assert', '_Thread_local',
];
const C_TYPES = [
  'void', 'char', 'short', 'int', 'long', 'float', 'double', 'signed', 'unsigned', '_Bool',
  '_Complex', '_Imaginary', 'size_t', 'ptrdiff_t', 'bool',
];

// `@brackets` actions need the language to list its bracket pairs.
const brackets = [
  { open: '{', close: '}', token: 'delimiter.curly' },
  { open: '[', close: ']', token: 'delimiter.square' },
  { open: '(', close: ')', token: 'delimiter.parenthesis' },
];

const c = {
  defaultToken: '',
  brackets,
  keywords: C_KEYWORDS,
  types: C_TYPES,
  operators: /[=><!~?:&|+\-*/^%]+/,
  tokenizer: {
    root: [
      // A line marker in preprocessed text: `# 12 "file.c" 2`.
      [/^# \d+ "[^"]*"( \d)*$/, 'comment.address'],
      [/^\s*#\s*\w+/, 'keyword.directive'],
      [/[a-zA-Z_]\w*(?=\s*\()/, { cases: { '@keywords': 'keyword', '@types': 'type', '@default': 'function' } }],
      [/[a-zA-Z_]\w*/, { cases: { '@keywords': 'keyword', '@types': 'type', '@default': 'identifier' } }],
      { include: '@whitespace' },
      [/0[xX][0-9a-fA-F]+[uUlL]*/, 'number'],
      [/\d+\.\d*([eE][-+]?\d+)?[fFlL]?/, 'number'],
      [/\d+[uUlL]*/, 'number'],
      [/"([^"\\]|\\.)*$/, 'string.invalid'],
      [/"/, 'string', '@string'],
      [/'([^'\\]|\\.)'/, 'string'],
      [/[{}()[\]]/, '@brackets'],
      [/[;,.]/, 'delimiter'],
      [/@operators/, 'operator'],
    ],
    whitespace: [
      [/[ \t\r\n]+/, ''],
      [/\/\*/, 'comment', '@comment'],
      [/\/\/.*$/, 'comment'],
    ],
    comment: [
      [/[^/*]+/, 'comment'],
      [/\*\//, 'comment', '@pop'],
      [/[/*]/, 'comment'],
    ],
    string: [
      [/[^\\"]+/, 'string'],
      [/\\./, 'string.escape'],
      [/"/, 'string', '@pop'],
    ],
  },
};

// bcc IR: `function @f(i32) -> i32 external {`, `block0(v0: i32):`,
// `v3 = iadd.i32 nsw v1, v2`, `slot0 = stack_slot 12, align 4`.
const bccIr = {
  defaultToken: '',
  brackets,
  keywords: [
    'function', 'external', 'internal', 'target', 'triple', 'datalayout', 'abi', 'lowered',
    'align', 'noundef', 'nsw', 'nuw', 'inbounds', 'exact', 'signext', 'zeroext', 'sret',
    'byval', 'global', 'constant', 'declare',
  ],
  instructions: ['return', 'jump', 'brif', 'br_table', 'call', 'unreachable', 'trap'],
  tokenizer: {
    root: [
      [/;.*$/, 'comment'],
      [/"([^"\\]|\\.)*"/, 'string'],
      [/@[\w.$]+/, 'function'],
      [/\bblock\d+\b/, 'label'],
      [/\bslot\d+\b/, 'variable.slot'],
      [/\bv\d+\b/, 'variable'],
      [/(=\s*)([a-z_]+)((?:\.[a-z0-9]+)?)/, ['operator', 'keyword.instruction', 'type']],
      [/^(\s+)([a-z_]+)((?:\.[a-z0-9]+)?)(?=\s)/, ['', 'keyword.instruction', 'type']],
      [/\b(i1|i8|i16|i32|i64|i128|f32|f64|ptr|void)\b/, 'type'],
      [/\b(eq|ne|slt|sle|sgt|sge|ult|ule|ugt|uge)\b/, 'keyword.condition'],
      [/[a-z_]\w*/, { cases: { '@instructions': 'keyword.instruction', '@keywords': 'keyword', '@default': 'identifier' } }],
      [/-?\d+/, 'number'],
      [/->|[=:,]/, 'operator'],
      [/[{}()[\]]/, '@brackets'],
    ],
  },
};

const llvm = {
  defaultToken: '',
  brackets: [...brackets, { open: '<', close: '>', token: 'delimiter.angle' }],
  keywords: [
    'define', 'declare', 'target', 'triple', 'datalayout', 'source_filename', 'global',
    'constant', 'private', 'internal', 'external', 'dso_local', 'noundef', 'nsw', 'nuw',
    'inbounds', 'align', 'label', 'to', 'x', 'zeroinitializer', 'undef', 'poison', 'null',
    'true', 'false', 'signext', 'zeroext', 'nonnull', 'tail', 'attributes', 'unnamed_addr',
  ],
  instructions: [
    'ret', 'br', 'switch', 'unreachable', 'add', 'sub', 'mul', 'sdiv', 'udiv', 'srem', 'urem',
    'shl', 'lshr', 'ashr', 'and', 'or', 'xor', 'alloca', 'load', 'store', 'getelementptr',
    'trunc', 'zext', 'sext', 'ptrtoint', 'inttoptr', 'bitcast', 'icmp', 'fcmp', 'phi', 'call',
    'select', 'freeze', 'fadd', 'fsub', 'fmul', 'fdiv',
  ],
  tokenizer: {
    root: [
      [/;.*$/, 'comment'],
      [/"([^"\\]|\\.)*"/, 'string'],
      [/![\w.]+/, 'keyword.metadata'],
      [/@[\w.$]+/, 'function'],
      [/%[\w.$]+/, 'variable'],
      [/^[\w.$]+:/, 'label'],
      [/\b(i\d+|ptr|void|half|float|double)\b/, 'type'],
      [/\b(eq|ne|slt|sle|sgt|sge|ult|ule|ugt|uge)\b/, 'keyword.condition'],
      [/#\d+/, 'keyword.metadata'],
      [/[a-z_][\w.]*/, { cases: { '@instructions': 'keyword.instruction', '@keywords': 'keyword', '@default': 'identifier' } }],
      [/-?\d+/, 'number'],
      [/[=,*]/, 'operator'],
      [/[{}()[\]<>]/, '@brackets'],
    ],
  },
};

// AT&T syntax: `movl $2, 0(%rsp)`, `.LBB1_0:`, `.cfi_offset %rbx, -16`.
// Monarch keeps state across lines, so these grammars use one root state and
// anchor the line-leading forms with `^`.
const operands = [
  [/%[a-z0-9]+/, 'register'],
  [/\$-?(0x[0-9a-fA-F]+|\d+)/, 'number.immediate'],
  [/-?(0x[0-9a-fA-F]+|\d+)/, 'number'],
  [/"([^"\\]|\\.)*"/, 'string'],
  [/<[^>]+>/, 'label'],
  [/\.?[A-Za-z_][\w.$]*/, 'function'],
  [/[(),+]/, 'delimiter'],
];

const asm = {
  defaultToken: '',
  tokenizer: {
    root: [
      [/#.*$/, 'comment'],
      [/^[\w.$]+:/, 'label'],
      [/^\s*\.\w+/, 'keyword.directive'],
      [/^\s*[a-z][a-z0-9]*/, 'keyword.instruction'],
      ...operands,
    ],
  },
};

// The site's machine-code listing: `<main>:` headings, then an offset, the
// encoded bytes, and the disassembled instruction.
const objdump = {
  defaultToken: '',
  tokenizer: {
    root: [
      [/^<[^>]+>:/, 'label'],
      [
        /^(\s*[0-9a-f]+)(\s+)((?:[0-9a-f]{2} )*[0-9a-f]{2})(\s+)([a-z][a-z0-9]*)/,
        ['comment.address', '', 'number.bytes', '', 'keyword.instruction'],
      ],
      ...operands,
    ],
  },
};

// bcc's token dump: `objects.c:1:5: identifier `bump``.
const tokens = {
  defaultToken: '',
  tokenizer: {
    root: [
      [/^[^:\s]+:\d+:\d+:/, 'comment.address'],
      [/`[^`]*`/, 'string'],
      [/\b(keyword|identifier|punctuator|integer constant|string literal|character constant|floating constant)\b/, 'type'],
      [/=\s*-?\d+/, 'number'],
      [/\([a-z ]+\)/, 'keyword'],
    ],
  },
};

// Phase 3's listing: `6:5     identifier          long_name   notes`. The
// spelling is coloured by its kind; white space and new-lines are muted.
const ppEntry = (kind, token, spelling = /\S+/) => [
  new RegExp(`^(\\d+:\\d+\\s+)(${kind})(\\s+)(${spelling.source})`),
  ['comment.address', 'type', '', token],
];
const ppTokens = {
  defaultToken: 'comment',
  tokenizer: {
    root: [
      ppEntry('identifier', 'identifier'),
      ppEntry('pp-number', 'number'),
      ppEntry('string-literal', 'string', /(?:u8|[uUL])?"(?:[^"\\]|\\.)*"/),
      ppEntry('character-constant', 'string', /[uUL]?'(?:[^'\\]|\\.)*'/),
      ppEntry('punctuator', 'keyword'),
      ppEntry('other', 'register'),
      ppEntry('whitespace|newline', 'comment.address', /'[^']*'/),
      [/.+$/, 'comment'],
    ],
  },
};

// bcc's syntax tree: `  block-item: declaration type=int @2:5`.
const syntaxTree = {
  defaultToken: '',
  tokenizer: {
    root: [
      [/@\d+:\d+/, 'comment.address'],
      [/^\s*[a-z-]+:(?=\s)/, 'keyword'],
      [/\b[a-z-]+=/, 'variable.slot'],
      [/\b(identifier)(\s+)(\w+)/, ['type', '', 'identifier']],
      [/\b(constant)(\s+)(-?\d+)/, ['type', '', 'number']],
      [/\b(binary|unary)(\s+)(\S+)/, ['type', '', 'operator']],
      [/^\s*[a-z][a-z-]*/, 'type'],
      [/\(int\)|\(char\)/, 'keyword'],
    ],
  },
};

// The semantic view (`--semantic-types --syntax-locations`): scopes, then
// `object total type=int duration=automatic size=4 align=4 @8:9`, then
// `expression 3 type=int category=rvalue` and `convert expression 3 …`.
const semanticTypes = {
  defaultToken: '',
  tokenizer: {
    root: [
      [/@\d+:\d+/, 'comment.address'],
      [/^(target)(\s+)(\S+)/, ['keyword', '', 'type']],
      [/^(\s*)(object|function|parameter|typedef|enumerator|member)(\s+)([\w$]+)/, ['', 'keyword', '', 'function']],
      [/^(\s*)(scope)(\s+)(\d+)(\s+)([a-z-]+)/, ['', 'keyword', '', 'number', '', 'label']],
      [/^\s*(convert expression|expression|tag|target|type-name)\b/, 'keyword'],
      [/\b(type|operation)(=)("[^"]*"|[\w$]+)/, ['variable.slot', 'operator', 'type']],
      [/\b(binding)(=)([\w$]+)/, ['variable.slot', 'operator', 'variable']],
      [/\b(category|kind|duration|linkage|constant)(=)([a-z-]+)/, ['variable.slot', 'operator', 'label']],
      [/\b([a-z-]+)(=)/, ['variable.slot', 'operator']],
      [/"[^"]*"/, 'string'],
      [/-?\b\d+\b/, 'number'],
    ],
  },
};

// The WebAssembly text format as the stepper lists it: one instruction per
// line, `;; label = @1` comments on blocks and `(;@1;)` on branches.
const wat = {
  defaultToken: '',
  brackets: [{ open: '(', close: ')', token: 'delimiter.parenthesis' }],
  keywords: [
    'module', 'func', 'param', 'result', 'local', 'type', 'import', 'export', 'memory', 'table',
    'global', 'data', 'elem', 'start', 'mut', 'declare', 'funcref', 'externref',
  ],
  control: [
    'block', 'loop', 'if', 'else', 'end', 'br', 'br_if', 'br_table', 'return', 'call',
    'call_indirect', 'drop', 'select', 'unreachable', 'nop',
  ],
  tokenizer: {
    root: [
      [/;;.*$/, 'comment'],
      [/\(;[^;]*;\)/, 'comment'],
      [/"([^"\\]|\\.)*"/, 'string'],
      [/(call)(\s+)(\$[\w.$]+)/, ['keyword.instruction', '', 'function']],
      [/(func)(\s+)(\$[\w.$]+)/, ['keyword', '', 'function']],
      [/\$__stack_pointer\b/, 'variable.slot'],
      [/\$[\w.$]+/, 'variable'],
      [/\b(i32|i64|f32|f64)\.[a-z0-9_]+/, 'keyword.instruction'],
      [/\b(memory|local|global|table|ref|data|elem)\.[a-z_]+/, 'keyword.instruction'],
      [/\b(i32|i64|f32|f64|v128)\b/, 'type'],
      [/\b(offset|align)(=)/, ['keyword', 'operator']],
      [/[a-z_][\w.]*/, { cases: { '@control': 'keyword.instruction', '@keywords': 'keyword', '@default': 'identifier' } }],
      [/-?(0x[0-9a-fA-F]+|\d+(\.\d*)?([eE][-+]?\d+)?|inf|nan(:0x[0-9a-fA-F]+)?)/, 'number'],
      [/[()]/, '@brackets'],
    ],
  },
};

export function registerLanguages(monaco) {
  const languages = {
    c,
    'bcc-ir': bccIr,
    llvm,
    asm,
    objdump,
    tokens,
    'pp-tokens': ppTokens,
    'syntax-tree': syntaxTree,
    'semantic-types': semanticTypes,
    wat,
  };
  for (const [id, grammar] of Object.entries(languages)) {
    monaco.languages.register({ id });
    monaco.languages.setMonarchTokensProvider(id, grammar);
  }
  monaco.languages.setLanguageConfiguration('c', {
    comments: { lineComment: '//', blockComment: ['/*', '*/'] },
    brackets: [['{', '}'], ['[', ']'], ['(', ')']],
    autoClosingPairs: [
      { open: '{', close: '}' },
      { open: '[', close: ']' },
      { open: '(', close: ')' },
      { open: '"', close: '"', notIn: ['string'] },
      { open: '/*', close: ' */', notIn: ['string'] },
    ],
    surroundingPairs: [
      { open: '{', close: '}' },
      { open: '(', close: ')' },
      { open: '[', close: ']' },
    ],
  });
  monaco.languages.setLanguageConfiguration('wat', {
    comments: { lineComment: ';;', blockComment: ['(;', ';)'] },
    brackets: [['(', ')']],
  });
  for (const id of ['bcc-ir', 'llvm']) {
    monaco.languages.setLanguageConfiguration(id, {
      comments: { lineComment: ';' },
      brackets: [['{', '}'], ['[', ']'], ['(', ')']],
    });
  }
}
