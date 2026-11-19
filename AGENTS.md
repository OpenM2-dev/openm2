# openm2 — instructions for coding agents

openm2 is an open-source game client and server, written from scratch in Rust. The client plays from asset folders in openm2's own formats, which `openm2 assets import` makes from the player's own Metin2 client on the player's machine; client and server speak openm2's own protocol. It is growing into an engine with open formats and an original sample world (`docs/roadmap.md`).

This file is shared by all coding agents; `CLAUDE.md` just imports it. `crates/m2-protocol/AGENTS.md`, `crates/m2-core/AGENTS.md`, `crates/m2-formats/AGENTS.md` (also covering `m2-assets`), `crates/m2-render/AGENTS.md`, `crates/m2-ui/AGENTS.md`, `crates/m2-audio/AGENTS.md`, `crates/m2-server-core/AGENTS.md` (also covering `m2-server`) and `crates/m2-content/AGENTS.md` add rules for those crates.

## Read first
1. `docs/roadmap.md`: what works, what's next, open questions.
2. `docs/architecture.md`: the agreed design, and in **Design choices** why it is so. Don't change a design choice silently: propose it, then update the Design choices in the same change.
3. `docs/source-policy.md`: what may and may not enter the repo. **Read it before any protocol or file-format work.**
4. `docs/glossary.md`: Metin2 terms (empire, vnum, proto, phase, EterPack, …). Don't guess them.

