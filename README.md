# Planning Poker

Estimation sessions with no backend. Every participant's browser opens an
[iroh](https://www.iroh.computer) endpoint and joins a gossip topic; estimates
travel between peers over end-to-end encrypted QUIC. Nothing is persisted, and
the room ceases to exist when the last tab closes.

The whole thing is static files, so it is served from GitHub Pages.

## How it works

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

**Relays carry the traffic, not the meaning.** Browsers can't send UDP, so
iroh carries these connections over a relay via WebSocket. The relay forwards
encrypted QUIC packets and cannot read them. By default the app uses the relays
n0 operates; `?relay=<url>` points a room at a different one.

## Layout

```
public/          static site, deployed as-is to Pages
  index.html
  app.js         renders room state, forwards clicks
  styles.css
  pkg/           built by build.sh, gitignored
wasm/            the Rust crate compiled to WebAssembly
  src/protocol.rs  room state, merging, commitments, signing, statistics
  src/ticket.rs    invite encoding
  src/session.rs   iroh endpoint + gossip, the wasm_bindgen surface
tests/           browser test driving three peers through a session
```

`protocol.rs` and `ticket.rs` build on any target and carry the unit tests;
`session.rs` is browser-only.

## Developing

```sh
./build.sh          # compile the wasm module into public/pkg/
./serve.sh          # http://localhost:8080
cargo test --manifest-path wasm/Cargo.toml
```

There is also a browser test that drives three peers through a whole session —
see [tests/README.md](tests/README.md).

`build.sh` installs the `wasm-bindgen` CLI matching `wasm/Cargo.lock` if it is
missing. Opening `public/index.html` off disk will not work — ES modules and
WebAssembly need a real http origin.

### Running your own relay

Public relays are reachable from most networks, but not all. To run one:

```sh
cargo install iroh-relay --features server
iroh-relay --dev                                  # http://localhost:3340
./serve.sh
```

Then open `http://localhost:8080/?relay=http://localhost:3340`. The query
string is carried into the invite links you share, so the whole room agrees on
the relay. A room on its own relay also skips n0's address lookup — invites
carry full addresses, so nothing outside your network is contacted.

## Deploying

`.github/workflows/pages.yml` runs the tests, builds the wasm module and
publishes `public/`. It deploys on a push to the default branch, and on a push
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
