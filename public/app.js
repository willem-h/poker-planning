// UI layer. All networking lives in the wasm module: this file starts a
// session, renders whatever room state it hands back, and forwards clicks.

import init, { Session, deck } from './pkg/poker.js';

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
  inviteUrl: el('invite-url'),
  copy: el('btn-copy'),
  leave: el('btn-leave'),
  peerCount: el('peer-count'),
  roundLabel: el('round-label'),
  reveal: el('btn-reveal'),
  reset: el('btn-reset'),
  cards: el('cards'),
  stats: el('stats'),
  deck: el('deck'),
  myId: el('my-id'),
};

let session = null;
let myVote = null;
let lastRound = null;

const NAME_KEY = 'poker.name';

function inviteFromLocation() {
  const match = window.location.hash.match(/room=([^&]+)/);
  return match ? decodeURIComponent(match[1]) : '';
}

// Optional `?relay=` override, for self-hosted relays or networks that block
// the public ones. Carried into invites so the whole room agrees on it.
function relayFromLocation() {
  const fromQuery = new URLSearchParams(location.search).get('relay');
  const fromHash = window.location.hash.match(/relay=([^&]+)/);
  return fromQuery || (fromHash ? decodeURIComponent(fromHash[1]) : '') || undefined;
}

function setStatus(kind, text) {
  ui.status.className = `status status-${kind}`;
  ui.statusText.textContent = text;
}

function showError(message) {
  ui.homeError.textContent = message;
  ui.homeError.hidden = false;
}

// --- rendering -------------------------------------------------------------

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

// --- actions ---------------------------------------------------------------

function castVote(value) {
  if (!session) return;
  myVote = value;
  session.vote(value);
  renderDeckSelection(false);
}

async function startSession(invite, name) {
  ui.start.disabled = true;
  ui.homeError.hidden = true;
  setStatus('connecting', 'Starting…');

  try {
    session = await Session.join(
      invite || undefined,
      name,
      relayFromLocation(),
      (json) => renderRoom(JSON.parse(json)),
      (kind, detail) => setStatus(kind, detail),
    );
  } catch (err) {
    ui.start.disabled = false;
    setStatus('idle', 'Not connected');
    showError(String(err?.message ?? err));
    return;
  }

  const fragment = `#room=${session.invite}`;
  ui.inviteUrl.value = `${location.origin}${location.pathname}${location.search}${fragment}`;
  ui.myId.textContent = `you are ${session.endpointId}`;
  // Reloading the page rejoins the same room rather than dropping to the form.
  history.replaceState(null, '', `${location.search}${fragment}`);

  ui.home.classList.remove('is-active');
  ui.room.classList.add('is-active');
}

async function leaveRoom() {
  if (!session) return;
  const leaving = session;
  session = null;
  myVote = null;
  lastRound = null;

  ui.room.classList.remove('is-active');
  ui.home.classList.add('is-active');
  ui.start.disabled = false;
  setStatus('idle', 'Not connected');
  // Drop the room out of the URL so a reload does not rejoin it.
  history.replaceState(null, '', location.pathname + location.search);
  ui.invite.value = '';
  ui.start.textContent = 'Start a room';

  // `leave` consumes the session on the Rust side, so nothing may touch it after.
  await leaving.leave();
}

// --- wiring ----------------------------------------------------------------

async function main() {
  await init();
  renderDeck(deck());

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
    startSession(ui.invite.value.trim(), name);
  });

  ui.leave.addEventListener('click', () => leaveRoom());

  ui.reveal.addEventListener('click', () => session?.reveal());
  ui.reset.addEventListener('click', () => {
    myVote = null;
    session?.reset();
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
    if (!session || event.metaKey || event.ctrlKey) return;
    if (document.activeElement?.tagName === 'INPUT') return;
    const button = [...ui.deck.children].find((b) => b.dataset.value === event.key);
    if (button && !button.disabled) castVote(button.dataset.value);
  });
}

main().catch((err) => {
  setStatus('error', 'Failed to load');
  showError(`Could not start the app: ${err}`);
});
