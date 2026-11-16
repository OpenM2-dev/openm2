# m2-protocol — rules for agents

openm2's protocol: messages, encoding and decoding, frames. **No I/O**: nothing here opens sockets, reads files at runtime, or spawns threads.

## Layout
- `packets/openm2.toml`: the message declarations, the **source of truth**. The format is described at the top of the file.
- `src/profile/openm2/generated.rs`: written by `just gen`, never by hand.
- `src/profile/openm2/mod.rs`: re-exports the generated code; the version (`PROTOCOL`) and the frame limits (`MAX_CLIENT_FRAME`, `MAX_SERVER_FRAME`).
- `src/codec.rs`: `Reader`/`Writer`, `Decode`/`Encode`, varints, strings, blobs, lists, options, errors.
- `src/message.rs`: the `Message` trait (`from_bytes` / `to_bytes`: a varint id, then the fields), `MessageSpec` (a row of the generated `CLIENT_MESSAGES`/`SERVER_MESSAGES` tables, with its secret fields) and `Direction`.
- `src/tls.rs`: what TLS needs from the protocol: `ALPN` (`openm2/1`, the major version) and `KeyPin`, a server key's pin (`sha256:` and 64 hex digits; parsed and printed here, hashed by the edges).
- `src/wire.rs`: the frame layer. `Deframer` cuts a stream into messages (a varint length, then the message) and refuses frames over its maximum; `wire::frame` frames a message.
- `tests/openm2_vectors.rs`: byte-exact vectors, one or more per message; tables, refusals, frames, round trips and random input.
- Specs: `docs/specs/protocol/openm2/` (README: encoding, frames, versions, size; the generated `messages.md`; behaviour by topic).

## Rules
- **A message** is a varint id (unique per direction), then its fields. Declare it as `[[message]] {name, dir, id, doc, fields}` in `packets/openm2.toml` and run `just gen` (the `add-message` skill).
  - The output is the message structs, `ClientMessage`/`ServerMessage` and the message tables, plus `docs/specs/protocol/openm2/messages.md`.
  - `Hello` (client 1), `Welcome` (server 1) and `Refused` (server 2) never change.
- **Field types:**
  - integers, `bool`, `[T; N]` of fixed-size items, structs;
  - `string<N>` (UTF-8, a u16 length, at most N bytes), `blob<N>`, `list<T; N>` (a u16 count) and `opt<T>`.
- **Decoding** checks every length and count against its maximum and the bytes left before allocating. It refuses overlong varints, tags other than 0/1, bad UTF-8 and leftover bytes, so what decodes encodes back to the same bytes.
- **Secrets** (numbers, strings, blobs): `secret = true`. `Debug` prints `<redacted>`, and `redacted()` empties them for captures.
- **No panics:** `unwrap`, `expect`, indexing and `panic!` are denied in `src/`. Return `DecodeError`/`EncodeError`/`WireError`. Tests may unwrap.
- **Every message gets a byte-exact vector** in `tests/openm2_vectors.rs`: the frame as on the wire, length included, with a comment per field, checked both ways with `check_c2s`/`check_s2c`. Round-trip proptests are a bonus.
- **Frame limits:** the biggest message of each direction must fit its limit (`the_biggest_messages_fit_their_frame_limits`).
- **Changing a layout** or the meaning of an id is a new minor version (`PROTOCOL`, and the server's versions in `m2-server-core`'s `hello.rs`); adding a message is not. A new major version is a new ALPN name (`tls::ALPN`), for a change `Hello` cannot settle.
- **Clippy `large_enum_variant`** is allowed on the generated message enums on purpose (messages are short-lived; boxing would make matching awkward).
