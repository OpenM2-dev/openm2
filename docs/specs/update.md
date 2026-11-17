# Updates: signed manifests and the launcher

**Code:** `crates/m2-update` (the manifest, its signature, the checks, the plan and the requests that fetch it, bundles; no I/O), `crates/m2-launcher` (`openm2-launcher`: downloads, the install folder, starting the game, its window), `crates/m2-cli/src/release.rs` (`openm2 release`).

**Checked by:**
- `crates/m2-update/tests/manifest.rs` (byte-exact vectors of both formats, refusals, paths, bundles cut and the requests that fetch them, random input);
- `crates/m2-launcher/tests/update.rs` (updates from a local HTTP server into a temporary folder; a flat channel behind a redirect, as a GitHub release's downloads, and a redirect away from HTTPS refused; bundles fetched whole and then by range, a server that doesn't answer ranges, a bundle stopped part-way, a content at several paths downloaded once, requests at once; how far an update says it has got; a version naming `assets` started without a client; a game that doesn't import asking for the client folder);
- `crates/m2-launcher/tests/window.rs` (the window drawn offscreen in a game's colours with its picture) and the unit tests of `src/window/screen.rs` (each stage's texts and buttons), `src/progress.rs` (the console's lines) and `src/settings.rs` (`layout`, `[window]`, and the release's own `deploy/windows/launcher.toml`);
- `crates/m2-cli/tests/launcher.rs` (the launcher's import with `openm2` itself, on a synthetic client: imported at the first start, current at the next, imported again for another import's number, a failed import leaving the last, one that stopped part-way deleted);
- `crates/m2-cli/tests/release.rs` (`release keygen`, `manifest`, `sign` and `verify` together; imported files refused, and let through with `--allow-imported`; `--bundles` and `--add`).

Decisions 021, 029, 035 and 036. Every rule here is openm2's own.

## What is trusted
- **The signature, not the server.** A manifest counts only when it is signed by the key in the install folder's `launcher.toml`. HTTPS keeps the downloads private, but a CDN or a server that lies can only stop updates, not change what is installed.
- **Every file is checked** against the size and SHA-256 its manifest gives, before it is used, whether it came on its own or in a bundle.
- **Versions only go up:** a manifest older than the installed one is refused, and so is one past its expiry, so an old manifest can't be served forever to hold players back.
- **Paths never leave the install folder** (below).

## The signed manifest
One file, so a manifest and its signature are always published together:

| Bytes | What |
|---|---|
| 0–152 | the signature line: `openm2-manifest ed25519 `, the signature as 128 lowercase hex digits, a line feed (`\n`) |
| 153– | the manifest: JSON, UTF-8, at most 8 MiB |

- **The signature** is Ed25519 (RFC 8032) over the manifest's bytes exactly as they follow the line. They are checked before they are read as JSON.
- **The manifest:**

  | Key | What |
  |---|---|
  | `format` | `1`, or `2` for a manifest with bundles |
  | `channel` | the channel's name: 1–32 of `a-z`, `0-9` and `-` |
  | `sequence` | a whole number from 1 that only goes up from one manifest of the channel to the next (`openm2 release manifest` takes the time in seconds) |
  | `version` | what players are told they run: 1–64 printable ASCII characters |
  | `expires` | when the manifest stops counting: `YYYY-MM-DDTHH:MM:SSZ`, UTC |
  | `bundles` | format 2 only: 1–65,536 bundles (below), each with its `size` in bytes (1 byte to 4 GiB) and `sha256` |
  | `files` | 1–65,536 files: `path`, `size` in bytes (at most 4 GiB each, 64 GiB together) and `sha256` (64 lowercase hex digits); a bundled file also has `bundle`, its bundle's place in `bundles` from 0, and `offset`, where its bytes start in it |

  An unknown key is refused. `openm2 release manifest` writes the files in the order of their paths' bytes and the JSON indented by two spaces; readers take any order.
- **Bundles** carry a version's small files together, so a first install needs few requests:
  - a bundle is the bytes of its files one after another, nothing else;
  - a bundled file has at least one byte and ends inside its bundle;
  - **a bundle's files cover it exactly:** by offset, from 0 to its size, without gaps or overlaps. Files at the same offset have the same size and SHA-256: one content at several paths. Every bundle is named by a file. So every byte of a bundle is checked as a file's;
  - format 1 has no `bundles`, `bundle` or `offset`, and format 2 has at least one bundle. A manifest without bundles is written as format 1, which launchers before bundles read.
- **Paths** are where a file goes in the version's folder:
  - 1–240 bytes, parts split by `/`, at most 32 parts;
  - each part 1–120 of `A-Z`, `a-z`, `0-9`, space and `. _ - + ( )`, not starting with `.` or a space and not ending with `.` or a space;
  - no part is a Windows device name (`con`, `prn`, `aux`, `nul`, `com0`–`com9`, `lpt0`–`lpt9`, in any case, with or without an extension);
  - no two paths are the same ignoring case, and no file is the folder of another.

  So there are no absolute paths, drive letters, `..`, backslashes, `:` streams or hidden files.

## The keys
- **The public key** is written `ed25519:` and 64 hex digits (32 bytes), in either case. It goes in `launcher.toml`.
- **The private key** is a PKCS#8 file in PEM (`-----BEGIN PRIVATE KEY-----`), as `openm2 release keygen` writes it and `openssl genpkey -algorithm ed25519` does. Signing reads it from the environment variable `OPENM2_RELEASE_KEY`, never from a file named on the command line or from an argument.

## On the server
Under the channel's address (`url` in `launcher.toml`), in one of two layouts (`layout` in `launcher.toml`):

| | `folders` (the default) | `flat` |
|---|---|---|
| the channel's signed manifest | `<channel>/manifest.signed` | `<channel>.manifest.signed` |
| each file on its own, and each bundle | `files/<sha256>` | `<sha256>` |

- **The signed manifest** must not be cached for long: players see a new version as soon as it's there.
- **Files and bundles** are named by their SHA-256 in lowercase hex. They never change, so they can be cached forever, and channels share them.
- **`flat`** is for a host that serves one flat list of names, as a GitHub release's downloads: `https://github.com/<owner>/<repo>/releases/latest/download/<name>` answers with a redirect (`302`) to the latest release's file of that name. So every release carries all of its version's files, not only the new ones, and a version whose files were published with an earlier release can't be reached.
- **Redirects** are followed, at most 10, only to `https://` (or `http://` to 127.0.0.1, localhost or [::1], as a channel's own address). A redirect elsewhere fails the request.
- **Ranges:** the launcher asks for part of a bundle with `Range: bytes=A-B`, at updates. The server must answer `206 Partial Content` with that `Content-Range`. Whole files and bundles are asked for with a plain GET.

Publishing a version: upload the files that aren't there yet, then the signed manifest. In a flat channel on GitHub, both go up with the release itself (below).

**openm2's own Windows release** is a flat channel: `deploy/windows/launcher.toml` names the repository's latest release, the channel `windows-x64` and the key its manifests are signed with, and the release's zip carries that file beside the programs. `.github/workflows/release.yml` writes the manifest of the version's files (`openm2.exe`, `openm2-launcher.exe`, the licences, the third-party notices and the README) with `release manifest --objects` for a year, signs it with `OPENM2_RELEASE_KEY` (a secret of the repository's `release` environment), checks it with the committed key, and uploads `windows-x64.manifest.signed` and every `<sha256>` with the zip. The channel carries the programs only: the player's client is imported on their machine, never downloaded.

**Its setup,** `openm2-setup.exe` (`deploy/windows/setup.iss`, Inno Setup 6), installs what the zip holds for the player alone, with no administrator, into `%LOCALAPPDATA%\Programs\openm2` by default: a folder the launcher can update in place. It adds a Start menu shortcut, a desktop one if the player asks, and an uninstaller, and can start the launcher at its end. The uninstaller takes its own files and what the launcher makes (the install folder's table, below): `app/`, `previous/`, `staging/`, `imported/`, `importing/`, `client.toml`, `settings.toml`, `servers.toml`, the logs, the lock and `openm2-launcher.old`; the player's own client folder is never touched. `release.yml` builds it from the zip's folder and tries it before it publishes: a silent install, its files, and a silent uninstall that leaves nothing. Both the zip and the setup are named without their version, so `releases/latest/download/openm2-setup.exe` is always the latest.

## `openm2 release`
- **`keygen --out FILE`** writes a new private key to `FILE`, which must not exist yet (readable only by its owner on Unix), and prints the public key as `launcher.toml` takes it.
- **`manifest --dir DIR --channel NAME --version TEXT --out FILE`** writes the manifest of every file under `DIR`:
  - `--sequence N` (default: the time now, in seconds since 1970);
  - `--days N`: how long it counts (default 90);
  - `--objects OUT` also copies each file to `OUT/<sha256>`, ready to upload as `files/`: with `--bundles`, the bundles and the files on their own, not the bundled files;
  - `--bundles` puts every file under 16 MiB into bundles of at most 16 MiB: each content once, at its first path, in the order of the paths, and a new bundle when the next content wouldn't fit. Empty files and larger files stay on their own. The same folder always gives the same bundles, and each file is checked again as its bytes go in;
  - `--add FILE` (again for more) adds the files and bundles of a manifest written elsewhere, as `release manifest` writes it: for a version whose files are made in more than one place. Its channel, sequence, version and expiry don't count, its files are neither read nor copied, and paths must still be unique;
  - `--allow-imported` lets through files an import marked (`openm2 assets import` and `content import`, `assets.md`, "Imported files"), which it otherwise refuses, naming them: only for a channel on the player's own machine (`http://127.0.0.1`), to try the launcher at a whole client's size. Players' channels never carry them.

  Any path that breaks the rules above stops it.
- **`sign FILE --out FILE`** checks the manifest, signs it with `OPENM2_RELEASE_KEY` and writes the signed file; it prints the public key it signed with.
- **`verify FILE --key ed25519:…`** checks a signed manifest as the launcher does: its signature, its rules, its channel (`--channel NAME`, else the one it names) and its expiry. It prints the channel, sequence, version, expiry, the number of files and their total size and the number of bundles, and why it's refused when it is. Its exit code is 0 only when everything holds.

## The install folder
| Name | What |
|---|---|
| `openm2-launcher` (`.exe`) | what players start |
| `launcher.toml` | from the download (below) |
| `client.toml` | the player's client folder and keys file, written at the first start (below) |
| `settings.toml` | the player's settings in the game (volumes, the interface's size, the names over characters), which the game reads and writes ([`client.md`](client.md), "The player's settings"); updates never touch it |
| `servers.toml` | the player's servers on the game's Servers board, beside `settings.toml`, which the game reads and writes ([`client.md`](client.md), "The player's servers"); updates never touch it |
| `imported/` | the player's client imported: an asset folder ([`assets.md`](assets.md)) the launcher makes with the version's own game; updates never touch it, channels never carry it |
| `importing/` | the next import while it's made, deleted at the next start when an import stopped part-way |
| `import.log` | what the last import and its check reported |
| `launcher.log` | what the launcher said on its last start, in the window or on the console, emptied once it holds the lock |
| `game.log` | what `play` wrote, when the window started it, emptied at each start |
| `launcher.lock` | held while a launcher runs; a second one stops |
| `app/` | the version in use: its manifest's files, and `.manifest`, the signed manifest it came from; `window.toml` among them when the version brings the window's look (below) |
| `previous/` | the version before the last update, the same way |
| `staging/` | the next version while it's being fetched |
| `openm2-launcher.old` | the launcher an update replaced, deleted at the next start |

- **`launcher.toml`**, from the game's download; the launcher never changes it:

  | Key | What |
  |---|---|
  | `format` | `1` |
  | `url` | the channel's address: `https://`, no query or fragment (`http://` only to `127.0.0.1`, `localhost` or `[::1]`) |
  | `channel` | the channel's name |
  | `layout` | how the channel's server names its files, `folders` (the default) or `flat` (On the server, above); optional |
  | `key` | the public key manifests must be signed with |
  | `[window]` | how the window looks, optional, and so is each key in it: `name` (the title, and the large text without a picture; 1 to 64 characters), `picture` (a PNG file beside `launcher.toml`, 4 : 1, drawn at 640 × 160), `accent` (the gauge and the main button) and `background`, as `#rrggbb`. Without them the window is openm2's own. A picture that won't read is noted in `launcher.log`, and the window shows without it |

  Changing the address or the key needs a new download.
- **A version's `window.toml`**, when the channel's version has one: `format = 1` and the keys of `[window]`, its `picture` a path in the version's folder. The window draws with the installed version's look first, then `launcher.toml`'s, then its own; one that won't read is noted in `launcher.log`, and the next is used. The window opens before the update, so a new look shows from the start after the update that brings it; `launcher.toml`'s `[window]` is what a first start shows.
- **`client.toml`:** `format = 1`, `client` (the player's client folder) and `keys` (their keys file, `formats/keys.md`). Relative paths start from the install folder. They are saved once the client is imported (7), so they name the client `imported/` came from.
- **Unknown keys** are refused in both, naming the file.

## What the launcher does
1. **Takes the lock** (`launcher.lock`), deletes `openm2-launcher.old` and `importing/` (an import that stopped part-way), and puts `previous/` back as `app/` when `app/` is missing (an update stopped between its two renames).
2. **Fetches the channel's signed manifest** (unless `--no-update`): at most 8 MiB plus its line, 20 s to connect, 60 s for any read.
3. **Checks it** with `launcher.toml`'s key and channel, and the time now:
   - **the same sequence as `app/.manifest`:** nothing to do;
   - **a higher one with the same files** (paths, sizes and SHA-256s, bundled or not): only `app/.manifest` is replaced;
   - **a higher one:** an update (4);
   - **a lower one, another channel, an expired one, or a bad signature:** refused.

   When the update is refused or fails, the launcher says why and starts the installed version, if there is one. With `--update-only` it stops with exit code 1 instead.
4. **Builds `staging/`:**
   - **A file already in `staging/`** with its size and SHA-256 is kept (from an update that stopped part-way).
   - **Otherwise one in `app/` or `previous/`** with the same SHA-256, by their manifests, is linked (a hard link, or a copy where links fail).
   - **Otherwise it's fetched, each content once:** the version's other files with that content are linked to it.
     - **A file on its own** comes from `files/<sha256>`.
     - **Bundled files come by bundle:** files next to each other, or less than 256 KiB apart, in one request, the bytes between them read and dropped. That is the whole bundle with a plain GET when it is all of it (a first install), else a range. A server that sends the whole file for a range fails the update: it doesn't answer ranges.
     - **8 requests at once,** on threads that share the HTTP client's connections.
     - **Each file's bytes go into `<path>.part`** as they come, never more than its size; it is flushed, checked and renamed. No bundle is kept on disk.
     - **After the first failure no more requests start.** Those under way finish, so what they brought is kept, and the next start asks only for the rest.
     - **It says how far it has got:** the window's gauge follows every byte read, at most ten times a second; the console says how many files and bytes it fetches in how many requests, then every 2 seconds how far it has got.
   - **Every file is checked** for its size and SHA-256 before it counts; a linked file that fails is fetched instead.
   - **On Unix, the game and the launcher** (`openm2`, `openm2-launcher`) are made executable: a manifest carries no modes.
   - **Anything in `staging/`** that isn't one of the manifest's files is deleted, and `.manifest` is written last.
5. **Swaps:**
   - deletes `previous/`;
   - renames `app/` to `previous/`;
   - renames `staging/` to `app/`, or `previous/` back to `app/` when that fails.

   A running game keeps `app/` from being renamed: the launcher says to close it, and `staging/` waits for the next start.
6. **Replaces itself,** when it runs from the install folder and `app/` holds an `openm2-launcher` (`.exe`) whose SHA-256 differs from its own: it renames itself to `openm2-launcher.old` and copies the new one in its place, renaming itself back if that fails. The new launcher runs from the next start.
7. **Imports the player's client**, unless the version plays with its own files:
   - **It asks the game** whether `imported/` holds its import: `app/openm2 assets current imported` ([`assets.md`](assets.md)), which reads only `assets.toml` and answers at once.
   - **It imports** when it doesn't (no import yet, or another import's number: an update changed the import), when `--client` or `--keys` name another client folder or keys file than `client.toml`, or with `--import`. It asks for what `client.toml` and the flags don't name (the flags' paths from the folder the launcher was started in): in the window, two fields filled by typing, pasting (Ctrl+V) or dropping the folder and the file on it, or on Windows with a Browse button beside each, which opens the system's own dialog for a folder or a `.toml` file over the window, starting where the field points; checked to be a folder and a file; on the console, a line each. It says why it imports (no import yet; another client folder or keys file; or, after an update, that this version reads clients differently, with the import's numbers), and runs `app/openm2 assets import --client DIR --keys FILE --out importing` (the lines it writes show as it goes, the last one in the window; its report goes to `import.log`, and its first line is said), then `app/openm2 assets check importing` (its report too). Closing the window stops them. Only when both pass it deletes `imported/`, renames `importing/` to `imported/` and saves the client folder and keys file in `client.toml`. A failed import or check deletes `importing/`, leaves `imported/` and `client.toml` as they were, and says why.
   - **A game that doesn't import** (it refuses `assets current` as a command it doesn't know, with exit code 2: openm2 0.2.1 and earlier, which `--previous` may start) plays with the client folder itself, as before: the launcher asks for it as above, checks it with `app/openm2 vfs check` and saves it in `client.toml` when the check passes; `--import` is noted as unused.
   - **Not when the version plays with its own files:** when `app/openm2.toml` names `assets` (an asset folder the channel carries; the launcher reads only that key of it), nothing is asked or imported, and `--client`, `--keys` and `--import` are noted as unused.

   With a TMP4 client on a laptop with 20 threads, the first start takes 43 seconds to the login screen and 2.2 GB more on disk, next to the client's own 1.7 GB; the next start, 1.6 seconds.