## Prerequisites
- [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml` and installs itself on first build.
- A C compiler, for the C code of the TLS library (aws-lc-sys): on Windows the Visual Studio Build Tools with the C++ workload (rustup's installer offers them; the crate brings prebuilt NASM objects), on Linux gcc or clang.
- A graphics adapter for the tests that draw; they fail without one. A software one is enough: Windows has WARP; on Linux without a GPU, Mesa's Vulkan drivers (`mesa-vulkan-drivers`, as CI installs).
- [just](https://github.com/casey/just) ≥ 1.44.
- `cargo-deny` (`cargo install --locked cargo-deny`). `just preflight` lists what's missing; ask the human to install it, don't install tools yourself.
- **Windows:**
  - Use `127.0.0.1`, not `localhost`.
  - Docker runs inside WSL (`wsl docker …`), only needed for the server's image (`docs/server.md`).
  - Builds are much faster with a Defender exclusion for `target/` or with the repo on a Dev Drive.
  - The linker is `rust-lld` (`.cargo/config.toml`).

## Commands
Run everything through `just` from the repo root. CI runs `just check`. Every `openm2`, `openm2-server` and `openm2-launcher` command lists its flags with `--help`; the docs named below say what they do.

| Command | What it does |
|---|---|
| `just check` | Everything CI checks: fmt, clippy, tests, drift, check-deps, no-assets, snapshots, notices, cargo-deny |
| `just quick` | fmt, clippy and tests for the crates you changed, plus drift. Use while iterating; run `just check` before handing off |
| `just fmt` / `just lint` | Format / clippy with warnings as errors |
| `just test` | All tests. `just test -p m2-protocol` passes args to `cargo test` |
| `just gen` | Regenerate message code and docs from `crates/m2-protocol/packets/*.toml` |
| `just drift` | Fail if generated files are out of date |
| `just check-deps` | Enforce the crate dependency direction and `publish = false` |
| `just no-assets` | Fail if game files, archive keys or large binaries are in the tree |
| `just deny` | cargo-deny: advisories, licenses, bans, sources |
| `just notices` | The third-party notices the Windows release ships, into `target/THIRD-PARTY-NOTICES.txt`; `just check` writes them too, so a package without a licence text fails there, not at the release |
| `just changelog [TAG]` | A release's changelog as Markdown: the commit subjects since the previous `v*` tag, grouped into Features, Fixes and Other changes (`--from REF` for another start); the release's notes start with it |
| `just preflight` | Report missing tools |
| `just openm2 <args>` | Run the CLI, e.g. `just openm2 messages --dir s2c` |
| `just openm2 message decode --dir s2c <hex>` | Split hex bytes into frames and decode each message |
| `just openm2 cap decode <file>` | Print a `.m2cap` capture, one message per line |
| `just openm2 bot --host 127.0.0.1 --provider ID` | The headless client on openm2's server: signs in with provider `ID` (prints the address to open), then `--create`, `--say`, `--walk`, `--fight`, `--shop`, `--skill`, `--whisper`, `--befriend`, … and `--capture local/x.m2cap` (secrets blanked). The server's key from `OPENM2_SERVER_KEY` or `--server-key` |
| `just openm2 key public` | This device's public key (`ed25519:…`), made in `device-key.pem` beside `OPENM2_SETTINGS` or `--device-key FILE` when there is none; `bot`/`play --provider key` sign in with it on a server started with `--key-sign-in` (`--owner-key` makes it a game master) |
| `just openm2 status --host H --server-key sha256:…` | Whether a server is up, without signing in; exit code 0 when auth and game answer. `bot`, `play` and `status` read missing settings from `openm2.toml` beside the program or `--config FILE` (`docs/specs/client.md`) |
| `just openm2 release manifest\|sign\|verify\|keygen …` | A channel's signed update manifest, its objects and bundles (`docs/specs/update.md`). The private key only from `OPENM2_RELEASE_KEY`; imported files are refused |
| `just launcher --dir DIR` | Run `openm2-launcher` on an install folder with a `launcher.toml`: updates `DIR/app` from the channel, imports the player's client into `DIR/imported` when needed, then starts `openm2 play` (`docs/specs/update.md`); its Singleplayer runs `app/openm2-server` on a save in `DIR/saves` and plays there (Multiplayer is `play` as before). `--screenshot local/x.png --show STAGE` draws its window offscreen (`saves`, `new-save`, `hosting`, `save-failed` among the stages). Tests serve a channel on `127.0.0.1` (`crates/m2-launcher/tests/update.rs`) |
| `just server serve --memory --providers local/providers.toml` | Run openm2's own server: auth on 11002 and game on 13001 at 127.0.0.1 under TLS 1.3, sign-in through the providers' file (secrets from `OPENM2_PROVIDER_<ID>_SECRET`) or, with `--key-sign-in`, devices' keys (`--owner-key` the owner's), the world from `--content DIR`, game masters from `--game-masters`; `--save DIR` keeps the world in one file there, `--memory` in memory, else Postgres at `OPENM2_DATABASE_URL` (`just server migrate`); `--ready-line` and `--until-stdin-closes` for a program that runs it. Everything in `docs/server.md`. Ask the human before starting it, Postgres or Docker for them |
| `just openm2 content import --client <dir> --out local/content` | openm2's content files (maps with walk masks, monsters, items, skills, spawns) from the human's client into a folder of theirs; the human writes the rest beside them (`docs/specs/server/content.md`); `content current <dir>` says whether a folder holds this openm2's import. Imported files are their data: never commit them |
| `just openm2 assets import --client <dir> --out local/assets` | An asset folder in openm2's own formats (glTF, PNG, TOML, sounds) from the human's client (`docs/specs/assets.md`). Every file is marked as imported: never commit them. `assets check <dir>` reads a folder back; `assets current <dir>` says whether it holds this openm2's import |
| `just openm2 pack ls <file.eix>` | List an archive's entries (`pack cat`, `pack extract` too) |
| `just openm2 vfs check --client <dir>` | Decode every file of a client, check CRCs, parse the proto tables (`vfs ls`, `vfs cat` too) |
| `just client-check <dir>` | The same, plus every format's `check`, in a release build with `local/keys.toml`: run by the human on their client |
| `just openm2 proto dump <file>` | Print `item_proto`/`mob_proto` as JSON Lines |
| `just openm2 gr2 model <file>` | What a renderer gets from a GR2 file (`gr2 info`, `gr2 dump`, `gr2 check --client <dir>` too) |
| `just openm2 dds info <file>` | A texture's format and levels (`dds png`, `dds check --client <dir>` too); `tga` and `jpeg` likewise |
| `just openm2 script dump <file>` | A text script (`.msm`, `.msa`, …) one value per line (`script check --client <dir>`, `--census`; `script motion <file.msa>`) |
| `just openm2 map info <map> --client <dir>` | What a map is made of, with its atlas points (`map ls`, `map check`; `map attr` draws its attribute flags) |
| `just openm2 view model <gr2\|glb> --anim <gr2\|glb> --screenshot local/x.png` | Draw a posed model into a PNG, or in a window without `--screenshot`; with `--assets <dir>` an actor file (`--shape`, `--hair`, `--motion`). Use the `verify-visually` skill |
| `just openm2 view map <map> --assets <dir> --screenshot local/x.png` | Draw a map of an asset folder: terrain, water, buildings, trees, effects (`--at X,Y --distance M --pitch 90`, `--floors`, `--no-effects`); a window with WASD without `--screenshot` |
| `just openm2 view tree <recipe> --screenshot local/x.png` | Grow trees from openm2's recipes (`crates/m2-cli/trees.ron`), no client needed (`--season`, `--variants`, `--seed`) |
| `just openm2 view effect <x.effect.toml> --assets <dir> --screenshot local/x.png` | Play an effect file and draw it at `--time S`, or `--frames N --duration S` in a grid; a `.fly.toml` flies at a target |
| `just openm2 sound info <file> --client <dir>` | A sound's kind, rate, channels, length and peak (`sound wav`, `sound play`, `sound check`). Agents cannot hear: check numbers, and ask the human to listen |
| `just openm2 view ui [layout.ron] --assets <dir> --screenshot local/x.png` | Draw interface layouts, or one of `play`'s screens with `--screen login\|select\|create\|loading\|game`, filled in with `--set`, `--value`, `--show`, `--item`, `--skill`, … and scaled with `--ui-scale` (`docs/specs/ui.md`). Use the `verify-visually` skill |
| `just openm2 play --host 127.0.0.1 --assets <dir>` | Play in a window on openm2's server: the Servers board, sign-in with a provider, character select and create, then the game (`docs/specs/ui.md` for every window and key, `docs/specs/client.md` for settings and servers). Needs the server's key (`OPENM2_SERVER_KEY`). `--screenshot local/x.png --provider ID [--screen …] [--say …] [--walk …] [--fight] …` draws one picture and logs out |

Game file commands read the user's keys from `--keys <file>` or `OPENM2_KEYS` (`docs/specs/formats/keys.md`). Real client files exist only on the human's machine; agents work with the synthetic data the tests build.

## Folder map
```
crates/
  m2-protocol/   openm2's protocol: messages, encoding/decoding, frames, the version, the ALPN name and server key pins. No I/O.
    packets/       openm2.toml: the message declarations (source of truth)
    src/profile/   the protocol's module; generated.rs is written by `just gen`
  m2-core/       deterministic client state machine: messages/commands in, effects out.
                 No I/O, no clock, no threads. Scenario snapshots in tests/snapshots/
  m2-net/        blocking TLS 1.3 transport (rustls) that drives m2-core (a Remote sends commands from another thread); pinned keys or the system's authorities; providers' sign-ins (PKCE, the browser's redirect on 127.0.0.1); .m2cap captures
  m2-formats/    game file parsers and writers: EterPack, MCOZ/LZO1X/XTEA, proto tables, GR2/Oodle1, DDS, WAV, text scripts, the monster list, the skill descriptions and table, maps and their atlas points, model attributes (collision shapes, floors), effects, sound scripts and spots; the model layer every model format is read into, and glTF 2.0 (`.glb`) read into it and written from it. No I/O.
  m2-assets/     the user's client on disk: keys.toml, archives, VFS, safe extraction, texture lookup, maps and properties, the areas' and atlas pictures, atlas points, the model scripts of characters and monsters, the locale and its items and skills, sound scripts, music, metin2.cfg's volumes; asset folders (openm2's own formats: paths and references, actor, motion, map and area files, the skin and images, item and skill tables, the import's markers)
  m2-ui/         interface toolkit: RON layouts and skins, widget state, item slots, input, text (cosmic-text, own glyph atlas), draw lists. No GPU
  m2-render/     wgpu renderer: poses, skinned model pipeline, terrain and water, the sky, trees grown from recipes (with textures drawn by code), effects (played on the CPU, their own pipeline), the interface pipeline, offscreen targets, PNG. Owns no window
  m2-audio/      sound: WAV and MP3 decoded into frames, loudness and side by distance, kira output (sounds, loops, streamed music). Owns no clock
  m2-cli/        the `openm2` binary: inspection tools, game file and model tools, the headless bot, the game window (`play`)
    ui/            the skin file and layouts of openm2's windows (RON)
    trees.ron      the built-in tree recipes and the words of tree files' names that pick them
    tests/         end-to-end tests against a scripted server (tests/support/) and synthetic clients; own_server.rs runs the bot on m2-server
  m2-content/    openm2's own game content format: maps, monsters, items, skills, races, levels, drops, starting items, shops, packs and spawns as TOML files, walk masks as PNG; parsing, rules and writing; skills' formulas; straight walks and paths across walk masks. No I/O
  m2-server-core/ openm2's own server as a deterministic state machine: connections' messages, the time and storage answers in; messages, storage requests and events out; the world's monsters and NPCs, fights, deaths and levels, items and the ground, shops, skills, whispers and friends. No I/O, no clock
  m2-server/     the server's edges: tokio listeners, TLS 1.3 (its certificate, the handshakes) and connection tasks, OpenID Connect providers (discovery, the code exchange, ID tokens' checks), storage (Postgres, or memory for tests; no passwords), OpenTelemetry, the content folder from disk, the `openm2-server` binary
    src/store/migrations/  the database's migrations, numbered SQL files (never changed once released)
  m2-update/     updates without I/O: a channel's manifests, their Ed25519 signatures, paths that can't leave the install folder, the checks, the plan of what to reuse and the requests that fetch the rest; bundles of small files
  m2-launcher/   `openm2-launcher`: a channel's signed manifest, files and bundles over HTTPS, 8 requests at once, the install folder's versions (app/, previous/, staging/) and their swap, the launcher's own update, the player's client folder and keys, the client's import into `imported/`, then `openm2 play`; local saves (`saves.rs`, `host.rs`: `openm2-server` run on one, the world imported into `content/`); its window (`src/window/`, `ui/`: the layout and the flat-colour skin)
branding/        the project's logo (the README's banner, the launcher's, the setup's icon and pictures), not under the code's licence; made outside the repo, never edited here (its README)
Dockerfile, compose.yaml, deploy/   the server's image, the server with Postgres (and a collector for telemetry; an empty content folder; licenses/ the terms the image carries)
xtask/           codegen and the checks behind `just` (Rust, not shell scripts)
docs/
  roadmap.md     what works, what's next, open questions
  architecture.md the design and its Design choices
  source-policy.md, glossary.md, server.md (running openm2's own server)
  specs/protocol/openm2/  the protocol: encoding, frames, versions (README), behaviour specs, generated messages.md
  specs/formats/  game file layouts (archives, containers, proto tables, keys.toml, GR2, DDS, TGA, scripts, maps, effects, sounds) and glTF, openm2's own
  specs/client.md the client's settings file (openm2.toml), the player's servers and `openm2 status`
  specs/update.md signed manifests, the channel's layout, `openm2 release` and what the launcher does
  specs/assets.md asset folders: paths, images, assets.toml, actor and motion files, effect files, maps (map and area files, the areas' rasters), what `play` needs (skin, items, skills, effects), `play --assets`, `openm2 assets import`, `check` and `current`
  specs/render.md how models are posed, textured, lit and framed; how maps, effects and the interface are drawn
  specs/audio.md  what plays when, how loud and from where; the music
  specs/ui.md     interface layouts, skins, input, text and the in-game windows
  specs/server/   what openm2's own server does (session.md, world.md), stores (storage.md) and reads (content.md)
.claude/skills/  recipes for repeated tasks: add-message, capture-session, add-format-parser, verify-visually, add-ui-window
```

## Conventions
- **Dependencies point one way:** `m2-protocol` → `m2-core` → `m2-net` → `m2-cli`, `m2-formats` → `m2-assets` → `m2-cli`, `m2-formats`, `m2-ui` → `m2-render` → `m2-cli`, `m2-formats` → `m2-audio` → `m2-cli`, `m2-protocol` → `m2-server-core` → `m2-server`, `m2-content` → `m2-server-core`, `m2-server` and `m2-cli` (the import), `m2-update` → `m2-launcher` and `m2-cli` (`release`), and `m2-ui`, `m2-render` → `m2-launcher` (its window). Tests may use more (`ALLOWED_DEV_DEPS`): the server's tests run the client's session and host a save as the launcher does, the CLI's tests start the server. `just check-deps` enforces it. A new crate goes into `ALLOWED_DEPS` in `xtask/src/checks.rs` and into `docs/architecture.md`.
- **Messages are data.** Declare them in `crates/m2-protocol/packets/openm2.toml` and run `just gen`. Never edit `generated.rs` or `docs/specs/protocol/openm2/messages.md` by hand; `just drift` fails otherwise.
- **Messages are keyed by direction and id.** The same id means different things client→server and server→client. `Hello`, `Welcome` and `Refused` never change.
- **Bytes from the network or a game file are untrusted** (the server's from clients, the client's from servers).
  - Parsers return errors and never panic: `unwrap`, `expect`, indexing and `panic!` are denied in `m2-protocol`, `m2-core`, `m2-formats`, `m2-assets`, `m2-render`, `m2-ui`, `m2-audio`, `m2-content`, `m2-server-core` and `m2-update`.
  - Check lengths and counts against the bytes actually left, and cap sizes before allocating.
  - Names from game files become paths on disk only through `m2_assets::safe_relative_path`.
- **Game files:**
  - Every parser has a writer.
  - Tests build their data with the writers and our own test keys; no binary fixtures.
  - Tests that need the human's real client read `OPENM2_CLIENT_DIR` (`m2-assets`' `tests/real_client.rs`), or `OPENM2_ASSETS_DIR` for an import of it (`play`'s, `m2-cli`'s), and skip when it is unset.
- **Core is deterministic.**
  - Time comes in as `Millis`; nothing in `m2-core` or `m2-server-core` reads a clock, sleeps, or iterates a `HashMap` (their `clippy.toml` forbid them).
  - Match on message enums without a `_ =>` arm, so the compiler lists every place a new message needs handling.
- **Tests:**
  - Every message gets at least one **byte-exact vector**: the whole frame as bytes, its length included. Round-trip property tests are extra, not a replacement.
  - Tests that open sockets bind `127.0.0.1:0`, never `0.0.0.0` or a fixed port.
  - Snapshot (insta) changes must be reviewed. Leftover `.snap.new` files fail `just check`. Say in the handoff why a snapshot diff is expected. To write new or changed snapshots: `INSTA_UPDATE=always cargo test -p <crate>`, then read the `git diff` of the `.snap` files.
  - A protocol bug seen on a running server: record a capture (`capture-session` skill) and turn the failing part into a test before fixing it.
- **Secrets:**
  - Tickets, providers' codes and PKCE verifiers use `secrecy` types without `Debug`; there are no player passwords (players sign in with providers, or with a device's own key where the server allows it); generated messages print their `secret` fields as `<redacted>`. Providers' client secrets come from the environment only; ID tokens are never kept, logged or sent to the client.
  - They are never written to logs, captures or disk, and never taken from command-line arguments.
  - A device's private key (`device-key.pem`, `m2_net::DeviceKey`) is never printed, logged or captured; only its public key (`openm2 key public`) and its signatures leave it.
  - Archive and proto keys belong to the user (`keys.toml`): never commit, print or log them. `XteaKey` and `Keys` print only which keys are set.
  - A channel's private release key comes only from `OPENM2_RELEASE_KEY`; `SigningKey` prints only its public key. Update channels never carry players' client files or keys.
- **Dependencies:**
  - Exact versions in `[workspace.dependencies]`, crates.io only.
  - A new license in `deny.toml` is a design choice: record it in `docs/architecture.md`'s Design choices.
  - Upgrade one dependency at a time.
- **Docs follow code:** a behaviour change updates the matching spec in `docs/specs/` in the same change.
- **Releases:** a tag `vX.Y.Z`, the workspace's version, runs `.github/workflows/release.yml`, which publishes the server's image to GHCR and the Windows client to a GitHub Release, whose notes start with the changelog since the previous tag (`just changelog`). The human raises the version and pushes the tag; agents never tag.

## Recipe: add a message
Use the `add-message` skill (`.claude/skills/add-message/SKILL.md`). In short:
1. Decide what it carries and which way it goes; openm2 owns the protocol, so a new feature is a protocol change. Game numbers keep the values of the client's files.
2. Add a `[[message]]` to `crates/m2-protocol/packets/openm2.toml` with the next free id of its direction.
3. Run `just gen`.
4. Add a byte-exact vector test in `crates/m2-protocol/tests/openm2_vectors.rs`.
5. Handle it in `m2-core` and `m2-server-core` (the compiler points at the matches) and describe the behaviour in `docs/specs/protocol/openm2/` and `docs/specs/server/`.
6. Run `just check`.

## Recipe: add a file format
Use the `add-format-parser` skill (`.claude/skills/add-format-parser/SKILL.md`). In short: facts and their source → spec in `docs/specs/formats/` → reader **and** writer in `m2-formats` → a byte-exact test written field by field → property tests for random and damaged input → wiring in `m2-assets`/`openm2` → `just check`. Ask the human to run it against their client.

## Never do
- Commit, push, or rewrite git history (see Git rules below).
- Copy code, structs, header lists or comments from leaked Metin2 sources or tools derived from them. Facts only, in your own words (`docs/source-policy.md`).
- Add game files, archive keys, dumped proto data, content or assets imported from a client (`openm2 content import` and `assets import` mark them), or screenshots of real game assets to the repo. Write screenshots to `local/` (gitignored). `just no-assets` catches the obvious cases, picture files included; the rule covers all of them. The one folder of pictures is `branding/`, the project's logo, which the human adds (`branding/README.md`).
- Connect to official servers, or write code meant for them or for any server but openm2's own.
- Put Metin2 data into our own server, its database seeds or its tests, or make it accept the original client (`docs/source-policy.md`, "Our own server").
- Hand-edit generated files.
- Install tools, run `cargo publish`, or add git dependencies.

## Git rules (mandatory)
The human owns the git history. **By default, agents never commit.**

- **By default, never run** the commands below. Run one only when the human explicitly asks for that specific action. The permission covers that request only, not the rest of the session.
  - `git commit` (including `--amend`)
  - `git push`
  - `git merge`, `git pull`, `git rebase`
  - `git cherry-pick`, `git revert`
  - `git tag`
  - `git reset`
- **Work on the right branch.**
  - Check `git branch --show-current` before changing any file.
  - Never make changes on `main`.
  - If the task has no branch yet, create one from the latest `main`:
    ```
    git switch -c <type>/<short-desc> main
    ```
  - Branch names are `<type>/<short-kebab-desc>`, where type is `feat`, `fix`, `chore`, `docs`, `refactor` or `test`. Example: `feat/headless-login`.
  - If you're unsure which branch to use, ask.
- **Stage only what belongs to the task.**
  - Use `git add <paths>`. Never run a blind `git add -A` or `git add .`.
  - Never stage build output, captures, keys, game files, or changes unrelated to the task.
  - To unstage a file: `git restore --staged <path>`.
- **Finish with a handoff, not a commit:**
  1. Show `git status --short` and a short summary of what is staged.
  2. Propose a commit message in Conventional Commits format:
     ```
     type(scope): short summary (max 72 chars)

     - what changed and why, in plain words
     ```
     The summary line goes into the next release's changelog as it is (`feat` under Features, `fix` under Fixes, the rest under Other changes; `docs`, `test`, `ci`, `style` and `chore(release)` are left out), so write it for someone reading the release.
  3. List anything you deliberately left unstaged.

  The human reviews, commits and pushes.

## Roadmap
When you finish a task, update `docs/roadmap.md` and stage it with the task changes: remove what is now done (and add it to "What works" when it is something a player or operator sees), and add any new open questions.
