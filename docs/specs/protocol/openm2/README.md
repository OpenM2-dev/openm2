# `openm2` protocol specs

openm2's own protocol, between its client and its server. The client and the server speak only this.

Every connection runs under TLS 1.3 ("TLS" below).

- [`messages.md`](messages.md): the message tables and layouts, **generated** from `crates/m2-protocol/packets/openm2.toml` by `just gen`. Don't edit it by hand.
- Behaviour (when each message is sent and what the other side does):
  - [`session.md`](session.md): versions, logins, the heartbeat, characters, entering the game.
  - [`chat.md`](chat.md), [`entities.md`](entities.md) (the entities around us and our points), [`movement.md`](movement.md).
  - [`inventory.md`](inventory.md), [`combat.md`](combat.md), [`ground-items.md`](ground-items.md), [`shops.md`](shops.md), [`skills.md`](skills.md), [`friends.md`](friends.md), [`mounts.md`](mounts.md), [`upgrades.md`](upgrades.md), [`fishing.md`](fishing.md).
- What the server does with each message, and its rules: `docs/specs/server/`.

## Encoding
- **A message** is its id, then its fields in the order the TOML lists them.
- **Ids** are LEB128 varints: seven bits a byte, lowest first, the top bit set on every byte but the last. Every id so far takes one byte, and ids are per direction.
- **Integers** are little-endian, at their width.
- **Strings** (`string<N>`) are a u16 byte length, then at most N bytes of UTF-8. Invalid UTF-8 is an error. N counts bytes, so a name of 24 letters fits `string<64>` in any script.
- **Blobs** (`blob<N>`) are a u16 length, then at most N bytes.
- **Lists** (`list<T; N>`) are a u16 count, then at most N items. A list carries only what is there: points that are 0, skills not learned and empty shop places are left out.
- **Options** (`opt<T>`) are a 0, or a 1 and then the value. **Booleans** are a 0 or a 1.
- There are no sequence bytes and no fixed-width strings.

**One encoding per message.** Decoding refuses anything a writer would not produce:
- varints in more bytes than they need;
- tags other than 0 and 1;
- lengths or counts over their maximum;
- bytes left over.

So every message that decodes encodes back to the same bytes (`tests/openm2_vectors.rs` checks it on random input).

## Frames
On a connection each message travels in a frame: its length as a varint, then the message.
- **Limits:** a client's frames are at most 8,192 bytes, a server's at most 4,096 (`MAX_CLIENT_FRAME`, `MAX_SERVER_FRAME`). The biggest messages take 4,313 (a provider's sign-in with a code of 4,096 bytes; providers' codes reach about 4,000) and 1,788 (every skill learned); a chat line takes 516.
- The reader refuses a frame longer than its limit as soon as the length arrives, before buffering any of it, and refuses an empty one. After a bad frame, or a message that does not decode, the connection is closed.
- **An unknown message id is not an error:** the frame's length is known, so the reader skips the message and goes on. This lets a newer peer add messages without breaking an older one, once versions allow both to talk.
- The server writes everything queued for a connection in one write.

## TLS
Every connection runs under TLS 1.3 from its first byte (rustls with aws-lc-rs); the frames travel inside TLS records.
- **TLS 1.3 only,** post-quantum key exchange first (X25519MLKEM768, then X25519). No early data (0-RTT).
- **ALPN `openm2/1`,** the major version (`m2_protocol::tls::ALPN`), is required on both sides:
  - the server closes a connection whose handshake ends without it, and a client that offers only other names fails the handshake;
  - the client leaves a server that does not choose it.
- **The server's certificate:**
  - **the operator's,** for a name, from an authority: the client checks it against the system's trusted authorities and the name it was given (`--host`);
  - **or self-signed,** made by the server (`docs/server.md`, "TLS"): the client pins its key with `--server-key` (`OPENM2_SERVER_KEY`). A pin is `sha256:` and 64 hex digits, the SHA-256 of the key's SubjectPublicKeyInfo in DER (`m2_protocol::tls::KeyPin`).
  - With a pin the client checks the key and nothing else on the certificate: no names, no dates. The handshake's signature is checked as always, so only the holder of the key gets through.
- **The handshake comes first:** the client finishes it before it sends Hello, and the server hands a connection to its session only once it is done (`server/session.md`, "Connections").
- **Resumption:**
  - the server gives one session ticket per connection (stateless, its keys rotated by rustls);
  - the client keeps them in memory for the session, so the game connection resumes the auth connection's TLS session: the server's certificate is not sent again.
- **Records:** the server writes everything queued for a connection at once, so a burst of messages takes one record (up to 16 KiB of messages). The client writes a record per message. A record adds 22 bytes.

## Versions
- **The major version** is the TLS ALPN name, `openm2/1` ("TLS"): a client of another major version cannot finish the TLS handshake with the server.
- **`Hello`** is the client's first message on each connection: the minor version it speaks (`PROTOCOL`), and its build for the logs.
- **The minor version** is 3, the only one the server speaks. Client id 3 and server id 5 are reserved: no message uses them, so an older peer's message is never read as another.
- **The server answers** `Welcome` (its own minor version and build) or `Refused` (`session.md`, "Versions"). After `Refused` it closes the connection.
- **`Hello` (client 1), `Welcome` (server 1) and `Refused` (server 2) keep their ids and layouts in every version,** so any two versions can tell each other they do not match.

## Logins
- **Two connections:** on the auth server a provider's sign-in takes `StartSignIn`, `SignInOptions` and `ProviderLogin`, and is answered with `SignedIn`, the account's name and a 32-byte ticket (`session.md`, "Signing in with a provider"); `TicketLogin` presents the ticket on the game server (`session.md`, "Logins"). There is no password login.
- **Secrets:** tickets, providers' codes and PKCE verifiers are marked `secret`. `Debug` prints `<redacted>`, and `redacted()` empties them before a message is written to a capture, so not even their length shows.

## Size
The protocol sends what changed and no more:
- after entering, `PointsChanged` carries only the points that changed;
- `Points`, `Skills`, `CharacterList` and `ShopOpened` leave empty entries out;
- strings take their own length.

**Measured on the bot's whole run** (`the_bot_does_everything_on_our_own_server` in `crates/m2-cli/tests/own_server.rs`: sign in with the tests' provider, create a character, wear a sword, choose a skill group, spend a point, shop, fight, use a skill, drop and pick up, 10 s in the game):

| | Sent | Received | Both ways |
|---|---|---|---|
| openm2's messages, a provider's sign-in | about 410 bytes | about 1,500 bytes | about 1,900 bytes |
| openm2 on the sockets, TLS included | about 4,450 bytes | about 5,700 bytes | about 10,150 bytes |

- Points take 194 bytes in all (one `Points`, ten `PointsChanged`), skill levels 15, the shop 28, the empty character list 4. The provider's sign-in carries the PKCE verifier, the redirect and the code out, the providers and the nonce back (the tests' provider gives short codes; real ones are tens to thousands of bytes).
- The numbers move by a few dozen bytes from run to run (how many swings a fight takes, pings).
- A health change is 9 bytes on the wire (`points_changed` in the vectors).
- **TLS** adds its two handshakes, most of the 8 kB it costs: the post-quantum key shares take more than a kilobyte each way per handshake, and the auth connection's handshake carries the certificate. The game connection's handshake resumes the session. Then each record adds 22 bytes. The test checks that the game connection resumed and that the run stays under 16,000 bytes on the sockets; the bot prints its `traffic:` line.
