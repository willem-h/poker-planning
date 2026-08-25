// Drives three real browsers through a whole session against a running relay.
// See tests/README.md for what to start first.
//
//   node tests/browser.mjs
//
import { chromium } from 'playwright';

const APP = process.env.APP_URL ?? 'http://localhost:8080/index.html';
const TRANSPORT = process.env.TRANSPORT ?? 'iroh';

// Each transport needs its own server pointed at: iroh relays the session,
// WebRTC only needs somewhere to exchange the handshake.
const params = new URLSearchParams({ transport: TRANSPORT });
if (TRANSPORT === 'iroh') {
  params.set('relay', process.env.RELAY_URL ?? 'http://localhost:3340');
} else {
  params.set('signal', process.env.SIGNAL_URL ?? 'ws://localhost:8081');
}
const URL = `${APP}?${params}`;

const log = (tag, ...a) => console.log(`[${tag}]`, ...a);

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM_PATH || undefined,
});

async function newPeer(tag) {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  page.on('console', (m) => {
    if (m.type() === 'error') log(tag + ':console', m.text().slice(0, 200));
  });
  page.on('pageerror', (e) => log(tag + ':pageerror', String(e).slice(0, 400)));
  await page.goto(URL, { waitUntil: 'load' });
  return page;
}

const status = (p) => p.locator('#status-text').innerText();

const host = await newPeer('host');
await host.fill('#input-name', 'Ada');
await host.click('#btn-start');

// Wait for the invite link to appear (endpoint online + subscribed)
await host.waitForSelector('#view-room.is-active', { timeout: 90000 });
await host.waitForFunction(() => document.getElementById('invite-url').value.length > 0, null, { polling: 500, timeout: 90000 });
const invite = await host.locator('#invite-url').inputValue();
log('host', 'status:', await status(host));
log('host', `transport: ${TRANSPORT}, invite length: ${invite.length}`);

const guest = await newPeer('guest');
await guest.fill('#input-name', 'Grace');
await guest.fill('#input-invite', invite);
await guest.click('#btn-start');
await guest.waitForSelector('#view-room.is-active', { timeout: 90000 });

// Both should see 2 participants
async function waitPeers(p, n, tag) {
  await p.waitForFunction((n) => document.querySelectorAll('.seat').length === n, n, { timeout: 90000 })
    .catch(async () => { throw new Error(`${tag} never saw ${n} peers; saw ${await p.locator('.seat').count()}, status=${await status(p)}`); });
}
await waitPeers(host, 2, 'host');
await waitPeers(guest, 2, 'guest');
log('OK', 'both peers see each other. host status:', await status(host), '| guest status:', await status(guest));

// Vote
await host.locator('.card-btn[data-value="5"]').click();
await guest.locator('.card-btn[data-value="8"]').click();

// Both should show 2 hidden votes, no values leaked
await host.waitForFunction(() => document.querySelectorAll('.seat-hidden').length === 2, null, { polling: 500, timeout: 30000 });
await guest.waitForFunction(() => document.querySelectorAll('.seat-hidden').length === 2, null, { polling: 500, timeout: 30000 });
const leaked = await guest.evaluate(() => [...document.querySelectorAll('.seat-face')].map((e) => e.textContent).join(','));
log('OK', 'votes hidden on guest; faces show:', leaked);

// Reveal from the guest — no host role, anyone can
await guest.click('#btn-reveal');
await host.waitForFunction(() => document.querySelectorAll('.seat-revealed').length === 2, null, { polling: 500, timeout: 30000 });
await guest.waitForFunction(() => document.querySelectorAll('.seat-revealed').length === 2, null, { polling: 500, timeout: 30000 });
const faces = await host.evaluate(() => [...document.querySelectorAll('.seat-face')].map((e) => e.textContent).sort().join(','));
const stats = await host.locator('#stats').innerText();
log('OK', 'revealed on host:', faces);
log('OK', 'stats:', stats.replace(/\n/g, ' | '));

