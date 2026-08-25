// Bundles Trystero into single ES modules under public/vendor/.
//
// The site is served as plain static files with no bundler in the browser, and
// Trystero's npm package is split across several scoped packages, so its
// imports have to be resolved ahead of time.
import { build } from 'esbuild';
import { mkdir, stat } from 'node:fs/promises';

const targets = [
  // The default: signaling over public Nostr relays, no server of ours.
  { entry: 'trystero/nostr', out: 'public/vendor/trystero-nostr.js' },
  // For a room that should not touch anything outside your network, and what
  // the browser test runs against.
  { entry: '@trystero-p2p/ws-relay', out: 'public/vendor/trystero-ws.js' },
];

await mkdir('public/vendor', { recursive: true });

for (const { entry, out } of targets) {
  await build({
    entryPoints: [entry],
    outfile: out,
    bundle: true,
    format: 'esm',
    platform: 'browser',
    minify: true,
    target: 'es2022',
  });
  const { size } = await stat(out);
  console.log(`${out}  ${(size / 1024).toFixed(1)} KB`);
}
