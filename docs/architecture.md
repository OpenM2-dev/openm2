# Architecture

openm2 is a game client and its own server, written from scratch and growing into an engine. This file is the agreed design. Why its main choices were made, and what was turned down, is under [Design choices](#design-choices). To change a choice, propose it first, then update this file in the same change.

## Goals
1. **A client and a server of its own,** speaking openm2's own protocol. The server serves the openm2 client only, and the client plays on openm2 servers only.
2. **Bring your own assets.** `openm2 assets import` reads the user's own TMP4 client files (EterPack archives, or folders they already unpacked) and converts them into an asset folder of openm2's own formats, on the user's machine. The client plays only from asset folders. The repo holds no Metin2 data.
3. **Easy for coding agents to maintain:**
   - small crates with one job each;
   - data-driven where possible;
   - fast deterministic tests;
   - one command (`just check`) that says whether a change is good.
4. **An engine of its own:** its own server, its own protocol, and open formats (glTF 2.0, PNG and openm2's own TOML) with a small original sample world, so openm2 runs and is tested without anyone else's server or files. The sample world is still to come (`docs/roadmap.md`).
5. **One license and one set of rules for everything in the repo:** client, server and content are MIT OR Apache-2.0, and all of them follow `docs/source-policy.md`.

## Shape: a sans-IO core with thin I/O edges

```
        bytes                messages              effects / events
TLS ──────────▶ m2-net ──────────────▶ m2-core ──────────────────────▶ m2-cli (bot, play)
    ◀──────────  (threads,  ◀──────────  (state    ◀── commands ───── m2-client (later:
        bytes     frames)     messages    machine)                      window, render, UI)
                     │                       │
                     └──── m2-protocol ──────┘   (messages, codec, frames)
```

- **m2-protocol** turns bytes into messages and back. Messages are declared in `packets/openm2.toml` and generated into Rust and docs (`just gen`). No I/O.
- **m2-core** is the client's brain:
  - `Session::step(now, input)` takes one input (start, connected, closed, a message, a command, a tick) and returns effects: connect to "the auth server" or "the game server", send a message, disconnect, report an event.
  - It owns the session stages, the game connection's phases and the world model (`World`): the characters, our character, the entities around us and where they are over time (tracks), points, items, chat.
  - It leads our own character's walks (`Command::MoveTo`, `Command::Stop`) and sends the move messages. The time comes in with every input, so walks replay exactly in tests too.
  - It never reads the clock or touches sockets, so every scenario can be replayed exactly in a test.
- **m2-net** runs the connections (`m2_net::run`):
  - each connection is TLS 1.3 with ALPN `openm2/1` (rustls), its handshake finished before the session's Hello; the server is trusted by a pinned key or the system's authorities (`m2_net::Trust`), and the game connection resumes the auth connection's TLS session;
  - one reader thread per connection passes the encrypted bytes through a bounded channel (if the session falls behind, the reader waits and TCP slows the server down);
  - the calling thread decrypts them, cuts them into frames (each its length, then the message), decodes them, steps the session as soon as a message arrives, writes what it sends, and ticks every 100 ms; it counts the bytes on the sockets (`Traffic`);
  - it maps "auth server"/"game server" to the configured host, records captures with secrets emptied, skips messages of ids it does not know, and turns a bad frame or a message that does not decode into a closed connection with a report;
  - a `Controller` (the bot, the game window) reacts to events with commands and may look at the session after each batch of inputs (`on_update`);
  - `m2_net::Net` hands out a `Remote`, so another thread (the window) can send commands while the session runs.
- **m2-cli** is the `openm2` tool: inspection commands for humans and agents (`messages`, `message decode`, `cap decode`, `pack`, `vfs`, `proto`, `gr2`, `dds`, `tga`, `jpeg`, `script`, `map`, `sound`), the importers (`assets import`, `content import`), the model, map, effect, tree and interface viewers (`view …`), the headless `bot`, `release` for update channels, and `play`: the servers, sign-in, character select and create, and loading screens, then the session on a network thread, the map and the actors drawn from a copy of its world every frame with the interface over them, their sounds and the music, and the player's input sent through the `Remote`. `play` lives in m2-cli until `m2-client` exists; its layouts and skin are in `crates/m2-cli/ui/`.

Game files take a separate path that never touches the network:

```
user's client folder ──▶ m2-assets ──────────────▶ m2-formats ──────────▶ m2-cli (pack, vfs, proto,
  pack/*.eix, *.epk       (files, VFS,              (parsers and writers,  assets import, content import)
  keys.toml                keys.toml, extraction)    no I/O)
```

- **m2-formats** parses game files from byte slices: EterPack indexes and entries, `MCOZ` containers (LZO1X, XTEA), the proto tables, GR2 models (Oodle1), DDS textures, TGA and JPEG images, WAV sounds, text scripts (model, motion and sound scripts, motion lists, environments, the monster list, sub-images, skill tables, spawn lists), effects and flying effects, maps (settings, area grids, objects, sound spots, texture sets, properties, the atlas), the models' attributes (collision shapes, floors). Every parser has a writer, used to build test data. No I/O. It also holds the model layer (`m2_formats::model`): skeletons, meshes with the textures they show, and animations as keys, as glTF has them. GR2 files are read into it (`Gr2File::model`, `Animation::to_animation`), and glTF files are read into it and written from it (`m2_formats::gltf`, `docs/specs/formats/gltf.md`).
- **m2-assets** does the file I/O: loads the user's `keys.toml`, opens archives, reads entries with positional reads, mounts loose folders and archives into one VFS, and extracts entries without ever writing outside the output folder. It also reads a map's files together (`m2_assets::map`), indexes the property files by id, finds the model scripts of player characters and monsters (`m2_assets::actor`), the image a sub-image cuts from (`sub_image_path`), the sound script of a motion, the music and the original client's volumes (`m2_assets::sound`). For asset folders it has their paths and references, reading and writing inside them, and the actor, motion, effect, flying effect, map and area files (`m2_assets::folder`, `actor_file`, `map_file`, `effect_file`, `fly_file`).
- **Open formats:** glTF 2.0 models, PNG images, and openm2's own TOML for actors, motions, maps, effects, the interface's skin and the item and skill tables, in an asset folder (`docs/specs/assets.md`). The glTF reader fills the same model layer as the GR2 reader, so `m2-render` doesn't know which format a model came from, and actor, motion, effect and map files convert to the same types as the client's scripts and maps, which the import writes them from. **Asset folders are the only way to play and to draw:** `play` and the `view` commands read nothing else (`source.rs` holds an asset folder, `view::Files` an asset folder or loose files). The client's files are read only by `assets import`, `content import` and the format tools, and the launcher imports a player's client into the install folder's `imported/`.
- **m2-ui** is the interface toolkit: windows from RON layout files, a skin file naming the interface's art, the windows' state, pointer and keyboard input in and events out, text shaped with cosmic-text into its own glyph atlas, and a draw list of quads out. No GPU, no window, no clock (`docs/specs/ui.md`).
- **m2-audio** plays sounds: WAV files (read by `m2-formats`) and MP3 files (nanomp3) decoded into frames, how loud and on which side a sound is heard from where it is, and the output through kira on the system's audio device: one-shot sounds, loops and streamed music, each with a volume. It owns no clock or window; its caller (`openm2 play`) says what plays when (`docs/specs/audio.md`).
- **m2-render** draws with wgpu: posing and skinning on the CPU side, one skinned pipeline, terrain and water pipelines, the interface's draw lists, effects played on the CPU with their own pipeline, the maps' skies, trees grown from openm2's own recipes, and offscreen targets read back as images. It takes models and animations of the model layer, texture data from `m2-formats` and draw lists from `m2-ui`, and owns no window; its caller passes a surface or draws offscreen.
- **m2-server-core** and **m2-server** are openm2's own server ([below](#our-own-server)).
- **m2-content** is openm2's own game content format: maps, monsters, items, skills, races, levels, drops, starting items, shops, packs, spawns and the operator's rule files as TOML, read, checked and written without I/O, and the reader of skills' formulas (`docs/specs/server/content.md`). The server reads it; `openm2 content import` writes the imported part from the operator's client. Maps' walk and water masks (PNG, a pixel a metre) and the rules for walking across them, straight lines and paths around walls, live here too, so the server and `play` follow the same ones.
- **m2-update** is updates without I/O: a channel's manifests (a version's files by path, size and SHA-256, a sequence that only goes up, an expiry), their Ed25519 signatures through aws-lc-rs, the checks a launcher makes before trusting one, the paths that can't leave an install folder, the plan of which files to reuse, and the requests that fetch the rest. Small files come in bundles, whole or by range (`docs/specs/update.md`).
- **m2-launcher** is `openm2-launcher`: it fetches a channel's signed manifest, files and bundles over HTTPS (reqwest on the workspace's rustls, 8 requests at once), builds the next version beside the one in use, checks every file, swaps the folders by renaming, replaces itself, imports the player's own client folder with the version's `openm2 assets import` into `imported/` when it isn't the game's import yet, then starts `openm2 play --assets` with it. Local saves are worlds in its `saves/`: it runs the version's `openm2-server` on one, the world imported from the same client with `openm2 content import`, then `play` on it. It does so in a window of its own, drawn with `m2-ui` and `m2-render` in flat colours and a game's own look, or on the console with `--no-window` and `--update-only`. `openm2 release` makes the keys and signed manifests.
- **Later crates:** `m2-client`, the winit app (until then `openm2 view …` and `play` own their windows, and the launcher its own). It is added when it has work to do, not as an empty stub.

