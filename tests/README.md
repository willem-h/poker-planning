# Tests

## Room logic

```sh
cargo test --manifest-path wasm/Cargo.toml
```

Covers merging, the round lattice, vote commitments, message signing, presence
expiry and the statistics. Runs on the host target — no browser involved.

## Browser session

`browser.mjs` drives three headless browsers through a full session: three
peers finding each other, hidden votes, a reveal from a peer that did not open
the room, a new round, a peer whose tab dies expiring off the table, and a
deliberate leave.

It needs a relay it can reach. The public ones work if your network lets them
through; running one locally keeps the test self-contained:

```sh
cargo install iroh-relay --features server
iroh-relay --dev            # http://localhost:3340

./build.sh
./serve.sh                  # http://localhost:8080

npm install -D playwright && npx playwright install chromium
node tests/browser.mjs
```

`APP_URL`, `RELAY_URL` and `CHROMIUM_PATH` override the defaults.