// New round from the host
await host.click('#btn-reset');
await guest.waitForFunction(() => document.getElementById('round-label').textContent === 'Round 2', null, { polling: 500, timeout: 30000 });
await guest.waitForFunction(() => document.querySelectorAll('.seat-none').length === 2, null, { polling: 500, timeout: 30000 });
log('OK', 'guest advanced to', await guest.locator('#round-label').innerText());

// Last round's numbers must not linger over the new round.
for (const [tag, p] of [['host', host], ['guest', guest]]) {
  const visible = await p.locator('#stats').isVisible();
  if (visible) throw new Error(`${tag} still shows stats from the previous round`);
}
log('OK', 'stats cleared on the new round');

// Third peer joins mid-session using the guest's invite
const guestInvite = await guest.locator('#invite-url').inputValue();
const third = await newPeer('third');
await third.fill('#input-name', 'Linus');
await third.fill('#input-invite', guestInvite);
await third.click('#btn-start');
await third.waitForSelector('#view-room.is-active', { timeout: 90000 });
await waitPeers(third, 3, 'third');
await waitPeers(host, 3, 'host');
log('OK', 'third peer joined via a non-creator invite; all see 3');

// What each connection actually settled on. This is the difference between the
// two transports, so assert it rather than just printing it.
// One path per other participant: WebRTC meshes the room, so with three peers
// present the host holds two connections of its own.
const expectedPaths = TRANSPORT === 'webrtc' ? 2 : 1;
await host
  .waitForFunction(
    (n) => {
      const paths = [...document.querySelectorAll('.path')];
      return paths.length === n && paths.every((e) => !e.textContent.endsWith('pending'));
    },
    expectedPaths,
    { polling: 500, timeout: 60000 },
  )
  .catch(async () => {
    throw new Error(
      `host never settled ${expectedPaths} connection paths; saw ` +
        (await host.evaluate(() => [...document.querySelectorAll('.path')].map((e) => e.textContent))),
    );
  });

const paths = await host.evaluate(() =>
  [...document.querySelectorAll('.path')].map((e) => ({ text: e.textContent, detail: e.title })),
);
log('paths', JSON.stringify(paths));
if (TRANSPORT === 'webrtc') {
  if (paths.length === 0) throw new Error('no connection paths reported');
  const relayed = paths.filter((p) => p.text.endsWith('relay'));
  if (relayed.length) throw new Error(`expected direct connections, got relayed: ${JSON.stringify(relayed)}`);
  log('OK', `${paths.length} direct peer connections, no relay in the path`);
} else {
  if (!paths.some((p) => p.text.endsWith('relay'))) {
    throw new Error('iroh in a browser should report a relayed path');
  }
  log('OK', 'iroh reports the session as relayed, as expected');
}

// A peer that closes its tab should drop off the table on its own.
await third.context().close();
await host.waitForFunction(() => document.querySelectorAll('.seat').length === 2, null, { polling: 500, timeout: 60000 })
  .catch(() => { throw new Error('host never dropped the peer that left'); });
log('OK', 'departed peer expired from the table');

await host.locator('.card-btn[data-value="8"]').click();
await guest.locator('.card-btn[data-value="8"]').click();
await guest.click('#btn-reveal');
await host.waitForFunction(() => document.querySelectorAll('.seat-revealed').length === 2, null, { polling: 500, timeout: 30000 });
log('OK', 'consensus round:', (await host.locator('#stats').innerText()).replace(/\n/g, ' | '));

// Leaving puts us back on the form and clears the room from the URL.
await guest.click('#btn-leave');
await guest.waitForSelector('#view-home.is-active', { timeout: 30000 });
if (guest.url().includes('#room=')) throw new Error('leaving left the room in the URL');
await host.waitForFunction(() => document.querySelectorAll('.seat').length === 1, null, { polling: 500, timeout: 60000 })
  .catch(() => { throw new Error('host still shows the peer that left deliberately') });
log('OK', 'leave returns to the form and clears the table');

await browser.close();
log('DONE', 'all checks passed');