### Allowed crate dependencies
Enforced by `just check-deps` (`ALLOWED_DEPS` and `ALLOWED_DEV_DEPS` in `xtask/src/checks.rs`):

| Crate | May depend on |
|---|---|
| m2-protocol | — |
| m2-core | m2-protocol |
| m2-net | m2-protocol, m2-core |
| m2-formats | — |
| m2-assets | m2-formats |
| m2-ui | — |
| m2-render | m2-formats, m2-ui |
| m2-audio | m2-formats |
| m2-cli | m2-protocol, m2-core, m2-net, m2-formats, m2-assets, m2-render, m2-ui, m2-audio, m2-content, m2-update |
| m2-content | — |
| m2-server-core | m2-protocol, m2-content |
| m2-server | m2-protocol, m2-server-core, m2-content |
| m2-update | — |
| m2-launcher | m2-update, m2-ui, m2-render |
| xtask | — |

Tests may use more (dev-dependencies): `m2-server`'s tests run `m2-core` and `m2-net` as real clients, and its binary the way `m2-launcher` hosts a local save, and `m2-cli`'s tests start `m2-server` (configured through `m2-server-core`) and import a client with `m2-launcher`'s functions, as the launcher does. Outside tests the client never depends on the server, nor the server on the client; they share only `m2-protocol` and `m2-content`.

