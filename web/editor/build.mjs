// Bundle the Monaco glue (src/main.js) and Monaco's editor worker into
// dist/ with esbuild: main.js, main.css (with the codicon font beside it)
// and editor.worker.js. Copy the site's web fonts (Space Mono regular and
// bold, Newsreader italic; each SIL OFL) into fonts/, named with their
// package version so their URLs never change meaning. web/build.sh and the
// Pages workflow run this before Trunk, which copies both directories into
// the site (see index.html).
import { copyFileSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { build } from 'esbuild';

rmSync('dist', { recursive: true, force: true });
rmSync('fonts', { recursive: true, force: true });
mkdirSync('fonts');
for (const [name, files] of [
  ['space-mono', [['latin-400-normal', '400'], ['latin-700-normal', '700']]],
  ['newsreader', [['latin-400-italic', '400-italic']]],
]) {
  const directory = `node_modules/@fontsource/${name}`;
  const { version } = JSON.parse(readFileSync(`${directory}/package.json`, 'utf8'));
  for (const [file, style] of files) {
    copyFileSync(`${directory}/files/${name}-${file}.woff2`, `fonts/${name}-${style}-${version}.woff2`);
  }
  copyFileSync(`${directory}/LICENSE`, `fonts/${name}-LICENSE.txt`);
}

const common = {
  bundle: true,
  minify: true,
  format: 'esm',
  target: ['es2022'],
  legalComments: 'linked',
  logLevel: 'warning',
};

await build({
  ...common,
  entryPoints: { main: 'src/main.js' },
  outdir: 'dist',
  loader: { '.ttf': 'file' },
  assetNames: '[name]',
});
await build({
  ...common,
  entryPoints: { 'editor.worker': 'monaco-editor/editor/editor.worker.js' },
  outdir: 'dist',
});
console.log('Built the editor bundle in web/editor/dist/ and the fonts in web/editor/fonts/.');
