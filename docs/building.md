# Building openm2

How to build openm2 from source and try its tools on your own client's files. To play, the [latest release](https://github.com/OpenM2-dev/openm2/releases/latest) is enough; to run a server, see [server.md](server.md). Contributors and coding agents: [AGENTS.md](../AGENTS.md) has the rules and every command.

## Prerequisites
- [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml` and installs itself on first build.
- A C compiler, for the TLS library's C code (aws-lc-sys): on Windows the Visual Studio Build Tools with the C++ workload (rustup's installer offers them), on Linux gcc or clang.
- [just](https://github.com/casey/just) 1.44 or newer, and `cargo install --locked cargo-deny`.
- For the tests that draw, a graphics adapter. A software one is enough: Windows has WARP; on Linux without a GPU, Mesa's Vulkan drivers (`mesa-vulkan-drivers`).

`just preflight` lists what's missing. On Windows, builds are much faster with a Defender exclusion for `target/` or with the repo on a Dev Drive.

## Check everything
```sh
just check             # everything CI checks: fmt, clippy, tests, generated files, dependencies, licences
just openm2 messages   # list the messages of openm2's protocol
```

## Try the tools on your own client
Your client's files stay on your machine: the commands below read them with your keys ([specs/formats/keys.md](specs/formats/keys.md)) and write into `local/`, which git ignores. Never commit what they make.

```sh
# Decode and check everything (keys: local/keys.toml)
just client-check "<client folder>"
just openm2 pack ls "<client folder>/pack/<name>.eix" --keys local/keys.toml

# Convert your client into an asset folder of openm2's own formats (a minute or so);
# openm2 plays and draws only asset folders (specs/assets.md)
just openm2 assets import --client "<client folder>" --keys local/keys.toml --out local/assets

# Look at a character, animated, in a window (or add --screenshot local/x.png)
just openm2 view model --assets local/assets \
  warrior_m.actor.toml --shape 4 --hair 2 --motion general/run

# Fly over a map (drag, wheel, WASD), or add --screenshot local/map.png
just openm2 view map <map> --assets local/assets

# Look at the interface's windows with your client's art
just openm2 view ui --assets local/assets

# Grow trees from openm2's own recipes: no client needed
just openm2 view tree beech --season fall --variants 4 --screenshot local/trees.png
```

## Run a server and play on it
```sh
# The server's content from your client: maps, walk masks, monsters, items, skills (server.md, "Content")
just openm2 content import --client "<client folder>" --keys local/keys.toml --out local/content

# This device's key (made in local/device-key.pem the first time): ed25519:…
just openm2 key public --device-key local/device-key.pem

# A server on this machine, with no database and no provider: players sign in with their device's key,
# and yours is its game master. It logs its own key at start: sha256:…
just server serve --memory --key-sign-in --owner-key ed25519:… --content local/content

# Play on it: click or W, A, S, D to walk, right drag to turn, wheel to zoom, Enter to chat
just openm2 play --host 127.0.0.1 --server-key sha256:… --provider key --device-key local/device-key.pem --assets local/assets
# ... or put the settings in openm2.toml beside the program (specs/client.md); `openm2 status` checks a server
```

With Postgres and an OpenID Connect provider, in Docker, and everything else the server does: [server.md](server.md).
