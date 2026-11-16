---
name: add-format-parser
description: Add or fix a parser for one of the client's game file formats in openm2 (m2-formats/m2-assets) - facts and source, spec, reader and writer, byte-exact and property tests, CLI wiring. Use when asked to read, parse, decode, support or fix a game file format (archives, proto tables, maps, models, scripts…).
---

# Add a file format parser

Read `crates/m2-formats/AGENTS.md` and `docs/source-policy.md` first. Real game files are only on the human's machine: you work from facts and synthetic data, and the human checks the real files at the end.

## 1. Get the facts
You need the byte layout:
- magic;
- version and record size, if any;
- every field's offset, size, type and byte order;
- what is compressed or encrypted, and with which key;
- strings: their field size, and whether the size includes the NUL.

Sources, facts only, in your own words:
- Quantum Core X (MPL-2.0), if the server reads the format.
- Permissively licensed projects that were not built from leaked sources, e.g. LSLib for GR2.
- Community documentation. Much of it comes from leaked sources: take facts, never text or code.
- The human's own files, examined on their machine (`openm2 pack cat`, a hex dump in `local/`).

Write down where each fact came from. If something is uncertain, say so in the spec and add it to `docs/roadmap.md` → Open questions. Never guess silently.

## 2. Spec
Add or extend a file in `docs/specs/formats/`, including:
- an offset table per structure;
- the reading rules: limits, what is refused, what is ignored;
- a **Verified against** line;
- a **Code** line.

Link the file from `docs/specs/formats/README.md`.

## 3. Reader and writer
In `crates/m2-formats/src/<format>.rs`:
- **Parse with `Reader`.** It checks bounds and names the structure in errors.
  - Check counts against `r.remaining()` before allocating.
  - Cap header sizes.
  - Use `FormatError` variants: `UnknownLayout` for versions and record sizes we do not know, `MissingKey`/`WrongKey` for keys.
- **Write with `Writer`,** producing exactly the same layout. Strings that do not fit are errors (`StringTooLong`), never cut.
- **Reuse** `container::decode`/`encode` for `MCOZ` data, `RawStr` for strings, and `Keys`/`KeyName` for keys. A new key slot needs a `KeyName` variant, a `keys.toml` field (`crates/m2-assets/src/keys.rs`) and a line in `docs/specs/formats/keys.md`.
- **Export** the main types in `lib.rs`.

## 4. Tests
In `crates/m2-formats/tests/<format>.rs`:
- **Byte-exact:** build the bytes by hand, field by field, with a comment per field and `assert_eq!(b.len(), <offset>)` at the spec's offsets. Check that the writer produces them and the parser reads them back to the expected struct.
- **Round trip** with our test keys (`tests/common/mod.rs`) if the format is encrypted.
- **Errors:** truncation, bad magic, unknown layout, missing or wrong key, sizes that disagree.
- **Property tests:**
  - random bytes never panic;
  - a valid encoding with a few bytes flipped never panics.

Run `cargo test -p m2-formats`.

## 5. Wire it up
- If the files live in archives, read them through `m2_assets::Vfs`. Game paths that touch the disk go through `safe_relative_path`.
- Add an `openm2` command to `crates/m2-cli/src/files.rs`, or extend one (`proto dump` prints JSON Lines; follow it).
- Add an end-to-end test to `crates/m2-cli/tests/game_files.rs` that builds a synthetic client.
- If `vfs check` should cover the format, add it there.
- Update `AGENTS.md` (commands table) and `docs/glossary.md` for new terms.

## 6. Check and hand off
```sh
just check
```
Then:
- Update `docs/roadmap.md` (remove what is done, add open questions).
- Stage by path and propose a commit message (git rules in `AGENTS.md`).
- In the handoff, ask the human to run the new command, or `cargo test -p m2-assets --test real_client` with `OPENM2_CLIENT_DIR`, on their client. Until they have, the spec stays "not yet verified".
