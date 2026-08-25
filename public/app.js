// UI layer. The room semantics live in the wasm module and the wire lives in a
// transport module; this file starts one, renders what it reports, and forwards
// clicks. It does not know which transport it got.

const el = (id) => document.getElementById(id);

const ui = {
  status: el('status'),
  statusText: el('status-text'),
  home: el('view-home'),
  room: el('view-room'),
  form: el('form-start'),
  name: el('input-name'),
  invite: el('input-invite'),
  start: el('btn-start'),
  homeError: el('home-error'),
  transportPicker: el('transport-picker'),
  transportBadge: el('transport-badge'),
  inviteUrl: el('invite-url'),
  copy: el('btn-copy'),
  leave: el('btn-leave'),
  peerCount: el('peer-count'),
  paths: el('paths'),
  roundLabel: el('round-label'),
  reveal: el('btn-reveal'),
  reset: el('btn-reset'),
  cards: el('cards'),
  stats: el('stats'),
  deck: el('deck'),
  myId: el('my-id'),
};

const NAME_KEY = 'poker.name';
const TRANSPORTS = { iroh: './transports/iroh.js', webrtc: './transports/webrtc.js' };
const DEFAULT_TRANSPORT = 'iroh';

let transport = null;
let handle = null;
let myVote = null;
let lastRound = null;
let pathTimer = null;

// --- url ------------------------------------------------------------------

const query = () => new URLSearchParams(location.search);

function inviteFromLocation() {
  return parseInvite(window.location.hash);
}

// Accepts a full invite URL, a bare fragment, or the room token on its own, so
// pasting from a chat window works whatever came along with it.
function parseInvite(text) {
  const match = text.match(/room=([^&\s]+)/);
  return decodeURIComponent(match ? match[1] : text.trim());
}

function chosenTransport() {
  const wanted = query().get('transport');
  return wanted in TRANSPORTS ? wanted : DEFAULT_TRANSPORT;
}

// Transport-specific overrides, all carried in the query string so they survive
// into the invite links people share and the whole room agrees on them.
function transportOptions() {
  const q = query();
  const turn = q.get('turn');
  return {
    relay: q.get('relay') ?? undefined,
    signal: q.get('signal') ?? undefined,
    turn: turn ? { urls: [turn], username: q.get('turnUser') ?? '', credential: q.get('turnPass') ?? '' } : undefined,
  };
}

// --- rendering -------------------------------------------------------------

function setStatus(kind, text) {
  ui.status.className = `status status-${kind}`;
  ui.statusText.textContent = text;
}

function showError(message) {
  ui.homeError.textContent = message;
  ui.homeError.hidden = false;
}

function renderDeck(values) {
  ui.deck.replaceChildren(
    ...values.map((value) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'card-btn';
      button.dataset.value = value;
      button.textContent = value === 'coffee' ? '☕' : value;
      button.setAttribute('aria-label', value);
      button.addEventListener('click', () => castVote(value));
      return button;
    }),
  );
}

function renderDeckSelection(revealed) {
  for (const button of ui.deck.children) {
    button.classList.toggle('is-selected', button.dataset.value === myVote);
    // Once cards are face up the round is settled; changing a vote then would
    // only invalidate the commitment everyone already checked.
    button.disabled = revealed;
  }
}

function renderRoom(state) {
  // A new round anywhere in the room clears our own selection.
  if (lastRound !== null && state.round !== lastRound) myVote = null;
  lastRound = state.round;

  ui.roundLabel.textContent = `Round ${state.round}`;
  ui.peerCount.textContent =
    state.participants.length === 1 ? '1 here' : `${state.participants.length} here`;
  ui.reveal.disabled = state.revealed;
  ui.reveal.textContent = state.revealed ? 'Revealed' : 'Reveal';

  ui.cards.replaceChildren(
    ...state.participants.map((p) => {
      const card = document.createElement('div');
      card.className = `seat seat-${p.state}${p.is_me ? ' is-me' : ''}`;

      const face = document.createElement('div');
      face.className = 'seat-face';
      if (p.state === 'revealed' || p.state === 'tampered') {
        face.textContent = p.value === 'coffee' ? '☕' : p.value ?? '–';
      } else if (p.state === 'hidden') {
        face.textContent = '✓';
      }

      const name = document.createElement('div');
      name.className = 'seat-name';
      name.textContent = p.name || p.short_id;
      name.title = p.id;

      card.append(face, name);

      if (p.state === 'tampered') {
        const flag = document.createElement('div');
        flag.className = 'seat-flag';
        flag.textContent = 'changed after committing';
        card.append(flag);
      }
      return card;
    }),
  );

  renderStats(state.stats);
  renderDeckSelection(state.revealed);
  // Peers arriving and leaving is exactly when the paths change.
  schedulePaths();
}

function renderStats(stats) {
  if (!stats) {
    ui.stats.hidden = true;
    return;
  }
  ui.stats.hidden = false;

  const figures = document.createElement('div');
  figures.className = 'figures';
  const add = (label, value) => {
    const figure = document.createElement('div');
    figure.className = 'figure';
    figure.innerHTML = `<span class="figure-value"></span><span class="figure-label"></span>`;
    figure.querySelector('.figure-value').textContent = value;
    figure.querySelector('.figure-label').textContent = label;
    figures.append(figure);
  };

  add('average', stats.average ?? '–');
  add('median', stats.median ?? '–');
  const answers = stats.tally.length;
  add('spread', stats.consensus ? 'agreed' : `${answers} answer${answers === 1 ? '' : 's'}`);

  const tally = document.createElement('div');
  tally.className = 'tally';
  for (const [value, count] of stats.tally) {
    const row = document.createElement('span');
    row.className = 'tally-item';
    row.textContent = `${value === 'coffee' ? '☕' : value} × ${count}`;
    tally.append(row);
  }

  ui.stats.replaceChildren(figures, tally);
}

