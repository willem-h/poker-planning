// Guards the wasm modules against a known way for the build to produce a file
// that loads everywhere in testing and fails in someone's browser.
//
// wasm-bindgen's glue calls `__wbindgen_externrefs.grow(4)` the moment a module
// instantiates. If anything in the pipeline gives that table a maximum equal to
// its initial size, that call throws and the whole app is dead on arrival —
// with no sign of trouble at build time. See
// https://github.com/WebAssembly/binaryen/issues/4711
import { readFile } from 'node:fs/promises';

const FUNCREF = 0x70;
const EXTERNREF = 0x6f;

function leb(bytes, i) {
  let result = 0;
  let shift = 0;
  for (;;) {
    const byte = bytes[i++];
    result |= (byte & 0x7f) << shift;
    shift += 7;
    if (!(byte & 0x80)) return [result, i];
  }
}

function tables(bytes) {
  const found = [];
  let i = 8; // past the magic number and version
  while (i < bytes.length) {
    const id = bytes[i++];
    let size;
    [size, i] = leb(bytes, i);
    const end = i + size;
    if (id === 4) {
      let count;
      let j;
      [count, j] = leb(bytes, i);
      for (let k = 0; k < count; k++) {
        const elementType = bytes[j++];
        const flags = bytes[j++];
        let initial;
        let max = null;
        [initial, j] = leb(bytes, j);
        if (flags & 1) [max, j] = leb(bytes, j);
        found.push({ elementType, initial, max });
      }
    }
    i = end;
  }
  return found;
}

let failed = false;

for (const path of process.argv.slice(2)) {
  const found = tables(await readFile(path));
  const externref = found.find((t) => t.elementType === EXTERNREF);
  const describe = (t) =>
    `${t.elementType === FUNCREF ? 'funcref' : 'externref'} ${t.initial}..${t.max ?? '∞'}`;

  if (!externref) {
    console.log(`  ${path}: no externref table [${found.map(describe).join(', ')}]`);
    continue;
  }

  if (externref.max !== null && externref.max <= externref.initial) {
    console.error(
      `  ${path}: FAIL — externref table cannot grow (initial ${externref.initial}, max ` +
        `${externref.max}). Instantiating this module would throw at ` +
        `__wbindgen_init_externref_table.`,
    );
    failed = true;
  } else {
    console.log(`  ${path}: ok [${found.map(describe).join(', ')}]`);
  }
}

if (failed) process.exit(1);
