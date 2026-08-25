# Tests

## Room logic

```sh
cargo test --manifest-path wasm/Cargo.toml
```

Covers merging, the round lattice, vote commitments, message signing, presence
expiry and the statistics. Runs on the host target — no browser involved. The
room is shared by both transports, so this suite covers both.

## Browser session

`browser.mjs` drives three headless browsers through a full session: three
peers finding each other, hidden votes, a reveal from a peer that did not open
the room, a new round, a peer whose tab dies expiring off the table, and a
deliberate leave.

It runs against either transport and asserts what the connections settled on:
`direct` for WebRTC, `relay` for iroh. Both need a server they can reach; the
public ones work if your network lets them through, and running them locally
keeps the test self-contained:

```sh
cargo install iroh-relay --features server
iroh-relay --dev            # http://localhost:3340
npm run relay:ws            # ws://localhost:8081

./build.sh
./serve.sh                  # http://localhost:8080

npm install && npx playwright install chromium
TRANSPORT=iroh   node tests/browser.mjs
TRANSPORT=webrtc node tests/browser.mjs
```

`APP_URL`, `RELAY_URL`, `SIGNAL_URL` and `CHROMIUM_PATH` override the defaults.
