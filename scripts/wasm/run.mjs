// Runs a WebAssembly module bcc compiled, under Node's WASI preview 1 for
// a wasm32-wasip1 module (which exports `_start`), or validates any module.
//
//   node --no-warnings scripts/wasm/run.mjs program.wasm [args...]
//   node scripts/wasm/run.mjs --validate module.wasm
//
// The exit status is the program's; a trap prints its message to stderr
// and exits with 132, as a native trap (SIGILL) reports 128 + 4.
import { readFileSync } from 'node:fs';
import { WASI } from 'node:wasi';

const argv = process.argv.slice(2);
if (argv[0] === '--validate') {
  const bytes = readFileSync(argv[1]);
  try {
    new WebAssembly.Module(bytes);
  } catch (error) {
    console.error(`invalid module: ${error.message}`);
    process.exit(1);
  }
  process.exit(0);
}
const [path, ...args] = argv;
const bytes = readFileSync(path);
const wasi = new WASI({ version: 'preview1', args: [path, ...args], env: {}, returnOnExit: true });
let instance;
try {
  ({ instance } = await WebAssembly.instantiate(bytes, { wasi_snapshot_preview1: wasi.getImportObject().wasi_snapshot_preview1 }));
} catch (error) {
  console.error(`cannot instantiate: ${error.message}`);
  process.exit(1);
}
try {
  process.exitCode = wasi.start(instance);
} catch (error) {
  console.error(`trap: ${error.message}`);
  process.exitCode = 132;
}