// How each connection actually settled — the whole point of having two
// transports to compare.
//
// Coalesced: a burst of room updates should cost one round of getStats(), and
// a connection needs a moment after it forms before it has a nominated pair.
let pathsPending = false;

function schedulePaths() {
  if (pathsPending || !handle) return;
  pathsPending = true;
  setTimeout(() => {
    pathsPending = false;
    renderPaths();
  }, 400);
}

async function renderPaths() {
  if (!handle) return;
  const paths = await handle.paths();
  ui.paths.replaceChildren(
    ...paths.map((path) => {
      const chip = document.createElement('span');
      chip.className = `path path-${path.kind}`;
      chip.textContent = `${path.label}: ${path.kind}`;
      chip.title = path.detail;
      return chip;
    }),
  );
}

// --- actions ---------------------------------------------------------------

function castVote(value) {
  if (!handle) return;
  myVote = value;
  handle.vote(value);
  renderDeckSelection(false);
}

async function startSession(invite, name) {
  ui.start.disabled = true;
  ui.homeError.hidden = true;
  setStatus('connecting', 'Starting…');

  try {
    transport = (await import(TRANSPORTS[chosenTransport()])).default;
    renderDeck((await transport.load()).deck);

    handle = await transport.join({
      invite: invite || undefined,
      name,
      ...transportOptions(),
      onState: renderRoom,
      onStatus: setStatus,
    });
  } catch (err) {
    ui.start.disabled = false;
    setStatus('idle', 'Not connected');
    showError(String(err?.message ?? err));
    return;
  }

  const fragment = `#room=${handle.invite}`;
  ui.inviteUrl.value = `${location.origin}${location.pathname}${location.search}${fragment}`;
  ui.myId.textContent = `you are ${handle.id}`;
  ui.transportBadge.textContent = `${transport.label} — ${transport.tagline}`;
  // Reloading the page rejoins the same room rather than dropping to the form.
  history.replaceState(null, '', `${location.search}${fragment}`);

  ui.home.classList.remove('is-active');
  ui.room.classList.add('is-active');

  renderPaths();
  // A backstop: a connection can be renegotiated without the room changing.
  pathTimer = setInterval(renderPaths, 5000);
}

async function leaveRoom() {
  if (!handle) return;
  const leaving = handle;
  handle = null;
  myVote = null;
  lastRound = null;
  clearInterval(pathTimer);

  ui.room.classList.remove('is-active');
  ui.home.classList.add('is-active');
  ui.paths.replaceChildren();
  ui.start.disabled = false;
  setStatus('idle', 'Not connected');
  // Drop the room out of the URL so a reload does not rejoin it.
  history.replaceState(null, '', location.pathname + location.search);
  ui.invite.value = '';
  ui.start.textContent = 'Start a room';

  // The iroh session is consumed on the Rust side, so nothing may touch it after.
  await leaving.leave();
}

// --- wiring ----------------------------------------------------------------

function wireTransportPicker() {
  const current = chosenTransport();
  for (const button of ui.transportPicker.querySelectorAll('button')) {
    button.classList.toggle('is-selected', button.dataset.transport === current);
    button.addEventListener('click', () => {
      // The transport is a URL parameter so that it is carried into invites and
      // survives a reload; switching means reloading onto it.
      const q = query();
      q.set('transport', button.dataset.transport);
      location.search = q.toString();
    });
  }
}

function main() {
  wireTransportPicker();

  const prefilled = inviteFromLocation();
  if (prefilled) {
    ui.invite.value = prefilled;
    ui.start.textContent = 'Join room';
  }
  ui.name.value = localStorage.getItem(NAME_KEY) ?? '';
  ui.name.focus();

  ui.invite.addEventListener('input', () => {
    ui.start.textContent = ui.invite.value.trim() ? 'Join room' : 'Start a room';
  });

  ui.form.addEventListener('submit', (event) => {
    event.preventDefault();
    const name = ui.name.value.trim();
    if (!name) {
      showError('Pick a name so the room knows who you are.');
      return;
    }
    localStorage.setItem(NAME_KEY, name);
    startSession(parseInvite(ui.invite.value), name);
  });

  ui.leave.addEventListener('click', () => leaveRoom());
  ui.reveal.addEventListener('click', () => handle?.reveal());
  ui.reset.addEventListener('click', () => {
    myVote = null;
    handle?.reset();
  });

  ui.copy.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(ui.inviteUrl.value);
      ui.copy.textContent = 'Copied';
    } catch {
      // Clipboard access can be denied; selecting the text is the fallback.
      ui.inviteUrl.select();
      ui.copy.textContent = 'Press ⌘C';
    }
    setTimeout(() => (ui.copy.textContent = 'Copy'), 1500);
  });

  // Number keys pick a card, so a whole round can be run from the keyboard.
  document.addEventListener('keydown', (event) => {
    if (!handle || event.metaKey || event.ctrlKey) return;
    if (document.activeElement?.tagName === 'INPUT') return;
    const button = [...ui.deck.children].find((b) => b.dataset.value === event.key);
    if (button && !button.disabled) castVote(button.dataset.value);
  });
}

main();
