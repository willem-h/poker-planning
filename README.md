# Planning Poker

Estimation sessions with no backend. Nothing is persisted, and the room ceases
to exist when the last tab closes. The whole thing is static files, so it is
served from GitHub Pages.

It ships **two transports for the same room**, switchable on the landing page
or with `?transport=`:

| | Path between peers | Servers involved | Module size (gzipped) |
|---|---|---|---|
| **iroh** | always relayed | an iroh relay, for the whole session | 1.5 MB |
| **WebRTC** | direct, TURN as fallback | a signaling relay, until peers connect | 136 KB + 59 KB |

Both run identical room code — the same merging, the same vote commitments, the
same signatures. Only the wire differs, so the two can be compared directly. The
room shows what each connection actually settled on: `direct` or `relay`.

## Why two

A browser cannot open a UDP socket, so it cannot hole punch, so
[iroh runs relay-only there](https://docs.iroh.computer/about/faq) — the relay
stays on the path for the whole session. It cannot read anything (the QUIC
inside is end-to-end encrypted), but it is there.

WebRTC is the only browser API that hole punches. Peers meet over a signaling
relay, connect directly, and drop it. But direct is best-effort: where a NAT
refuses to be traversed — commonly 10–30% of connections, far more behind
corporate firewalls — WebRTC falls back to a TURN relay, which you have to
supply (`?turn=`). Without one, those peers simply fail to connect.

So neither is unconditionally better. iroh trades a permanent relay hop for
connecting every time and needing nothing from you; WebRTC trades a fallback you
must provide for a direct path most of the time.

## How the room works

Everything below is transport-independent — it is the same Rust either way.

**No host.** Earlier versions elected one browser as the host and funnelled
everything through it, which meant the session died with that tab and needed a
manual copy-paste handshake per participant. Now every peer holds the whole
room and broadcasts only its own slice of it. Merging is last-writer-wins per
peer, so peers converge regardless of message order, and a peer leaving is a
non-event for everyone else. Anyone can reveal or start a new round.

**Rounds advance monotonically.** A round is a `(number, revealed)` pair
ordered so that revealing always beats hiding and a higher number always wins.
Any peer may advance it and everyone lands on the same value without a round of
agreement first.

**Votes stay hidden until reveal.** With no host to withhold them, a peer that
broadcast its raw estimate up front could be read by anyone with the developer
tools open. So while a round is open peers publish only `blake3(value:nonce)`.
On reveal each peer publishes its value and nonce, and every other peer checks
them against the commitment it already holds. A vote changed after committing
is shown as such and left out of the statistics.

**Messages carry their author.** Gossip spreads by flooding, so the peer that
hands you a message is usually not the peer that wrote it. Every announcement
is signed by its author and verified on arrival, so a peer relaying for others
cannot rewrite their votes on the way through.

**Presence is a heartbeat.** Peers re-announce every five seconds and are
dropped after 45 seconds of silence, so a closed tab clears itself from the
table without anyone having to say goodbye. The window is wide because browsers
throttle timers in hidden tabs; when gossip reports a peer unreachable it is
narrowed to a few seconds, which is what makes a closed tab disappear promptly
without evicting someone who just switched away.

## Layout

```
public/
  index.html
  app.js             renders room state, forwards clicks; transport-agnostic
  styles.css
  transports/
    iroh.js          wraps the iroh session
    webrtc.js        wraps Trystero + the transport-free room
  pkg/               iroh build          ⎫
  pkg-webrtc/        WebRTC build        ⎬ built by build.sh, gitignored
  vendor/            bundled Trystero    ⎭
wasm/
  src/protocol.rs    room state, merging, commitments, signing, statistics
  src/ticket.rs      invite encoding (iroh only)
  src/session.rs     iroh endpoint + gossip
  src/room_api.rs    the room with no transport, driven from JS
tests/               browser test driving three peers through a session
scripts/             vendor bundling, local signaling relay
```

One crate, two cargo features. `protocol.rs` builds on any target and carries
the unit tests; `session.rs` and `room_api.rs` are browser-only and mutually
exclusive.

A transport module is small: it moves opaque bytes and reports peers coming and
going. Anything speaking that shape can carry a room.

## Developing

```sh
./build.sh          # both wasm modules + the bundled signaling client
./serve.sh          # http://localhost:8080
cargo test --manifest-path wasm/Cargo.toml
```

There is also a browser test that drives three peers through a whole session on
either transport — see [tests/README.md](tests/README.md).

`build.sh` installs the `wasm-bindgen` CLI matching `wasm/Cargo.lock` if it is
missing, and needs npm for the Trystero bundle. Opening `public/index.html` off
disk will not work — ES modules and WebAssembly need a real http origin.

### Query parameters

| | |
|---|---|
| `?transport=iroh\|webrtc` | which wire to use |
| `?relay=<url>` | iroh: use this relay instead of n0's |
| `?signal=<ws url>` | WebRTC: signal here instead of over public Nostr relays |
| `?turn=<url>` | WebRTC: fallback for peers whose NAT will not be traversed (`turnUser`, `turnPass` alongside) |

All of them are carried into the invite links you share, so the whole room
agrees.

### Running your own servers

Neither transport needs one by default. If your network won't reach the public
relays, or you want a room that touches nothing outside it:

```sh
cargo install iroh-relay --features server
iroh-relay --dev                     # iroh:   http://localhost:3340
npm run relay:ws                     # WebRTC: ws://localhost:8081
```

Then `?relay=http://localhost:3340` or `?signal=ws://localhost:8081`. A room on
its own iroh relay also skips n0's address lookup — invites carry full
addresses, so nothing outside your network is contacted.

## Deploying

`.github/workflows/pages.yml` runs the tests, builds both wasm modules and the
signaling bundle, and publishes `public/`. It deploys on a push to the default branch, and on a push
to any branch with an open pull request, so a change can be looked at before it
is merged.

Two one-time settings:

- **Settings → Pages → Source → GitHub Actions**
- **Settings → Environments → `github-pages` → Deployment branches → All
  branches**, otherwise anything but the default branch is refused with
  *"not allowed to deploy to github-pages due to environment protection rules"*.

A repository has only one Pages site, so **whichever branch deployed last is the
one that is live** — a deploy from a pull request replaces what was there. The
run's summary says which branch it published. Deploys are serialised, and a
branch stops deploying once its pull request closes.

Any static host works — there is no server side to deploy.
