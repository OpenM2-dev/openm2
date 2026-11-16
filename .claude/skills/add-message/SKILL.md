---
name: add-message
description: Add or change a message of openm2's own protocol - TOML declaration, codegen, byte-exact vector test, client and server handling, specs. Use when asked to add, change, decode or support a message, a field, or a feature that needs the client and the server to say something new to each other.
---

# Add a message

openm2 owns its protocol: a feature the client and the server share is a protocol change. The declaration in `crates/m2-protocol/packets/openm2.toml` is the source of truth; code and the docs table are generated from it. Read `crates/m2-protocol/AGENTS.md` and `docs/specs/protocol/openm2/README.md` first.

## 1. Decide what it carries
- **Which way:** `c2s` or `s2c`; a message goes one way only. Two directions are two messages.
- **What:** only what the other side needs. Send what changed, not everything (`PointsChanged`, not every point); leave empty entries out of lists.
- **Game numbers** (classes, kinds, point numbers, wear positions, window numbers) keep the values of the client's game files and of openm2's content.
- **Limits:** every string, blob and list has a maximum. Keep the client's frames under `MAX_CLIENT_FRAME` (8,192 bytes) and the server's under `MAX_SERVER_FRAME` (4,096); `the_biggest_messages_fit_their_frame_limits` in the vectors checks the biggest.
- **Never** change `Hello` (client 1), `Welcome` (server 1) or `Refused` (server 2). Changing another message's layout, or giving an id a new meaning, is a new minor version (`PROTOCOL` in `crates/m2-protocol/src/profile/openm2/mod.rs`, and the server's oldest and newest versions in `m2-server-core`'s `hello.rs`); adding a message is not, as unknown ids are skipped.

## 2. Declare it
Add it after the last message of its direction, with the next free id:

```toml
[[message]]
name = "SkillGroup"
dir = "s2c"
id = 35
doc = "Our character's skill group."
fields = [
  { name = "group", type = "u8", doc = "1 or 2; 0 for none." },
]
```

- **Types:** `u8` to `u64`, `i8` to `i64`, `f32`, `bool`, `[T; N]` of fixed-size items, a `[[struct]]` declared above its first use, `string<N>` (UTF-8, at most N bytes), `blob<N>`, `list<T; N>`, `opt<T>`.
- **No fields:** leave `fields` out.
- **Tickets, codes, verifiers:** `secret = true` (numbers, strings and blobs only). Debug output shows `<redacted>` and captures empty the field.
- **Field names:** snake_case, not Rust keywords (`kind`, not `type`).

## 3. Generate
```sh
just gen
```
The generator checks the file (unique ids per direction, maxima that fit a u16, structs declared before use). Fix the TOML until it passes. Never edit `generated.rs` or `messages.md` by hand.

## 4. Byte-exact vector
In `crates/m2-protocol/tests/openm2_vectors.rs`, add a test with the frame exactly as on the wire, **its length included**, commented field by field, through `check_c2s(ClientMessage::X(..), &wire)` or `check_s2c(ServerMessage::X(..), &wire)`. They encode and frame the message and compare with the bytes, then deframe and decode the bytes and compare with the message. Add the message to the round-trip strategies further down. `just openm2 message decode --dir s2c <hex>` checks bytes by hand.

## 5. Handle it on both sides
Build: the compiler lists the exhaustive matches that need an arm.
- **Client:** `Session::on_message` (or a `Command` that sends it) in `crates/m2-core/src/session.rs`, with a scenario in `crates/m2-core/tests/scenarios.rs`.
- **Server:** `Server::message` in `crates/m2-server-core/src/server/mod.rs` and the module of its topic, with a scenario in `crates/m2-server-core/tests/`. Check everything the client sends on the server: its stage, ranges, distances, what the player sees.
- Snapshots change: `INSTA_UPDATE=always cargo test -p <crate>`, read the diff, explain it in the handoff.
- The bot and `play` (`crates/m2-cli`) and their scripted servers (`crates/m2-cli/tests/`) may need the new message too.

## 6. Specs
- When it is sent and what the other side does: `docs/specs/protocol/openm2/<topic>.md`.
- What the server does with it, and its rules: `docs/specs/server/session.md` or `world.md`.

## 7. Check and hand off
```sh
just check
```
Then update `docs/roadmap.md` (remove what is done, add open questions), stage by path, and propose a commit message (see the git rules in `AGENTS.md`).
