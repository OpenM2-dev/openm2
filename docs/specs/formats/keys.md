# `keys.toml`: the user's keys

The client encrypts some archive indexes, some archive entries and the proto tables with XTEA ([compression.md](compression.md)). The keys are part of the user's client, so **the user supplies them**. openm2 never ships keys, and `just no-assets` refuses a `keys.toml` anywhere outside a `fixtures/` folder.

**Code:** `crates/m2-assets/src/keys.rs` (file), `crates/m2-formats/src/keys.rs` (`Keys`, `KeyName`).

## Format
```toml
# Keys from your own client: four 32-bit numbers each, in the client's order.
[eterpack]
index = [0x00000000, 0x00000000, 0x00000000, 0x00000000]    # encrypted .eix files
entries = [0x00000000, 0x00000000, 0x00000000, 0x00000000]  # entries of type 2

[proto]
item = [0x00000000, 0x00000000, 0x00000000, 0x00000000]     # item_proto
mob = [0x00000000, 0x00000000, 0x00000000, 0x00000000]      # mob_proto
```

- **Values.** Every key is optional. A key is 4 whole numbers from 0 to `0xFFFFFFFF`; decimal or `0x` hex both work.
- **Unknown names are errors,** so a typo (`[eterpak]`) is caught.
- **Missing keys.** A structure that needs a missing key fails with an error that names the key to add: `eterpack.index`, `eterpack.entries`, `proto.item` or `proto.mob`.
- **Wrong keys.** The error says the key is wrong, because decryption did not give the `MCOZ` magic.

## Where it lives
- **Choosing the file.** Commands take `--keys <file>`, or read the path from `OPENM2_KEYS`.
- **Where to keep it.** Inside the repo, keep it in `local/` (ignored by git): `local/keys.toml`.
- **Finding the keys.** The keys are in your client, in its executable. The proto keys are also in Quantum Core X's source (its item and monster managers), because the server reads the same tables.
- **Never shown.** openm2 never prints a key: `Keys` and `XteaKey` show only which keys are set, and errors about `keys.toml` give the line, never its text.