## The protocol layer
- **One protocol, openm2's own** (`docs/specs/protocol/openm2/`), declared in `packets/openm2.toml`. openm2 owns it: a feature the client and the server share is a protocol change.
- **Messages:** a varint id, unique per direction, then the fields: integers little-endian, UTF-8 strings, blobs, lists and options each with a maximum checked before anything is allocated. One encoding per message: what decodes encodes back to the same bytes.
- **Frames** (`m2_protocol::wire`): a varint length, then the message. A frame over its direction's limit (8,192 bytes from the client, 4,096 from the server) is refused before it is buffered. An unknown id is skipped.
- **Versions:** the major one in TLS ALPN (`openm2/1`, `m2_protocol::tls`), the minor one in `Hello`. The server answers `Welcome` or `Refused`, which never change. Adding a message is not a new minor version: an older peer skips its id.
- **TLS 1.3** under every connection (rustls with aws-lc-rs): the operator's certificate checked against the system's authorities, or the server's self-signed one pinned by its key's SHA-256 (`m2_protocol::tls::KeyPin`).
- **Secret fields.** Fields marked `secret = true` (a ticket, a provider's code and PKCE verifier) get a generated `Debug` that prints `<redacted>`, and `redacted()` empties them so captures hold neither them nor their length.
- **Sign-in with a provider:** the client asks the auth server for its providers and a nonce, opens the system's browser for the authorization code flow with PKCE (`m2_net::signin` catches the redirect on 127.0.0.1), and relays the code. The server exchanges it and checks the ID token (`m2_server::oidc`), and `(iss, sub)` names the account. The client holds no token.
- **Sign-in with a device key**, where the operator allows it: the client signs the nonce and the connection's TLS channel binding (RFC 9266) with an Ed25519 key of its own (`m2_net::DeviceKey`), and the server checks the signature (`m2_server::key_sign_in`); `(openm2:key, the public key)` names the account. Both cores only pass the nonce and the signature on; the edges hold the TLS connection.
- **Text** is UTF-8 everywhere; control characters are dropped or shown as `?` where text is logged or drawn.

## Our own server
What it does: `docs/specs/server/`; how to run it: `docs/server.md`.

```
        bytes              messages                effects
TLS ──────────▶ m2-server ────────────▶ m2-server-core ─────────▶ m2-server
    ◀──────────  (tokio:    ◀──────────  (Server::step:  ── Store ──▶ (storage,
        bytes     listeners,  messages    connections,   ◀─ Stored ──  accounts)
                  frames)                 sign-ins,      ── CheckSignIn ──▶ (providers:
                                          world)         ◀─ SignInChecked ─ code, ID token)
```

- **m2-server-core** is the server's brain:
  - `Server::step(now, input)` takes one input (a connection accepted, a message, a connection closed, a store answer, a provider's sign-in checked, a tick, shutdown) and returns effects: send a message, close a connection, ask storage, check a sign-in, report an event.
  - It owns every connection's stage (hello, sign-in, the character list, loading, the game), the tickets, failed sign-ins and tickets per address, and the world: the players (who sees whom, moves, chat, whispers and friends, points, deaths and levels) and the content's monsters and NPCs (one vid table, a grid of cells as wide as the view radius, groups that sleep while no player is near, fights checked and worked out on the server, monsters that fight back, die and come back, players' items and items on the ground, NPCs' shops, teachers, stables and smiths, skills, mounts, fishing; `docs/specs/server/world.md`).
  - It never reads the clock, touches sockets or waits on storage: storage answers come back as inputs, the wall clock is given once at start (`Server::started_at`), and randomness comes from a ChaCha20 generator seeded by the edge (the world on a stream of its own). Its rules are those of `m2-core` (its `AGENTS.md`).
- **m2-server** is the edges and the `openm2-server` binary:
  - `start(config, store)` binds both listeners and runs on the caller's tokio runtime (`runtime()` makes one with two worker threads), with the world's content from `config.content` (`content::load_dir` reads a folder); tests start it on port 0;
  - one task owns the core, steps it on every input and every 100 ms, and carries out the effects;
  - `tls`: the certificate, the operator's or a self-signed one kept in the server's data folder; each connection's TLS handshake runs in a task of its own (10 s at most, a limited number at once) before the core hears of it;
  - each connection has a reader task (deframing with `m2-protocol`, at most 100 messages a second) and a writer task with a bounded queue, which writes everything queued at once; a client that does not read is closed;
  - `store`: the requests carried out in Postgres (sqlx, our own embedded migrations, TLS checked by default) or in memory for tests, by the same rules; every account is a provider's player's or a device key's, found by their identity (no passwords); or a save's folder, one JSON file;
  - `oidc`: the operator's OpenID Connect providers, discovered at start; each sign-in's code exchanged over HTTPS (reqwest with rustls) and its ID token checked (jsonwebtoken, RS256 and ES256 through aws-lc-rs), the keys fetched again when old or unknown;
  - `telemetry`: logs to stderr, and traces (a span per store request) and metrics through OpenTelemetry when `OTEL_EXPORTER_OTLP_ENDPOINT` is set;
  - deployed as a Docker image (`Dockerfile`, distroless and non-root) with Postgres beside it (`compose.yaml`); stopping on SIGTERM saves every character first. A tag publishes the image to GHCR and the Windows client to a GitHub Release, and one server at a time runs a database's world (an advisory lock).
- **The same shape as the client:** the core is deterministic and replayed in tests; networking, storage (a relational database) and telemetry (an open standard) are the edges.
- **The same protocol code:** the server uses `m2-protocol` the other way round, decoding what the client sends and encoding what the client reads.
- **The client and the server test each other:** the client's scripted sessions show what the server must answer, and the bot runs against it end to end in CI, with no outside server.
- **It serves the openm2 client only.** Its rules follow the protocol specs and, for the game, QCX's behaviour and other allowed sources; where they leave a gap, the rule is openm2's and the spec says so.
- **Sign-in through providers, or devices' keys:** players sign in through the OpenID Connect providers the operator lists, or, when the operator turns it on, with their device's own key. The server keeps no passwords and does not start without a way in.
- **Its data:** a content folder in openm2's own format (`docs/specs/server/content.md`): for the Metin2 maps, imported from the operator's own client and kept on the operator's machine, plus the operator's own rule files; original data for the sample world; made-up data in tests (`docs/source-policy.md`).

## Game files
Specs: `docs/specs/formats/`.

- **Parsers:**
  - Hand-written on a bounds-checked `Reader`, under the same crate lints as the protocol: no `unwrap`, `expect`, indexing or `panic!`.
  - Our own LZO1X decoder, XTEA and CRC-32.
  - Every size from a file is checked against the bytes left and capped before anything is allocated.
- **Writers for everything.** Tests build archives and proto tables with our writers and our own test keys, in memory or in a temp folder. Real files are only read by local tests that the user enables with `OPENM2_CLIENT_DIR`.
- **Keys:**
  - The user's keys come from `keys.toml` (`--keys` or `OPENM2_KEYS`).
  - `Keys` and `XteaKey` print only which keys are set.
  - A missing key gives an error naming the key to add.
- **Layouts are versioned.** A structure with a version or record size is matched against the layouts we know. An unknown one is an error that names the known ones, never a guess.
- **Strings stay bytes** (`RawStr`, often EUC-KR). They are printed escaped, and paths go through `safe_relative_path` before they touch the disk.
- **VFS:**
  - Loose folders override archives.
  - Archives are mounted in the order of the client's archive list (`pack/Index`), then by file name; the first archive holding a name wins.
  - Lookups normalize Ymir paths (lowercase, `/`).

## Rendering
- Plain **wgpu + winit**, not a game engine. The rules are in `docs/specs/render.md`.
- Metin2's look needs a few simple pipelines:
  - textured meshes with vertex lighting, fog and alpha test;
  - GPU-skinned characters (one pipeline; rigid meshes are skinned with one joint);
  - splatted terrain and water (tile textures blended on the GPU from a texture array, baked shadows, flat see-through water);
  - effects: particles and mesh effects played on the CPU, drawn with their own pipeline;
  - sprites and text for the interface (one pipeline for `m2-ui`'s draw lists, blended over the world; glyphs in atlas pages like any other texture).
- **Trees are grown, not read:** `m2_render::tree` grows them from RON recipes, seeded, into ordinary meshes for the model pipeline, with textures drawn by code. The client's SpeedTree files are never read.
- **Posing is CPU code** (`m2_render::pose`): sample the animation, build world and skinning matrices. Parts (hair, armour) follow the main skeleton by bone name.
- **Textures are decoded on the CPU** to RGBA8 and uploaded with their mipmaps, so any adapter works, software ones included.
- **Colour stays in gamma space**, like the original: no sRGB conversion.
- **Every view can render offscreen to a PNG** (`--screenshot`), so agents can look at the result. Tests draw synthetic scenes and fail without a graphics adapter; a software one is enough (WARP on Windows, Mesa's lavapipe on Linux, as CI uses).

## Audio
- **kira** on cpal, without its decoders. The rules are in `docs/specs/audio.md`.
- **Decoding is ours or pure Rust:** WAV by `m2_formats::wav`, MP3 by nanomp3; music is decoded while it plays.
- **Where a sound is heard from is plain code** (`m2_audio::spatial`): loudness by distance, the side from the listener's right, so it is tested without a device.
- **No device is not an error:** `play` goes on silent, and `--screenshot` never opens one. Tests run the output on kira's mock backend.

## Determinism and observability
- Core is deterministic; tests feed it messages and times and snapshot the result.
- `tracing` logs at the I/O edges (`RUST_LOG=m2_net=debug` shows every message's name and length; never contents).
- Session captures: `.m2cap`, JSON Lines, one record per message with secrets emptied when written (`m2_net::capture`); `openm2 cap decode` reads them back.
- A closed connection says why: a frame over its limit, a message that does not decode, the server's refusal.
- `openm2 bot --dump-state` prints the final world state (Rust debug format).
- `openm2 vfs check` decodes every file of a client and reports, per archive, what decoded, which CRCs matched and what failed; `openm2 proto dump` prints proto tables as JSON Lines.

## Security
- Bytes from the network and game files are untrusted (the server's from clients, the client's from servers):
  - parsers check every length against the bytes left and cap allocations;
  - parsers never panic, and property tests feed every parser random and damaged input;
  - fuzz targets are still to come (`docs/roadmap.md`).
- openm2's protocol runs under TLS 1.3 only, with ALPN `openm2/1` on both sides (`docs/specs/protocol/openm2/README.md`, "TLS").
  - The client checks the server's certificate against the system's authorities, or pins its key; with a pin it still checks the handshake's signature.
  - The server's private key is never logged; certificate and key files are read up to 1 MiB, and the key it makes is readable only by its owner (on Unix).
  - Tickets, providers' codes and PKCE verifiers live in `secrecy` types, are never logged, captured or stored, and never come from command-line arguments. There are no passwords. Tickets are 32 random bytes, single use, 30 s, compared in constant time.
  - A device's private key stays in its file (PKCS#8 PEM, made only where no file is, owner-only on Unix) and in `secrecy` types while read; only its public key and its signatures leave the client, and a signature covers one connection's channel binding and one nonce.
- The server checks everything a client sends (its stage, ranges, distances, what the player sees), refuses an unknown version with `Refused`, and throttles failed sign-ins per address.
- The client connects only to the servers the user configured or added. Server-supplied addresses need the user's opt-in.
- Text from the server is printed and drawn with control characters escaped or shown as `?`, so a server cannot send terminal escape sequences; the server logs a client's text the same way. Chat and whisper text is never logged.
- Reader threads hand bytes over through a bounded channel, so a fast server cannot make the client buffer without limit.
- Archive names never leave their folder (`m2_assets::safe_relative_path`):
  - drive letters are dropped;
  - absolute paths, `..`, control characters and Windows device names are refused;
  - extraction creates folders one level at a time, never writes through a link, and never overwrites a file.
- Game file keys belong to the user. They are never committed (`just no-assets`) or printed.
- Updates (`docs/specs/update.md`):
  - the launcher trusts a manifest's Ed25519 signature by the key in `launcher.toml`, not the server it came from; it refuses another channel's, an expired one and one older than the installed version;
  - every file's size and SHA-256 are checked before it's used, and a download never takes more than its size;
  - manifest paths are a narrow set of characters (no `..`, absolute paths, drive letters, `:` streams, device names or hidden files), so none leaves the install folder;
  - the private key comes only from `OPENM2_RELEASE_KEY`, is never printed, and `openm2 release keygen` writes it to a new file (owner-only on Unix);
  - channels never carry players' client files or keys: the launcher asks for their own, and imports them into the install folder's `imported/`, which updates never touch;
  - nothing the launcher logs or shows names the path of its channel's address.
- CI:
  - read-only token, actions pinned by commit SHA, no secrets;
  - `cargo-deny` checks advisories, licenses and sources.

## Design choices
Why openm2 is built the way it is, and the main alternative each choice turned down. Behaviour is in the specs; this section keeps the reasons. A new design choice adds an entry here in the same change.

### Sources and assets
- **Written from scratch; players bring their own client.** The repo holds no code, game files, proto dumps, archive keys or images made from real assets (`docs/source-policy.md`, `just no-assets`). Players point openm2 at their own client and keys, converted on their machine. *Turned down:* building on the leaked sources, as most community projects do, which no open-source project can ship. The current official client (`.ecpk` archives) is out of scope: its format is undocumented and many files need keys its servers send.
- **The project's logo is in `branding/`, outside the code's licence.** The README's banner, the launcher's window, the release's login screen (`openm2.toml`'s `logo`) and the setup's icon and pictures use it; `branding/README.md` says what each file is and that builds and forks others ship use their own name and logo. `just no-assets` lets `.png` files through there only. *Turned down:* the logo under MIT or Apache-2.0 (anyone could ship it as theirs), and fetching it from another site when a release is built (the release would depend on that site).
- **Test data is synthetic.** Every parser has a writer; tests build their data with it and openm2's own test keys. Tests that need a real client read `OPENM2_CLIENT_DIR` (or an import of it, `OPENM2_ASSETS_DIR`) and skip without it.

### Engine and rendering
- **Plain wgpu and winit, not Bevy.** Metin2's look is simple (textured meshes, vertex lighting, fog, alpha test, skinning, splatted terrain), and the game logic already lives in engine-free crates. *Turned down:* Bevy, whose large breaking releases every few months make agents write stale APIs, and whose PBR renderer and ECS would mostly go unused. Revisit for modern lighting, or if a stable, slow-moving engine appears.
- **A small renderer of our own.** One pipeline for every model (skinned with up to four joints; a rigid mesh is one joint at weight 1), textures decoded on the CPU to RGBA8 so every adapter works (WARP, llvmpipe, GLES), gamma-space colour, alpha test, no culling, wgpu's errors returned, not panicked. Every view renders offscreen to a PNG for tests and agents. *Turned down for now:* block-compressed texture upload (later, if whole maps need the memory) and a separate static pipeline.
- **Trees grown from openm2's recipes** (`crates/m2-cli/trees.ron`), picked by the words of a tree file's name, seeded, with textures drawn by code. The SpeedTree `.spt` files hold growth settings, not geometry, and only SpeedTree's library can grow them. *Turned down:* the composite billboards (flat up close and from above), our trees with the client's bark and leaves (they read `.spt` files), and copying SpeedTree's growth (guessing at code nobody can see).
- **An interface toolkit of our own, `m2-ui`.** Windows from RON layouts, looks from a skin file, a draw list out, no GPU or clock, so it is tested without fonts or an adapter. Text is shaped by cosmic-text into openm2's own glyph atlas, so glyphs and sprites share one draw list and one draw order. Fonts come from the system; openm2 ships none. *Turned down:* glyphon (its own render pass, so text and sprites can't interleave), and egui, iced or slint (large dependencies next to a toolkit the engine has). The original's window scripts are not read.
- **Audio through kira on cpal, without kira's decoders.** WAV is openm2's own reader; MP3 is nanomp3, a pure-Rust port of minimp3 (CC0). Decoding, the distance rules and what `play` schedules are plain code with tests; the output runs on kira's mock backend in tests. *Turned down:* kira's decoders (they bring symphonia, MPL-2.0, and formats the client doesn't use), and a mixer of our own on cpal.

### Core and determinism
- **Sans-IO cores.** `m2-core` and `m2-server-core` are state machines: inputs and the time (`Millis`) in, effects out. No sockets, threads or clock, and no hash-ordered iteration (their `clippy.toml` forbid them). `m2-net` and `m2-server` own the I/O. So scenario tests are plain function calls with snapshots, sessions replay exactly, and the bot and the game window share all their logic. The cost is a layer of effect and event types, kept in step by exhaustive matches.
- **Randomness and wall time come from the edge.** The server's core draws from a ChaCha20 generator the edge seeds (tests seed it with a fixed value), with the world on a stream of its own so monsters never change tickets. The wall clock comes in once, as `Server::started_at(epoch)`, for waits that must outlive a restart (skill training). *Turned down:* waits in the core's milliseconds, which a restart would reset.

### Protocol, TLS and sign-in
- **openm2's own protocol between its own client and server.** Varint frames capped per direction, little-endian fields, UTF-8 strings, lists and options that carry only what is there, unknown ids skipped. The major version is the ALPN name, the minor one is in `Hello`, and `Hello`, `Welcome` and `Refused` never change. After entering, only changed points are sent; everything queued goes out in one TLS record; the game connection resumes the auth connection's TLS session. *Turned down:* another server's plaintext, unversioned packets behind an adapter; Noise (a second crypto stack beside HTTPS's); QUIC (UDP that some networks handle badly, for what is one ordered stream).
- **Messages declared in TOML and generated.** `crates/m2-protocol/packets/openm2.toml` generates the structs, codecs, enums and `docs/specs/protocol/openm2/messages.md`; `just drift` fails if they are stale, and the generated Rust is plain code that compiler errors point into. Adding a message is a TOML entry plus a byte-exact vector. *Turned down:* hand-written codecs that drift from their docs, `macro_rules!` declarations nobody can read, and derive crates (binrw) that leave the docs separate.
- **TLS 1.3 only, through rustls with aws-lc-rs**, on both listeners. The operator's certificate is checked against the system's authorities, or the server's self-signed key is pinned by its SHA-256 (the handshake signature is still checked). The cost: a C compiler in every build (aws-lc-sys).
- **Sign-in only with OpenID Connect providers, the code relayed to the server.** The operator lists providers; the client runs the authorization code flow with PKCE (S256) in the system's browser and catches the redirect on 127.0.0.1; it relays the code, and the server exchanges it, checks the ID token (RS256 or ES256, the algorithm from the key, never `none` or HMAC; issuer, audience, expiry, the connection's nonce) and names the account by `(iss, sub)`. The client never holds a token; the server keeps none and no passwords, so it has none to leak, guess or reset. It has no built-in development provider (a back door the moment it's left on). *Turned down:* passwords beside providers; the client exchanging the code (it would hold a token another server could replay); linking accounts by e-mail; a nonce from the client; the device flow; the `openidconnect` crate (a larger tree for a few requests).
- **Devices' own keys as a second way in, off by default.** For a server on the player's own machine, or among friends, with no provider and no internet (`--key-sign-in`): the client makes an Ed25519 key once, and signs the connection's nonce and its TLS channel binding (RFC 9266's exporter, both sides of one connection get the same bytes) with a label of openm2's own; the server checks the signature at its edge and names the account by the key. It is no back door: the message names no account, so a key reaches its own account and nobody else's, a signature taken off the wire is worth nothing on another connection, and a game master is still only an account the operator names (`--game-masters`, or `--owner-key` for a key). What it allows is anyone who reaches the server making an account, which is why the operator chooses it. *Turned down:* passwords or a shared secret per save (something to leak, guess and type); TLS client certificates (custom verifiers on both sides, and every connection would carry one); signing the nonce alone (a server in the middle could pass it on); a built-in provider (a browser for nothing, and an account for whoever reaches it as anyone); the first account being the game master (a race anyone can win).
- **Servers by address, trusted on first use.** `play`'s login screen lists the server its settings name (openm2's Windows release names the project's, below) and the player's own servers (`servers.toml`, beside the settings file). An address may carry the server's key (`#sha256:…`) as a link its operator publishes; without one, a self-signed server's key is shown and asked about at the first join, kept, and a changed key is refused. No list is fetched from any server or URL. *Turned down:* the list in the launcher (changing server would mean quitting the game), links only (players couldn't type an address), and the list in `settings.toml` (an older build rewriting the settings would drop it).

### Server
- **tokio at the edges; the core stays synchronous.** One task owns the core and steps it on every input and every 100 ms; each connection has a reader task and a writer task with a bounded queue, and a client that falls behind is closed. *Turned down:* a thread per connection, as the client does; a server should hold thousands of mostly idle connections, and storage and telemetry are async anyway.
- **PostgreSQL through sqlx, SQL written by hand, our own migration runner.** Queries are parameterised SQL built at run time, so a build needs no database; numbered migrations are embedded and applied in one transaction under an advisory lock, and never changed once released. An in-memory store answers the same requests for tests and `serve --memory`, and a save's file for a server on the player's own machine (below). *Turned down:* tokio-postgres (lower level, no gain), diesel (a large DSL), sea-orm (an ORM), and SQLite (cheapest, but one process only).
- **A save is the in-memory store written to one JSON file** (`serve --save DIR`, `m2_server::store::FileStore`). A world on the player's own machine holds a few accounts, so the whole of it is written at every change: a temporary file, synced, renamed over the save, so the file is always a whole save. The memory store's rules are the save's (one `MemoryData::apply`), a file lock keeps a second server off it, and it is checked at start as a database's constraints would. *Turned down:* SQLite (a C library, and a third SQL dialect beside Postgres's queries), an embedded key-value store such as redb or sled (a format of its own for a few kilobytes), an embedded Postgres (a database server to ship and start), and writing only what changed (a log to replay and compact, for files this small).
- **TLS to the database, `verify-full` by default.** Without `sslmode` the server requires TLS and checks the database's certificate and name; `disable` is for a database beside it. A failed check stops the server. The authorities are webpki-roots (allowed for that crate alone; its licence text ships in the image), and `sslrootcert` adds a database's own. *Turned down:* libpq's `prefer`, which silently falls back to plain text; ring with the system's roots (a second crypto library); native-tls (OpenSSL in the image).
- **One server per database.** A running server holds a Postgres advisory lock, and a second waits until the first has stopped and saved everyone. *Turned down:* a renewed lease row: more code, and an advisory lock goes away with a crashed server's connection.
- **OpenTelemetry through `tracing`.** Logs always; traces and metrics over OTLP only when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Never tickets, codes or chat text. *Turned down:* logs alone, and Prometheus alone (metrics only, its own format).
- **A Docker image and a compose file.** A distroless, non-root image built with the `dist` profile, base images pinned by digest, configuration from the environment; `compose.yaml` adds Postgres and, as an option, a collector. The image never holds content.
- **Game masters are named by account in the server's settings** (`--game-masters`, or `--owner-key` for a device's key). Their commands are chat lines checked on the server, unknown to everyone else, and each one carried out is an event in the log. *Turned down:* a flag in the database (a migration, an admin command, and a way into a hosted database), accounts by number (a wiped database could make a stranger one), and a provider's claim (every provider names it differently).
- **Whispers are online only and never stored; their text is never logged.** A name is looked up among the players in the game only, and "nobody of that name in the game" and "no such character" get the same answer, so a whisper is no way to test which names exist. **Friendships** are one row per pair (`first_id < second_id`), so a half friendship can't exist. *Turned down:* stored whispers (mail), and a server `/w` command (the client reads its own chat line).
- **No conversation state.** Every answer to an offer (a teacher's skills, a soul stone's choice, an upgrade, a shop's question) is checked again from scratch when it comes. *Turned down:* remembering the offer: one more state to forget at every walk and close, for no added safety.

### Content and assets
- **openm2's own content format, imported from the operator's client.** The server reads only a content folder of TOML files (`docs/specs/server/content.md`), strictly checked with every problem named by file, line and column. `openm2 content import` writes the imported part from the operator's own client, marked so `just no-assets` refuses it; the operator's own files sit beside it and the import never writes them. *Turned down:* reading the client's tables at every start (ties the server to Metin2's layouts and its keys), content in Postgres (hard to write, review and version), RON or JSON (TOML has line numbers in errors and comments), and a starter world for the Metin2 maps in the repo.
- **Game rules the client doesn't carry are the operator's files,** with openm2's own defaults: drops and monsters' gold, shops, teachers, skill training, mounts, upgrades, fishing, packs for the client's spawn lists. Item kinds the client numbers (books, a forgetting book, poles, bait) are read by kind, as weapons are; what their values mean in the original is not assumed. *Turned down:* the original server's tables and numbers (forbidden), and rules hidden in imported files the next import would overwrite.
- **Walk and water masks as PNG in the content,** a pixel a square metre (`docs/specs/formats/map.md`, "Attributes"). Operators paint them in any image editor; the import writes them from the client's attributes. The walking rules (`m2-content`'s `walk`: whole-number lines, A* inside a capped box) are shared by the server, its monsters and `play`, so both sides always agree. *Turned down:* a binary format of our own, masks in TOML, the server reading the client's files, navigation meshes, and walls as polygons.
- **Open asset formats.** glTF 2.0 binary for models and animations, PNG for images, openm2's own TOML for actors, motions, maps, effects, the skin and the item and skill tables, WAV and MP3 for sound (`docs/specs/assets.md`). A neutral model layer in `m2-formats` takes GR2 and glTF alike, so the renderer doesn't know which a model came from. *Turned down:* the `gltf` crate (its own JSON model and errors outside our lints, and we need a writer anyway), FBX (proprietary), Collada (large XML), KTX2 now (an encoder and a C++ transcoder for what RGBA8 does), and Ogg (a new decoder).
- **What glTF can't say goes in openm2's extensions and `extras`,** used, never required, so other tools still open the files: shear in `OPENM2_scale_shear`, an animation's length and track groups, floors as nodes marked `openm2_floor`, mesh effects as a scene per frame with each mesh's visibility. Names that aren't UTF-8 are written with `%XX`. *Turned down:* dropping shear (many models would draw wrong), splitting sheared bones (exact only at the keys), and requiring the extension (other tools would refuse the files).
- **Asset files in the renderer's frame and in metres** (+Z up, forward −Y, as Blender shows a model), with words where the client chose by number (`blend = ["source_alpha", "one"]`). Maps are a folder per area with the client's grids kept (their borders included), the environment in the map file, and an image region written as a W3C media fragment (`ui/public.png#xywh=…`). *Turned down:* the client's numbers in TOML (an artist would need its numbering), whole-map rasters (to revisit with the sample world's tools), and Effekseer for effects (a C++ runtime and its own format).
- **Asset folders are the only way to play.** `play` and the `view` commands read nothing else; the client's files are read only by the importers and the format tools. One way at run time means what players play is what CI checks. *Turned down:* both ways for good (every feature built and tested twice), an import by hand before playing, and converting in memory at each start.
- **The import is versioned and deterministic.** `assets import` writes its number into `assets.toml`, raised whenever its output changes, and `assets current` answers from that file alone, so the launcher knows at once whether to import again. The output is the same byte for byte however many threads convert it (`std::thread::scope` and a shared queue). Every imported file is marked, and `just no-assets` and `release manifest` refuse them. *Turned down:* checking the whole import at every start, and rayon (a dependency for 40 lines).
- **The client's tables live in the asset folder, apart from the server's content.** Items, skills and monsters' names, with icons and models, are the asset folder's; the server's content keeps the same numbering. *Turned down:* the client reading the server's content, which would need a server's whole world and has no icons or models.
- **Layouts are openm2's own; art comes only through the skin's looks.** An asset folder's `skin.toml` replaces all of the built-in skin's art, and a missing look draws its fallback colour. *Turned down:* images named by openm2's own ids mapped per source (a game could change its art but not its looks).

### Updates and releases
- **A tag is a release.** `vX.Y.Z`, the workspace's version, publishes the server's image to GHCR (its digest in the notes) and the Windows client zip with its SHA-256 and third-party notices to a GitHub Release, each job with only the permissions it needs. The notes start with a changelog made from the Conventional Commits subjects since the previous tag (`cargo xtask changelog`). *Turned down:* cargo-dist (more than a zip and an image need), a release on every push (nothing to pin), a cloud's own registry, Docker's actions (three more actions to pin), GitHub's generated notes (they list pull request titles, which vary more than commit subjects), and a hand-kept `CHANGELOG.md` (one more step in every change).
- **Signed manifests and a launcher.** A channel publishes a manifest of a version's files by path, size and SHA-256, signed with Ed25519 (aws-lc-rs, already in the build); the signature is the first line of the manifest's own file, so one upload publishes both. Files are named by their SHA-256 and never change. The launcher trusts the key in `launcher.toml`, not the server, and checks every file. The install folder keeps `app/`, `previous/` and `staging/`, linking unchanged files and swapping by renames. *Turned down:* a detached signature (two uploads seen half-done), TUF (roles and canonical JSON for one key), minisign, updaters that replace one program from GitHub Releases, ureq (a second HTTP client), and binary deltas.
- **openm2's own release is a flat channel on GitHub.** `deploy/windows/launcher.toml` points the release's launcher at `releases/latest/download/`, where a release's downloads are named `<sha256>` and `windows-x64.manifest.signed` (`layout = "flat"`), signed with the engine's own key, a secret of the `release` environment, for a year. The zip works unzipped, and updates itself and the game from the next release. It carries the programs only; the player's client is imported on their machine. Redirects are followed only to HTTPS. *Turned down:* GitHub Pages (a second place to publish and a setting to turn on, for the same files), the API's release list (rate-limited, and a second way to update), and a zip whose launcher can't update (players would download every release by hand).
- **A setup for players, the zip beside it.** `openm2-setup.exe`, made with Inno Setup (already on GitHub's Windows image), installs for the player alone with no administrator, into a folder the launcher can update in place; its uninstaller takes what the launcher made too. It's tried on the runner (a silent install and uninstall) before it's published. *Turned down:* MSI with WiX (machine-wide, an administrator, and a folder the launcher couldn't write), MSIX (needs a code-signing certificate, and keeps the app's folder read-only), and the zip alone (players expect a setup).
- **Bundles.** Small files are published in bundles of up to 16 MiB, fetched whole or by range, each content once, 8 requests at once. A first install of tens of thousands of files asks for about a hundred objects. Offsets live in the signed manifest, so a bundle is just bytes. *Turned down:* one request per file even over HTTP/2, multipart ranges (object stores don't answer them), zip or tar bundles (more untrusted input), and compression (a dependency, and per-file ranges).
- **The launcher's window is drawn with the engine's own toolkit,** in flat colours with a game's own look (`launcher.toml`'s `[window]`, or the installed version's `window.toml`), from a Windows-subsystem release build; `--no-window` and `--update-only` keep the console's way. The ready screen offers the latest version; the previous one only after something failed. A Browse button beside each of the import's fields opens the system's own dialog on Windows, through `rfd` (Windows only, no backend features; its `unsafe` is inside the crate), on a thread of its own so the window goes on drawing; elsewhere the fields take typing, pasting and drops. *Turned down:* a console program, a whole native window (`unsafe`, Windows only), egui or iced, a picker of our own drawn in the window (a file browser to write), and paths typed or dropped only (players new to it look for a Browse button).
- **The launcher hosts a local save as a child process.** It starts the version's own `openm2-server` on the save's folder, on `127.0.0.1` with ports of the system's choosing, devices' keys the only way in and the player's key its game master; reads the ports and the key's pin from the server's ready line; starts `play` on it; and, when the game ends, closes the server's stdin, which saves everyone and stops it. A launcher that ends any other way closes it too. *Turned down:* the server inside the launcher (the launcher would depend on the server's crates, and a server's crash would take the window with it), stopping it with Ctrl+C (on Windows a console control event needs a console both share, and the launcher has none), waiting for `openm2 status` to answer instead of a ready line (fixed ports could be taken, and the key would still have to be read), and the server's own `content check` after the world's import (the import checks it against the format's rules before writing, and the server reads it again at start).
- **The Windows release pins the project's server.** Its `openm2.toml`, a file of the version, names `play.openm2.net` with its key, first on every install's Servers board; a move or a new key reaches installs with an update. A `--host` naming another server takes nothing of the file's server (a local save is not the project's server). *Turned down:* the server built into the program (every build and fork would list it, and a move would need a new build), a list of servers fetched from a site (a service to run and trust; the board's servers stay the ones the release and the player name), and no server of the project's own (a new player would find the board empty).
- **The Windows release carries the server.** `openm2-server.exe` is a file of the version like the game, so local saves need nothing else and update with it; its packages' licences join the release's notices. *Turned down:* a separate download for saves (a second thing to keep up to date), and saves only for players who run a server themselves.
- **The player's settings and servers live in the install folder,** found through `OPENM2_SETTINGS`. *Turned down:* `openm2.toml` (each version replaces it), `%APPDATA%` (apart from the install that uses it), and a flag (older versions started with `--previous` would refuse it).

### Game rules and looks
- **The server decides; the client shows.** Positions, hits, chances, waits and the fishing reaction are checked and rolled on the server, by its clock; the client only predicts what it will say. Looks come from the client's assets: the server sends numbers (which skills are on a player, which effect happened, what flew where, which items are worn), and the asset folder says how each looks. *Turned down:* looks named in the server's content (another client's assets couldn't choose their own), and the client guessing events from changed points.
- **Where the original is not known from allowed sources, openm2 decides and says so.** States' chances and durations, mounts' speeds, training's waits and similar numbers are openm2's own, written in the specs as such, until a better source is known. Raised points are not saved over a logout.
- **New features add messages, not fields.** Mounts, skills on players and similar come as messages sent after an entity's spawn rather than new fields in it, so the minor version stays and older peers keep working.

### Interface
- **The clipboard through arboard, text only.** Paste puts text on one line through the same checks as typing; nothing is copied from a secret field; the clipboard's text is never logged. *Turned down:* calling each system's clipboard ourselves (`unsafe`, three implementations), and other crates that use the same Windows crate and are less used.
- **One whisper window that switches conversations,** and a game menu whose Change character goes back to the list on the same connection (no browser sign-in again).

### Dependencies
- **Few, exact and settled.** Versions pinned exactly in the workspace, from crates.io only, at least two weeks old, upgraded one at a time. A new licence is a design choice; the per-crate exceptions are `triple_buffer` (MPL-2.0, which kira can't be built without), `webpki-roots` (CDLA-Permissive-2.0) and `clipboard-win` and `error-code` (BSL-1.0) (`deny.toml`).
- **Our own code where a crate would bring risk or weight:** hand-written parsers on a bounds-checked reader (not binrw or nom), our own LZO1X (the crates were GPL, C, or of unclear origin; `lzokay-native` is a test-only reference), XTEA and CRC-32, our own GR2 reader and Oodle1 decoder (written from liboodle's public-domain description, with the client's files winning where they disagree; Granny's own DLL is 32-bit Windows under RAD's licence), JPEG, the part of glTF openm2 draws, and the import's thread pool. Each is under the crate lints and has known-answer or differential tests.
