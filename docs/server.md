# Running openm2's own server

`openm2-server` is openm2's own server: auth and game in one process, for the openm2 client, speaking openm2's own protocol ([`specs/protocol/openm2/`](specs/protocol/openm2/README.md)), with accounts and characters in Postgres and its world in a content folder. What it does is in [`specs/server/session.md`](specs/server/session.md) and [`specs/server/world.md`](specs/server/world.md); what it stores in [`specs/server/storage.md`](specs/server/storage.md); its content in [`specs/server/content.md`](specs/server/content.md).

> **What it does:** players sign in, create characters, enter, walk (the maps' walls hold), see other players, talk, shout, whisper and keep friends; characters are kept in Postgres and found where they were left. Monsters and NPCs from a content folder stand and stroll around the players; players fight them, the monsters fight back, both die, monsters come back and players restart, gain experience and levels. New characters start with the content's level, items and gold; players wear and take off items, drink potions, drop and pick up items and gold, monsters drop gold and items, NPCs open their shops and smiths upgrade items. Players choose a skill group, spend skill points and use skills; game masters the operator names have commands of their own. The bot does most of it on our server in CI.

> **Sign-in:** players sign in only with the OpenID Connect providers you list (Google, Microsoft, Clerk, Keycloak, …). The server keeps no passwords, and it does not start without a provider ([Sign-in with a provider](#sign-in-with-a-provider)).

> **TLS:** both ports speak TLS 1.3 only. Without a certificate of yours the server makes a self-signed one, and players pin its key ([TLS](#tls)). The server listens on 127.0.0.1 by default, and `compose.yaml` publishes its ports on the host's 127.0.0.1 unless `OPENM2_PUBLISH` says otherwise.

## With Docker
`compose.yaml` runs the server and Postgres (on Windows: inside WSL, `wsl docker compose …` from the repo folder, or a WSL shell in `/mnt/c/…`):
```sh
export OPENM2_DB_PASSWORD=<a password for the database>    # or put both in .env (gitignored)
export OPENM2_PROVIDERS_FILE=./local/providers.toml        # your providers (Sign-in with a provider)
docker compose up --build -d                               # Postgres, then the server; migrations run at start
docker compose run --rm server tls key                     # the key players pin: sha256:…
docker compose logs -f server
docker compose stop                                        # saves every character, then stops
```
- The image is built from the repo (`Dockerfile`): the server alone, in a small runtime image (distroless, no shell) as a non-root user. Base images are pinned by digest.
- Postgres keeps its data in the `postgres` volume, and the server its self-signed certificate in the `data` volume ([TLS](#tls)); `docker compose down` keeps them, `down -v` deletes them (and players then need the new key).
- `OPENM2_PUBLISH=0.0.0.0` publishes the game's ports on every address of the host, for players on other machines.
- `OPENM2_PROVIDERS_FILE` names your providers file (`deploy/providers.example.toml` shows the format), their secrets in `./local/provider-secrets.env` ([Sign-in with a provider](#sign-in-with-a-provider)). Compose refuses to run without it.
- The server waits up to 30 s for Postgres at start (`docker compose restart` starts both at once), then gives up; compose's restart policy starts it again.
- **Telemetry:** `OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 docker compose --profile telemetry up -d` adds an OpenTelemetry collector that prints the traces and metrics it receives (`docker compose logs otel-collector`; its configuration is `deploy/otel-collector.yaml`). Point the endpoint at your own collector or backend to keep them.

## Running from the published image
Each release publishes the server's image: `ghcr.io/openm2-dev/openm2-server:X.Y.Z`, with its digest in the release's notes. A deployment pulls it by digest instead of building:
- **The settings** are the same as `compose.yaml`'s ([Settings](#settings)):
  - `OPENM2_DATABASE_URL`, the providers file and their secrets, the content folder;
  - `OPENM2_LISTEN=0.0.0.0` inside a container.
- **The database** is reached over TLS, checked against its certificate and name, unless the URL's `sslmode` says otherwise ([The database's TLS](#the-databases-tls)).
- **Keep the key players pin across new images:**
  - Make the certificate once, outside the deployment: `openm2-server tls key --data DIR` writes `DIR/tls/cert.pem` and `key.pem` and prints the pin.
  - Keep both files as the deployment's secret, and hand them to every new server with `OPENM2_TLS_CERT` and `OPENM2_TLS_KEY`.
  - A data volume kept across deploys works too ([TLS](#tls)).
- **One server per database:**
  - A running server holds the database's world ([`specs/server/storage.md`](specs/server/storage.md), "One world per database").
  - A second one logs `another server runs this database's world: waiting for it to stop`, and starts once the first has stopped and saved everyone.
  - So a deploy stops the old server before it starts the new one.
- **Check it from anywhere** with `openm2 status --host <name> --server-key sha256:…` ([`specs/client.md`](specs/client.md)). It prints the server's version and providers and checks the game port's TLS. Its exit code is 0 when both servers answer.

## Without Docker
```sh
just server serve --memory --providers local/providers.toml --data local/server   # no database: everything is lost on stop; the certificate kept in local/server/tls
OPENM2_DATABASE_URL="postgres://user:password@127.0.0.1/openm2?sslmode=disable" just server serve --providers local/providers.toml --data local/server    # Postgres on this machine, without TLS
OPENM2_DATABASE_URL=… just server migrate         # only the migrations
```
On Windows in PowerShell set variables with `$env:NAME = "…"` first.

## Content
The world's maps, monsters, items and spawns come from a content folder in openm2's own format ([`specs/server/content.md`](specs/server/content.md)). Without one the world is empty: players only.

**For the Metin2 maps, from your own client:**
```sh
just openm2 content import --client "<client folder>" --keys local/keys.toml --out local/content
```
It writes `content.toml`, `maps.toml`, `monsters.toml`, `items.toml`, `races.toml` and `skills.toml` (from the client's skill table), and a walk mask per map in `walk/` (where one may walk, from the maps' attribute files: a black and white PNG a pixel a metre), into `local/content` (gitignored), each marked as imported. A mask can be edited in any image editor (black is blocked), or drawn for a map that has none and named with `walk = "walk/<name>.png"` in `maps.toml` ([`specs/server/content.md`](specs/server/content.md), "Walk masks"). The experience each level takes can go into a `levels.toml` of your own ([`specs/server/content.md`](specs/server/content.md), "Level"); without it the server has a table of its own. What monsters drop goes into a `drops.toml` of your own, what new characters start with (a level, gold, items) into a `start.toml`, what NPCs sell into a `shops.toml`, which NPCs teach the skill groups into a `teachers.toml`, and how skills train past a master's into a `training.toml` ("Drop", "Starting items", "Shops", "Teachers", "Training"); without them monsters drop only their gold, characters start at level 1 with nothing, NPCs sell nothing, players choose a skill group with `/skillgroup 1` or `2`, and only skill books train skills past a master's, with openm2's chance and wait. From level 5 players choose their group at a teacher of their class, or with `/skillgroup` in a world without teachers, and spend skill points in `play`'s skills window. They hold your client's data: **never commit them** (`docs/source-policy.md`); `just no-assets` refuses them. It also writes `spawns/imported.toml`: the spawn lists your client's maps carry, if any ([`specs/server/content.md`](specs/server/content.md), "The import"). Their group lines name packs by number, and which monsters a pack holds is not in the client: write them into a `packs.toml` of your own ("Pack"), and the import's summary lists the numbers each map waits for. A second import needs `--force`, which replaces those files, `spawns/imported.toml` among them, and never touches `packs.toml` or the other spawn files.

**Then the spawns:** write `local/content/spawns/<name>.toml` yourself for what the client doesn't carry: groups of monsters (a monster number or a pack, a circle or a rectangle on a map, how many) and NPCs (a number and a point). Positions are metres from the map's top-left corner: `just openm2 map info <map> --client <dir>` shows the map's points, and `just openm2 view map <map> --assets <dir> --at X,Y --pitch 90 --screenshot local/spot.png` shows a spot from above (of an asset folder `assets import` made of the client: below).
```toml
format = 1

[[group]]
map = "metin2_map_a1"
at = [250, 80]
radius = 12
monster = 101
count = 5

[[npc]]
map = "metin2_map_a1"
at = [244.5, 61]
monster = 20016
rotation = 180
```

**Run it:**
```sh
just server content check local/content                               # read and check it, and say what it holds
just server serve --memory --providers local/providers.toml --content local/content    # or OPENM2_CONTENT=local/content
OPENM2_CONTENT_DIR=./local/content docker compose up -d               # Docker: mounted read-only at /content
```
The server refuses to start on content that breaks a rule, and lists each problem as `file:line:column: message`.

## TLS
Both listeners speak TLS 1.3 only, with ALPN `openm2/1` ([`specs/protocol/openm2/README.md`](specs/protocol/openm2/README.md), "TLS"). The certificate comes from the first of:
1. **Yours:** `--tls-cert` and `--tls-key` (`OPENM2_TLS_CERT`, `OPENM2_TLS_KEY`), PEM files, the chain with the server's certificate first (Let's Encrypt's `fullchain.pem` and `privkey.pem`, for example). Players connect to the name in it, with no pin: `--host play.example.org`. Restart the server when you renew it.
2. **The server's own:** `--data DIR` (`OPENM2_DATA`; the Docker image sets `/data`). At the first start the server makes a self-signed certificate (ECDSA P-256) into `DIR/tls/cert.pem` and `DIR/tls/key.pem`, the key readable only by its owner on Unix, and uses it from then on.
3. **Neither:** a self-signed certificate for this run only, with a warning: its key changes on every start.

**The key players pin** is the SHA-256 of the certificate's key, `sha256:` and 64 hex digits. The server logs it at start (`TLS 1.3 with the key sha256:…`), and `openm2-server tls key` prints it (with `--data` or the certificate's files). Players pass it to `play` and `bot` as `--server-key` or `OPENM2_SERVER_KEY`. Without it the client wants a certificate from an authority the system trusts, refuses a self-signed one and says what to do. A new certificate has a new key, so players need the new pin; give it to them yourself, not through the server.
- The private key is never logged. Certificate and key files are read up to 1 MiB.
- The game connection resumes the auth connection's TLS session (stateless tickets, one per connection, no early data).
- A connection has 10 s to finish its handshake, and at most 256 are under way at once, 16 from one address ([`specs/server/session.md`](specs/server/session.md), "Connections"). Failed handshakes are logged at debug (`RUST_LOG=m2_server=debug`) and counted in `openm2.tls.handshakes`.

## The database's TLS
The server talks to Postgres over TLS and checks the database's certificate and name, unless `OPENM2_DATABASE_URL` says otherwise with `sslmode`. This is openm2's default, `verify-full`; libpq and sqlx would `prefer` TLS without checking it.
- **A managed database** whose certificate comes from a public authority needs nothing more. The server trusts Mozilla's list of authorities (webpki-roots), built into it.
- **A database with its own authority:** add `sslrootcert=/path/ca.pem`, a PEM file the server can read (mounted into its container). It is trusted on top of the list.
- **A database beside the server without TLS,** as in `compose.yaml`: add `sslmode=disable`. Only do that when both share a private network.
- **The other modes:** `verify-ca` checks the authority but not the name; `require`, `prefer` and `allow` encrypt without checking (the server warns about them).
- **At start** the server logs `the database: HOST:PORT, sslmode=MODE`, never the URL.
- **When TLS fails,** the server stops at once and says why: the database offers no TLS, its certificate comes from an unknown authority, or it is for another name. It doesn't try again, unlike a database that isn't answering yet.

## Sign-in with a provider
Players sign in with an OpenID Connect provider of yours, or with their device's own key if you allow it ([Sign-in with a device key](#sign-in-with-a-device-key)): the server keeps no passwords. Any provider that publishes discovery (`/.well-known/openid-configuration`) works: Google, Microsoft, GitLab, Keycloak, Auth0, Clerk, … `play`'s login screen shows a button per provider; `bot --provider ID` prints the address to open. A player's first sign-in makes their account, found again by the provider's issuer and their id there ([`specs/server/storage.md`](specs/server/storage.md), "Accounts"). The server does not start without a way in: a provider, or devices' keys. To try a provider on your own machine, register a provider's free development tenant (a Clerk development instance, a Google project's desktop client) or run Keycloak.

1. **Register openm2 at the provider** as a desktop (native, "installed") application, with the authorization code flow and PKCE:
   - the redirect URI `http://127.0.0.1/callback`: the client listens on a port the system picks, and providers that follow RFC 8252 accept any port on the loopback address (some want the port left out, some `http://localhost` registered as well);
   - the scope `openid` only: the server reads no e-mail or profile;
   - a public client needs no secret; a confidential one gets `client_secret_post`.
2. **List it** in a TOML file (`--providers`, `OPENM2_PROVIDERS`; `deploy/providers.example.toml`), 1 to 8 of them:
   ```toml
   [[provider]]
   id = "google"                       # 1 to 16 of a-z, 0-9 and -; new accounts' names start with it
   name = "Sign in with Google"        # the button's text: words your players know
   issuer = "https://accounts.google.com"   # exactly as its ID tokens say
   client_id = "…apps.googleusercontent.com"
   ```
3. **Its secret**, if it has one, goes in `OPENM2_PROVIDER_<ID>_SECRET` (the id in capitals, `_` for `-`: `OPENM2_PROVIDER_GOOGLE_SECRET`), never in the file. With Docker, in `./local/provider-secrets.env`, one `NAME=value` a line.
4. **Start the server.** It reads each provider's discovery document and keys first, and refuses to start if one does not answer, names another issuer, or offers no PKCE with S256; the log says `provider google: https://accounts.google.com (2 keys)`.

- **What the server checks** ([`specs/server/session.md`](specs/server/session.md), "Signing in with a provider"): it exchanges the code itself, then wants an ID token signed with RS256 or ES256 by one of the provider's keys, for its client id, from the exact issuer, not expired, with the nonce it gave that connection. Keys are fetched again every 15 minutes and when a token names a new one.
- **The client** sees the code once and holds no token; the server keeps none.
- **Requests to providers** are HTTPS only, except to a provider on the loopback address (for development), with the system's authorities (the Docker image has Debian's), 10 s each; `HTTPS_PROXY` applies.
- **Logs** name the provider and the account by number; refusals give their reason at debug (`RUST_LOG=m2_server=debug`), never a code or a token. `openm2.sign_ins` counts sign-ins by provider and result.
- **Account names** are the provider's id, a colon and 13 letters and digits (`google:nixzoi2rkfeta`).

## Sign-in with a device key
With `--key-sign-in` (`OPENM2_KEY_SIGN_IN=true`) players may also sign in with their device's own key: an Ed25519 key the client makes the first time and keeps beside its settings ([`specs/client.md`](specs/client.md), "The device's key"). No provider, no browser and no internet are needed, so a server on your own machine runs with nothing else:
```sh
openm2-server serve --memory --key-sign-in --owner-key "$(openm2 key public)"
openm2 play --host 127.0.0.1 --server-key sha256:… --provider key --assets …
```
- **What the server checks** ([`specs/server/session.md`](specs/server/session.md), "Signing in with a device key"): the key's signature over the nonce it gave the connection and that connection's own TLS channel binding, so a signature seen on the wire is worth nothing anywhere else. The key names the account; the message names none.
- **Who gets in:** anyone who reaches the server makes an account with a key of their own, so leave it off on a public server unless that is what you want. A key gives no other right.
- **The owner:** `--owner-key ed25519:…` (`OPENM2_OWNER_KEY`), the key `openm2 key public` prints, makes that key's account a game master ([Game masters](#game-masters)). It needs `--key-sign-in`.
- **Account names** are `key:` and 13 letters and digits; `openm2 key public` and the server agree on them (`m2_server::store::key_account_name`).
- **With providers too:** both work at once; the providers file may not use the id `key`.

## Saves
A world on your own machine, without a database: `--save DIR` keeps accounts and characters in one file, `DIR/world.json`, written at every change ([`specs/server/storage.md`](specs/server/storage.md), "In a save folder"). The folder is also the server's data folder, so its key (`DIR/tls/`) stays the same from one start to the next, unless `--data` or `--tls-cert` says otherwise.
```sh
openm2-server serve --save local/saves/mine --key-sign-in --owner-key "$(openm2 key public)"
```
- **One server at a time** holds a save; a second one on the same folder stops at start, saying so.
- **A save is your data:** copy the folder to keep it, or to move it to another machine. A `world.json` that is damaged, or written by a newer server, is refused and left as it is.
- **With Postgres too?** No: `--save` and `--memory` each replace the database, and only one store is used.

## Run by another program
A program that starts the server for its player (the launcher for a local save, [`specs/update.md`](specs/update.md), "Local saves"; a script) wants to know when it is ready and to stop it cleanly:
- **`--ready-line`:** once both servers listen, the server writes one line on stdout, then nothing more there (its logs go to stderr): `ready auth=PORT game=PORT key=sha256:…`, the ports it listens on (useful with `--auth-port 0 --game-port 0`, which take free ones) and the pin of its key, for the client's `--server-key`.
- **`--until-stdin-closes`:** the server stops, saving everyone, when its stdin closes, as well as on Ctrl+C. A program that holds the server's stdin and closes it (or ends, even by crashing) stops it. Off by default: `docker run` without `-i` gives a closed stdin, which would stop the server at once.
- **The exit code** is 0 after a clean stop, else 1 with the reason on stderr.

## Settings
| Setting | Default | What |
|---|---|---|
| `OPENM2_DATABASE_URL` | none (required without `--memory` or `--save`) | the Postgres URL; it holds the database's password, so it is only ever read from the environment and never printed. Its `sslmode` is `verify-full` unless it names another ([The database's TLS](#the-databases-tls)) |
| `OPENM2_CONTENT`, `--content` | none: an empty world | the content folder ([Content](#content)); the Docker image reads `/content`, which `compose.yaml` mounts from `OPENM2_CONTENT_DIR` (default `deploy/empty-content`, an empty world) |
| `OPENM2_LISTEN`, `--listen` | `127.0.0.1` | the address both listeners bind, `0.0.0.0` for every address (the Docker image sets `0.0.0.0` inside the container) |
| `OPENM2_TLS_CERT`, `--tls-cert`; `OPENM2_TLS_KEY`, `--tls-key` | none | your certificate chain and its key, PEM ([TLS](#tls)) |
| `OPENM2_DATA`, `--data` | none (the Docker image: `/data`); a save's folder with `--save` | the server's folder: its own self-signed certificate in `tls/`, made at the first start ([TLS](#tls)) |
| `OPENM2_SAVE`, `--save` | none | a save's folder: accounts and characters in `world.json` instead of Postgres ([Saves](#saves)) |
| `--ready-line`, `--until-stdin-closes` | off | for a program that runs the server ([Run by another program](#run-by-another-program)) |
| `OPENM2_PUBLISH` | `127.0.0.1` | `compose.yaml` only: the host's address the game's ports are published on; `0.0.0.0` for every address |
| `OPENM2_PROVIDERS`, `--providers` | none: required without `--key-sign-in` | the providers file ([Sign-in with a provider](#sign-in-with-a-provider)); the Docker image reads `/providers.toml`, which `compose.yaml` mounts from `OPENM2_PROVIDERS_FILE` (required) |
| `OPENM2_KEY_SIGN_IN`, `--key-sign-in` | `false` | devices' own keys may sign in ([Sign-in with a device key](#sign-in-with-a-device-key)) |
| `OPENM2_OWNER_KEY`, `--owner-key` | none | a device's key, `ed25519:` and 64 hex digits, whose account is a game master; needs `--key-sign-in` |
| `OPENM2_PROVIDER_<ID>_SECRET` | none | a provider's client secret, for a confidential client; with Docker, in `./local/provider-secrets.env` |
| `OPENM2_GAME_MASTERS`, `--game-masters` | none: nobody | the accounts that may use the game masters' commands, account names separated by commas ([Game masters](#game-masters)) |
| `OPENM2_AUTH_PORT`, `--auth-port` | `11002` | the auth server's port |
| `OPENM2_GAME_PORT`, `--game-port` | `13001` | the game server's port |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | none | where traces and metrics go (OTLP over HTTP, e.g. `http://127.0.0.1:4318`); unset or empty: logs only. The other standard `OTEL_*` variables apply too |
| `RUST_LOG` | `info` | what is logged to stderr (`m2_server=debug`, `warn`, …) |

The game's rules (start positions, class values, the view radius, timeouts) are `m2_server_core::Config`'s defaults; they are not settable from outside yet.

## Game masters
Players of the accounts you name may set their character's level and give it gold and items in the game, and a mark shows over their characters ([`specs/server/session.md`](specs/server/session.md), "Game masters"):

| Command | What |
|---|---|
| `/level N` | the level becomes N, with the points of each level gained |
| `/gold N` | N more gold |
| `/item NUMBER [COUNT]` | the item into the bag, COUNT at most its stack |
| `/heal` | health and mana filled |
| `/mount N` | the mount at level N, 0 for none |
| `/speed N`, `/speed` | moving at speed N, 1 to 1000 (the server's own is 150; 1000 is 6.7 times as fast), until it leaves the game; alone, its own speed again |
| `/gm off`, `/gm on` | the mark over the character hidden, or shown again; kept with the character |

1. **Find your account's name:** sign in and type `/account` in the chat. It answers `Your account: google:nixzoi2rkfeta`. It is the same on every server with the same provider, and after the database is wiped.
2. **Name it** in `OPENM2_GAME_MASTERS` (or `--game-masters`), several separated by commas, then start the server again. The log says `game masters: 1`. A name that is not one stops the server at start, saying which. On a server that takes devices' keys, `--owner-key` names a key instead ([Sign-in with a device key](#sign-in-with-a-device-key)).
3. For anyone else these commands are unknown, as any other. Each one used is in the log: `game master account 2 on character 5: /item 11299 1`.

## What it records
- **Logs** (stderr): what the content holds, the TLS key's pin, the providers, how many game masters, connections, sign-ins, characters created, entering and leaving, deaths and level-ups, game masters' commands, migrations, storage errors; failed TLS handshakes at debug. Accounts and characters by number; never tickets, providers' codes and tokens, client secrets, private keys, the database URL or chat. A client's build (from its `Hello`) is logged with control characters as `?`.
- **Traces:** a span for each storage request (`store`, with the request's kind) and each sign-in checked (`sign_in`, with the provider).
- **Metrics:** `openm2.connections`, `openm2.players`, `openm2.entities` (monsters and NPCs), `openm2.groups.awake` (groups with a player near) and `openm2.ground_items` (items on the ground) (gauges), `openm2.logins` (by result: `ok` for a sign-in given a ticket, or the refusal of a sign-in or a ticket), `openm2.sign_ins` (by provider and result: `ok`, `refused`, `unavailable`), `openm2.tls.handshakes` (by result: `full`, `resumed`, `failed`, `timed_out`, `no_alpn`, `busy`), `openm2.messages.received` and `openm2.messages.sent`, `openm2.store.duration` (seconds, by request).

## Play on it
With the server running, and its key from its log or `tls key` ([TLS](#tls)):
```sh
export OPENM2_SERVER_KEY=sha256:…                 # or --server-key sha256:…
just openm2 assets import --client "<client folder>" --keys local/keys.toml --out local/assets   # once
just openm2 play --host 127.0.0.1 --assets local/assets
```
- `play` plays from an asset folder, openm2's own formats ([`specs/assets.md`](specs/assets.md)): `assets import` converts your client into one, on your machine, in a minute or so. Never commit it. The launcher does this for players ([`specs/update.md`](specs/update.md)).
- The login screen shows a button per provider of the server's; the system's browser opens at the provider, and the game goes on once you have signed in there. `play --provider google` signs in at once (`--no-browser` only prints the address).
- A new account has no characters: `play` shows the create screen. New characters start in their empire's first village ([`specs/server/session.md`](specs/server/session.md), "Creating").
- Without a window: `just openm2 bot --host 127.0.0.1 --provider google --create Hero --empire jinno --say "hello" --walk 3,0 --stay 5` prints the address to open in a browser, then plays. It prints the bytes its sockets carried at the end (`traffic:`).
- `--host` takes a name too; with your own certificate it must be the name in it.
- Two clients see each other within 100 m.
- **Instead of flags,** an `openm2.toml` beside the program (or `--config FILE`) can hold the host, ports, key, provider and asset folder ([`specs/client.md`](specs/client.md)). For players who download the client, it saves typing them.

## What it costs
One process with two worker threads, a task per connection and at most 8 database connections; it ticks ten times a second and does nothing in between. Monster groups with no player near sleep and cost nothing. A client gets at most 2,048 messages queued before it is closed for not reading (entering a crowded spot sends up to three per monster in view); what is queued goes out in one write.

**Traffic:** the bot's whole run (log in, create a character, shop, fight, use a skill, drop and pick up, 10 s in the game) takes about 1,650 bytes of messages both ways, and about 9,800 on the sockets with TLS, most of them its two handshakes ([`specs/protocol/openm2/README.md`](specs/protocol/openm2/README.md), "Size").

Measured with `compose.yaml` (Docker 29 in WSL, `docker stats`):

| | |
|---|---|
| the server's image | 11.3 MB to download, 44 MB unpacked |
| the server, idle | about 1 MB of memory, 0.07 % of a CPU |
| Postgres 18, idle | 43 MB of memory |
| the server with a whole client's imported content (1,334 monsters, 5,743 items) and a starter world of 10 monsters and 6 NPCs | about 14 MB of memory, 5.6 MB of it its own (a release build on Windows) |
| building the image | about a minute (fat LTO), with the build cache warm |

So a small VPS holds both with room to spare; players' load is not measured yet.

## Tests
- `cargo test -p m2-content`: the content format: every rule, written files read back, random folders.
- `cargo test -p m2-server-core`: the session's and the world's rules, the fights, the items, the shops and the skills, one snapshot per scenario, and random sessions (with monsters, fights, items, shops and skills too); every player signs in through the provider's messages, the test answering for the edge's checks.
- `cargo test -p m2-server`: real clients (`m2-core` and `m2-net`, as the bot uses them) against the running server, each signing in with the fake provider as a player the test names, one of them watching monsters stroll, one refused when the provider's token is for another client; TLS from outside (`tls.rs`: plaintext, no or another ALPN name, a handshake that never comes, too many handshakes from one address); the certificate in a data folder; providers' sign-ins against a fake provider made at test time (`oidc.rs`: the exchange, a token for another connection or client, a code used twice, keys that change, a provider that names another issuer or is down; `clients.rs`: a player signing in twice), and the ID token's checks with tokens built by hand (`alg: none`, HMAC with the RSA key, wrong claims, unknown keys); storage's rules on every store, identities too, and a save read back, held by one server and refused when damaged or newer; a character found where it was left after a restart, on a save and on Postgres; devices' keys (`keys.rs`); the binary run as another program would (`serve.rs`: a save, the ready line, stopped by closing stdin, started again); TLS to the database (`postgres_tls.rs`: through a stand-in with the test's own authority, verified, and refused for an unknown authority, another name or no TLS). The Postgres tests run when `OPENM2_TEST_DATABASE_URL` is set (each makes its own schema and drops it) and are skipped otherwise; CI runs them on Linux against a Postgres service.
- `cargo test -p m2-cli --test own_server`: the `openm2 bot` binary on our server, signing in with the fake provider (the test plays the browser) (the messages of entering in the session spec's order), the bot seeing monsters stroll, killing one (`--fight`), dying and restarting (`--restart`), wearing, dropping and picking up items and gold (`--use`, `--drop`, `--drop-gold`, `--pick-up`), shopping (`--shop`, `--buy`, `--sell`), using a skill (`--skill-up`, `--skill`), signing in twice to one account, another player to another, and a refused sign-in, and all of it but the restart in one run, with its bytes on the wire checked and its game connection resuming the TLS session; `--test content`: the import on a synthetic client.
- CI also builds the Docker image, starts it and has it make its certificate in `/data` (`tls key`).