8. **Starts `app/openm2 play --assets DIR`** with `imported/`'s absolute path, or `app/openm2 play --client DIR --keys FILE` for a game that doesn't import, or `app/openm2 play` alone for a version that names `assets`, with any arguments after `--`, in the install folder (relative paths in those arguments start there). `app/openm2.toml` (from the channel) gives the server, its key, the provider and the asset folder. `OPENM2_SETTINGS` names the install folder's `settings.toml` (its absolute path) to the game; a game before it doesn't read it, so `--previous` starts one as before. The launcher waits and exits with the game's exit code.
   - **From the window,** the player clicks Play. The game's output goes to `game.log`, and the window goes while it runs. A game that stops with an error brings the window back with its exit code and the end of `game.log`.
   - **On the console,** the game starts at once and shares the console.

- **`--previous`** starts `previous/`'s version instead, without updating: for when an update doesn't start. Its game says whether `imported/` is its import, as in 7.
- **`--import`** imports the client again before playing, even when `imported/` is the game's import.
- **The first start after an update from 0.2.1 or earlier** fails once: the old launcher runs it, and passes `play --client`, which the new game refuses (exit code 2: `play` reads only asset folders). By then the old launcher has put the new one in its place (6), and the next start imports (7).
- **`--dir DIR`** uses another install folder than the launcher's own.

