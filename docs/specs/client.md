# The client's settings, the player's servers and `openm2 status`

**Code:** `crates/m2-cli/src/config.rs` (the file), `bot.rs` (`ServerArgs`: the server, for `bot`, `play` and `status`), `play.rs` (the asset folder and provider), `source.rs` (the asset folder), `servers.rs` (addresses, the player's servers file, asking and checking a server), `status.rs`, `m2_net::probe` and `m2_net::server_key`.

**Checked by:**
- the unit tests of `config.rs`, `bot.rs` and `servers.rs` (the last against openm2's own server in the process);
- `crates/m2-net/tests/driver.rs` (the probes, a key read);
- `crates/m2-cli/tests/own_server.rs` (`status` against the server, from a settings file).

The rules here are openm2's own.

## The settings file
- **Where it is:**
  - `--config FILE`, which must exist;
  - else `openm2.toml` beside the program, when there is one;
  - else there are no settings.
- **What it is:**
  - TOML, at most 64 KiB;
  - `format = 1` is required;
  - an unknown key is refused, and so is a value that doesn't fit, naming the file.
- **The keys, all optional but `format`:**

  | Key | What | Read by |
  |---|---|---|
  | `format` | `1` | |
  | `host` | the server's name or address; not empty | `bot`, `play`, `status` |
  | `server_name` | what `play`'s Servers board calls that server, 1 to 32 characters on one line (else its host); builds before 0.18.0 refuse the key | `play` |
  | `auth_port` | the auth server's port, 1–65535 | the same |
  | `game_port` | the game server's port, 1–65535 | the same |
  | `server_key` | the pin of the server's key, `sha256:` and 64 hex digits | the same |
  | `provider` | the provider to sign in with at once, as `--provider` | `play` |
  | `sign_in_prompt` | what the provider shows at every sign-in, OpenID Connect's `prompt`: `login` (its sign-in again, where another account can be chosen) or `select_account` (its choice of account). None by default: a provider that remembers the player then signs them in at once, so the same account comes back every time | `bot`, `play` |
  | `assets` | the asset folder to play with ([`assets.md`](assets.md)) | `play`, `openm2-launcher` |

- **Relative paths** (`assets`) start from the settings file's folder.
- **No client folder or keys file:** `play` reads only asset folders, so a file naming `client` or `keys` is refused as an unknown key. `openm2 assets import` converts a client into an asset folder; the launcher does it for players.
- **What wins:**
  1. a flag (`--host`, `--auth-port`, `--game-port`, `--server-key`, `--provider`, `--assets`);
  2. then its environment variable (`OPENM2_SERVER_KEY`);
  3. then the file;
  4. then the defaults: ports 11002 and 13001, no key (the system's authorities), the login screen's buttons.
- **Without a host, or without an asset folder** anywhere, the command stops and says which flag or key to set. `play` alone starts without a host: its Servers board then lists only the player's own servers (below), and only `--provider` and `--screenshot`, which sign in to the settings' server at once, need one.
- **Started by `openm2-launcher`** (`update.md`), `play` reads `openm2.toml` from the channel's version beside it, and gets the player's client imported as `--assets` (the install folder's `imported/`); a version whose `openm2.toml` names `assets` gets no flag.

An example, beside a downloaded `openm2.exe`:
```toml
format = 1
host = "play.example.net"
server_key = "sha256:5e1f0a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7"
assets = "imported"
```

## The player's settings
What the player chooses in the game's Settings window ([`ui.md`](ui.md), "The game menu"): the volumes, muting, the interface's size and the names over characters. Unlike `openm2.toml`, which is the game's and comes with each version, this file is the player's, and the game writes it.

- **Where it is:** `--settings FILE`, else `OPENM2_SETTINGS`; the launcher sets that to `settings.toml` in its install folder, which updates never touch ([`update.md`](update.md), "The install folder"). Without either, the settings last for the session.
- **What it is:** TOML, at most 64 KiB; `format = 1` is required.
  ```toml
  format = 1
  music_volume = 0.5      # 0 to 1
  sound_volume = 1.0      # 0 to 1, every sound but the music
  mute = false            # nothing plays; the volumes are kept
  interface_size = 1.0    # 0.5 to 2, times the display's scale or --ui-scale
  names = true            # names over characters' heads (render.md, "Names over characters")
  ```
- **Nothing in it stops the game:** a missing file gives the defaults above; one that can't be read (not TOML, another format, too large) gives them with a note; a value out of its range is brought into it, a key left out is its default, and a key this version doesn't know (a later one's) is left alone.
- **When it is written:** half a second after a change in the Settings window (a slider dragged writes it a few times, not every frame), and when the game closes; through a file beside it renamed into place, making its folder if need be. A failed write is a note; the game goes on.
- **What wins:** `--music-volume` and `--sound-volume` set their volume for the session over the file's; the Settings window shows them, and a change there writes them too. `--mute` opens no audio device at all; the file's `mute` keeps the device open, silent, so the window can unmute.

## The player's servers
The servers a player added on `play`'s Servers board ([`ui.md`](ui.md), "The Servers board"), besides the one `openm2.toml` names.

- **An address:**
  - `host`, for the ports 11002 and 13001, or `host:AUTH:GAME`;
  - an IPv6 address in brackets: `[::1]:11102:13101`;
  - optionally followed by `#` and the server's key, `sha256:` and 64 hexadecimal digits: the server's link, which its operator can publish.

  Spaces around it are left out. A host is letters, digits, `-` and `.` (an IPv4 address too), at most 253 of them.
- **Where the file is:** `--servers FILE`, else `servers.toml` beside the player's settings file (`--settings` or `OPENM2_SETTINGS`, above), so the launcher's install folder holds it ([`update.md`](update.md), "The install folder"). Without either, the list lasts for the session.
- **What it is:** TOML, at most 64 KiB; `format = 1` is required.
  ```toml
  format = 1
  last = "127.0.0.1:11102:13101"      # the server joined last, by its address without the key
  [[server]]
  name = "My test server"             # 1 to 32 characters on one line
  address = "127.0.0.1:11102:13101"   # as above, without the key
  key = "sha256:5e1f0a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7"   # once trusted
  ```
- **Nothing in it stops the game:** a missing file is an empty list; one that can't be read (not TOML, another format, too large) is an empty list with a note; a server whose name, address or key doesn't read, or whose address is listed already (the same host, case aside, and ports), is left out with a note; past 64 servers, the rest are left out with a note. Keys a later version writes are left alone.
- **When it is written:** after each change on the Servers board (a server added, changed, removed or trusted, or joined), through a file beside it renamed into place, making its folder if need be. A failed write is a note; the game goes on.
- **Trusting a server** (`servers::check`, before a join):
  - a key in its link, or one trusted before: pinned;
  - none, and a certificate the system's authorities vouch for: trusted so, nothing kept;
  - none, and a self-signed certificate: the key is read in a TLS handshake of its own, in which the server proves it holds it (`m2_net::server_key`), and the player is asked whether to trust it; trusted, it is kept in the file;
  - a key that differs from the one trusted: refused (`m2_net::ProbeError::OtherKey`);
  - the settings file's server is trusted as its settings say (`server_key`, else the system's authorities), and never asked about.

## `openm2 status`
It checks that a server is up and speaks openm2, without signing in. Scripts and deploys use it.
- **The auth server:**
  - the same session the login screen runs to ask for the providers (`SessionConfig::sign_in_options`): TLS, `Hello`, `Welcome`, then the providers;
  - prints `auth  HOST:PORT  NAME, protocol N`, then `providers: id (name), …`.
- **The game server:**
  - a TLS handshake with the trusted key and ALPN `openm2/1`, then a clean close (`m2_net::probe`);
  - prints `game  HOST:PORT  answered from ADDRESS, TLS 1.3 and openm2/1`.
- **When something fails,** its line says `failed:` and why. That includes the pin the server's key didn't match, a `Refused` from another version, or a timeout. Asking the auth server is the same as `play`'s Servers board does (`servers::ask`).
- **The exit code** is 0 when both servers answered, else 1.
