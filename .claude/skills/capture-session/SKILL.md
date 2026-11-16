---
name: capture-session
description: Record and read an openm2 .m2cap capture of a bot or play session against openm2's own server, to check the protocol specs, investigate a bad message or a dropped connection, or turn real traffic into a test. Use when asked to capture, record, replay, debug a session, or verify the protocol against the server.
---

# Capture a session

A capture (`.m2cap`) records every message of a session as JSON Lines: one record per connect, message and close. Secret fields (tickets, providers' codes, PKCE verifiers) are emptied as the file is written, so neither they nor their length show. Format: `crates/m2-net/src/capture.rs`.

## 1. Server
openm2's own server, run locally (`docs/server.md`): never an official server. If none is running, ask the human to start one; don't start servers, Postgres or Docker for them without asking. Tests start the server themselves on `127.0.0.1:0` (`crates/m2-cli/tests/own_server.rs`), and a test can keep its capture.

## 2. Record
```sh
# both sign in with one of the server's providers: the human signs in in the browser
# OPENM2_SERVER_KEY is the server's key (its log, or `just server tls key --data <its folder>`)
just openm2 bot --host 127.0.0.1 --provider <id> --say "hi" --stay 10 --capture local/<name>.m2cap
just openm2 play --host 127.0.0.1 --provider <id> --assets <dir> --capture local/<name>.m2cap
```
- Captures go under `local/` (ignored by git). `*.m2cap` outside `fixtures/` folders is ignored too.
- `RUST_LOG=m2_net=debug` adds a log line per message (name and length).
- A failing session still leaves a readable capture: every record is flushed as it is written.

## 3. Read
```sh
just openm2 cap decode local/<name>.m2cap
```
One line per record: time in ms, connection, direction, the decoded message (secret fields show `<redacted>`) and which fields were emptied. A message that does not decode is kept only as far as is safe: whole when its message has no secrets, else only its id (`only its id was kept`); an unknown id keeps only the id.

## 4. Investigate
- **The connection closed:** the close record's reason says why (a frame over the limit, a message that does not decode, the server's refusal). On the server side, its log says which rule closed it (`docs/specs/server/session.md`).
- **A message that does not decode:** compare the bytes with `docs/specs/protocol/openm2/messages.md`; both sides are generated from the same TOML, so a mismatch means two builds of different versions, or a bug in a hand-written path. Fix it with the `add-message` skill.
- **Bytes on the wire:** a message's frame is its length (a varint) then the message; add back the emptied secrets (`own_server.rs`'s `wire_bytes` does). A capture holds the messages, not TLS: the bot's `traffic:` line gives the bytes on the sockets, handshakes and records included.
- **The TLS handshake failed:** the capture has no record of that connection; the session's error says why (another key than the pinned one, no `openm2/1`, a timeout). The server logs failed handshakes at debug (`RUST_LOG=m2_server=debug`).

## 5. Keep it as a test
A capture that shows something new (an order of messages, a refusal) becomes a test before the fix:
- Short sequences: copy the messages into a vector test (`crates/m2-protocol/tests/`), a scenario of the client (`crates/m2-core/tests/scenarios.rs`) or of the server (`crates/m2-server-core/tests/`).
- A whole session: script it with the scripted server of `crates/m2-cli/tests/support/` (send these frames, expect those).
- Before committing anything taken from a capture, check it holds no secrets: `grep -o '"redacted":\[[^]]*\]'` shows which fields were emptied. Account and character names are in it too, so use test accounts only.

## 6. Record what you learned
Compare with `docs/specs/protocol/openm2/` and `docs/specs/server/`. Update the spec, and record open questions in `docs/roadmap.md`.