## The window and the console
- **The window** is what a player sees: 640 × 400 logical pixels, drawn with `m2-ui` and `m2-render` in flat colours and the system's fonts, with the adapter that uses less power, or a software one when that fails.
  - **What it shows:** the game's picture or name; what the launcher does, with a gauge; then **Play** and **Close**: the installed version, the latest.
  - **When something fails** (the update, the game's start, or the game itself): what failed and why, with **Try again**, **Play** when the installed version can start anyway, and **Play the previous version** when `previous/` has a game (`--previous`'s way).
- **Release builds have no console** (on Windows a Windows-subsystem program), so the game's programs it starts (`play`, `assets current`, `assets import`, `assets check`, `vfs check`) start without a console window (`CREATE_NO_WINDOW`), their output going where the launcher sends it. Debug builds keep a console.
- **The modes:**

  | Started with | What it does |
  |---|---|
  | nothing | the window |
  | `--update-only` | the console's way, never a window: steps 1 to 6, then it stops |
  | `--no-window` | the console's way: everything said and asked on the console |
  | no window can be made (no display, no adapter at all) | the console's way; on Windows, if that fails too, `launcher.log` opens in Notepad |

- **The channel's address never shows its path**: `launcher.log`, the console's lines and the window's errors write its scheme and host, then `…`, as the path may be a player's secret link.
- **The console's lines** are the same in both console modes, for scripts. A Windows release build started from an interactive cmd or PowerShell writes them only when its output is redirected to a file or piped to another command (it can't attach to that console; PowerShell's `$x = & …` alone gets nothing); Git Bash pipes it. `launcher.log` holds them always.
