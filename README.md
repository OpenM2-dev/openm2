# openm2

<p align="center"><img src="branding/openm2-banner.png" alt="openm2" width="640"></p>

An open-source game client and server, written from scratch in Rust and built to be maintained mostly by coding agents. The client plays from the files of a **Metin2** client you already have, converted on your machine into openm2's own formats; client and server speak a protocol of their own. It is growing into an engine with open formats of its own.

> **Status: early.** What works today:
> - **Playing:** `openm2 play` signs in with an OpenID Connect provider, shows your characters, and plays the map around you: walking, fighting with weapon combos and skills, items, shops, upgrades, mounts, fishing, warps, chat, whispers and friends.
> - **Your own client's files:** archives, item and monster tables, models, textures, maps, effects and sounds are read, and `openm2 assets import` converts them into an asset folder of open formats (glTF, PNG, TOML) on your machine.
> - **Viewers:** `openm2 view` draws characters, maps, trees, effects and interface windows, in a window or into a PNG.
> - **The server:** `openm2-server` runs the world from a content folder in openm2's own format: monsters and NPCs, fights, levels, drops, shops, skills, mounts, upgrades, fishing and game masters, with Postgres and a Docker image.
> - **The protocol:** openm2's own, versioned, under TLS 1.3. Players sign in only through providers the server lists; the server keeps no passwords.
> - **Releases:** the Windows client ships with `openm2-launcher`, which keeps it up to date from signed manifests.
>
> What comes next, and what is still unknown: [docs/roadmap.md](docs/roadmap.md).

## What it is
- A new client written from scratch. It contains **no code, game files or data from the original game**. See [docs/source-policy.md](docs/source-policy.md).
- It talks to its own server, `openm2-server`, in a protocol of its own ([docs/architecture.md](docs/architecture.md#design-choices)).
- **Bring your own assets:** openm2 converts the models, maps, textures and sounds of a game client you already have (a TMP4 40250 client) into an asset folder of its own formats, on your machine, and plays from that. Nothing is downloaded or redistributed.
- **Next, an engine:** open formats with a small original sample world, so openm2 runs without any Metin2 files ([docs/roadmap.md](docs/roadmap.md)).

## What it is not
- Not for official servers. Connecting a third-party client to them breaks their terms of service.
- Not a source of game files, archive keys or leaked code.
- Not a server for the original client, nor a client for other servers: openm2's client and server speak only to each other.

## Playing
On Windows:
1. **Download and run [`openm2-setup.exe`](https://github.com/OpenM2-dev/openm2/releases/latest/download/openm2-setup.exe)**, the [latest release](https://github.com/OpenM2-dev/openm2/releases/latest)'s, also on [openm2.net](https://openm2.net). It installs openm2 for you alone, without an administrator, into `%LOCALAPPDATA%\Programs\openm2`, with a Start menu shortcut and an uninstaller. Or unzip [`openm2-windows-x64.zip`](https://github.com/OpenM2-dev/openm2/releases/latest/download/openm2-windows-x64.zip) into a folder you can write to.
2. **Start openm2** (`openm2-launcher.exe`). It downloads the game's programs from the latest release, then asks for your client folder (a TMP4 40250 client you already have, the one with `pack` in it) and your keys file ([`keys.toml`](docs/specs/formats/keys.md)). Browse to them, or drop them on its window.
3. **It converts your client** into openm2's own formats inside that folder, in about a minute, and starts the game. Later starts take a second or two.
4. **On the Servers board, add a server** by its address. Its operator gives you its key, or the game shows it at the first join and asks you to trust it.

- **Updates:** at each start the launcher checks the latest release and downloads only the programs that changed, signed with openm2's release key ([docs/specs/update.md](docs/specs/update.md)). Your client's files are never downloaded or uploaded. When a version reads clients differently, the launcher imports yours again by itself and says why.
- **Servers:** openm2 lists none of its own; you join the servers you add. To run one, see [docs/server.md](docs/server.md).
- **Local saves:** the launcher's **Local saves** plays a world of your own on your computer, with no account and no internet, you its game master. Its world is made from your client too; each save is a folder in `saves/` ([docs/specs/update.md](docs/specs/update.md), "Local saves").

## Building
Prerequisites: [rustup](https://rustup.rs) (the toolchain is pinned in `rust-toolchain.toml` and installed on first build), [just](https://github.com/casey/just), and `cargo install --locked cargo-deny`.

```sh
just check             # everything CI checks
just openm2 messages   # list the messages of openm2's protocol

# Your own client's files, with your keys (docs/specs/formats/keys.md)
just client-check "<client folder>"      # decode and check everything (keys: local/keys.toml)
just openm2 pack ls "<client folder>/pack/<name>.eix" --keys local/keys.toml

# Convert your client into an asset folder of openm2's own formats, on your machine (a minute or so);
# openm2 plays and draws only asset folders (docs/specs/assets.md). Never commit it
just openm2 assets import --client "<client folder>" --keys local/keys.toml --out local/assets

# Look at a character, animated, in a window (or add --screenshot local/x.png)
just openm2 view model --assets local/assets \
  warrior_m.actor.toml --shape 4 --hair 2 --motion general/run

# Fly over a map (drag, wheel, WASD), or add --screenshot local/map.png
just openm2 view map metin2_map_a1 --assets local/assets

# Look at the interface's windows with your client's art
just openm2 view ui --assets local/assets

# Run openm2's own server (docs/server.md), with the OpenID Connect providers players sign in with
# (deploy/providers.example.toml shows the file; register openm2 at a provider first)
just openm2 content import --client "<client folder>" --keys local/keys.toml --out local/content   # maps' walk masks too; then write local/content/spawns/*.toml
just server serve --memory --providers local/providers.toml --content local/content
# ... or with Postgres, in Docker
OPENM2_DB_PASSWORD=<a database password> OPENM2_PROVIDERS_FILE=./local/providers.toml docker compose up --build -d
docker compose run --rm server tls key        # the server's key: sha256:…

# Play on it: sign in with a provider (your browser opens), pick a character, click or W, A, S, D to walk, right drag to turn, wheel to zoom, Enter to chat
just openm2 play --host 127.0.0.1 --server-key sha256:… --assets local/assets
# ... or put them in openm2.toml beside the program (docs/specs/client.md); `openm2 status` checks a server
```

For contributors and coding agents: start with [AGENTS.md](AGENTS.md).

## Security note
openm2's protocol runs under **TLS 1.3**. A server with a certificate for its name is checked like a website; a server with a self-signed certificate (what `openm2-server` makes for itself) is trusted by its key, which you pass with `--server-key` (or `OPENM2_SERVER_KEY`). Get that key from the server's operator, not from the server itself. You sign in at a provider's own page in your browser: openm2 never sees your password there, keeps no token and stores no password of its own, and the server knows you only by your id at that provider.

## License
Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

The project's logo in [`branding/`](branding/) is not under these licences: you may show it unchanged to point at openm2, and builds and forks you ship use their own name and logo ([branding/README.md](branding/README.md)).

"Metin2" is a trademark of its owners. It is used here only to describe compatibility. This project is not affiliated with or endorsed by them, and uses none of their logos or art.
