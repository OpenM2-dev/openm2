# EterPack archives

An archive is two files with the same name: `<name>.eix`, the index, and `<name>.epk`, the data. The TMP4 client keeps them in its `pack/` folder.

**Verified against:**
- A TMP4 40250 client: 107 archives and 55,187 entries, all decoded with `openm2 vfs check`.
- Community documentation for the layout (facts only).
- Where the two disagreed, the client won. The size fields and the data CRC below were corrected that way.

**Code:**
- Parsing: `crates/m2-formats/src/eterpack.rs`.
- Files on disk: `crates/m2-assets/src/archive.rs`, `vfs.rs`, `path.rs`, `extract.rs`.

## Index (`.eix`)
All integers are little-endian. The file is in one of two forms:
- **Plain:** magic `EPKD`, version u32 (= 2), entry count u32, then the entries.
- **Wrapped:** an `MCOZ` container ([compression.md](compression.md)), encrypted with the `eterpack.index` key, holding the plain index.

Before the entries are read, the count is checked against the bytes left.

### Entry (192 bytes)
| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 4 | id | Position in the index |
| 4 | 161 | name | NUL-terminated, at most 160 bytes |
| 165 | 3 | padding | |
| 168 | 4 | name CRC | CRC-32 of the name |
| 172 | 4 | size A | See "Sizes" below |
| 176 | 4 | size B | See "Sizes" below |
| 180 | 4 | data CRC | See "CRCs" below; not reliable |
| 184 | 4 | offset | Where the stored bytes start in the `.epk` |
| 188 | 1 | type | See below |
| 189 | 3 | padding | |

### Entry types
| Type | Name in openm2 | Stored as | openm2 |
|---|---|---|---|
| 0 | `stored` | The file's bytes | Reads it |
| 1 | `lzo` | `MCOZ` container, not encrypted | Reads it |
| 2 | `lzo+xtea` | `MCOZ` container encrypted with the `eterpack.entries` key | Reads it |
| 3 | `panama` | Encrypted with a key the server sends at login | Refuses, naming the type |
| 4 | `hybrid` | Encrypted with keys the server sends at login | Refuses |
| 5 | `hybrid+sdb` | Like 4, with extra data from the server | Refuses |
| other | `unknown` | — | Refuses |

Types 3–5 exist for servers that send archive keys during login. QCX does not, so a client made for it should not need them. The TMP4 client has none: 20,730 entries are type 0, 30,222 type 1 and 4,405 type 2. `openm2 vfs check` counts them if they turn up.

### Sizes
One size field is the length of the entry's data in the `.epk`; the other is the space reserved for it, often padded. The tools that packed the TMP4 client disagree on which field is which: some archives have the data length first, others second. So openm2 does not trust either field by name:
- **Stored entries (type 0):** the data is the **smaller** of the two sizes (`Entry::listed_len`). This holds for every stored entry in the client.
- **Container entries (types 1 and 2):** the data is as long as the container's own header says (header + encrypted payload, or header + inner magic + stream; `container::Header::total_len`). The index sizes are ignored.
  - In one patch archive, 68 entries list both sizes 13–16 bytes shorter than their container (the packer apparently left out the 16-byte header).
  - The container still ends exactly where the next entry starts, so trusting the header is right.
- **Encrypted containers** are usually listed 4 bytes longer than the container: the 4 extra bytes after the payload ([compression.md](compression.md)).
- The decoded size is not in the index at all: it is the container's real size.

### Names
- **Form.** Names are stored normalized: ASCII letters lowercase, `/` as separator.
- **Drive prefix.** Many names start with a drive prefix such as `d:/ymir work/…` (the path on the original developers' machines). Others do not (`locale/…`).
- **Lookups** normalize the same way, so `D:\Ymir Work\PC\A.GR2` finds `d:/ymir work/pc/a.gr2`.
- **Non-ASCII bytes** stay as they are; names may contain EUC-KR. Tools print them escaped (`\xc7`) and accept the same escapes back.
- **Skipped and repeated names.** Entries with an empty name are skipped. If a name repeats within one index, the first entry wins and the repeats are counted.

### CRCs
- **Algorithm:** CRC-32 as in zlib (`crc32(b"123456789") == 0xCBF43926`).
- **Name CRC:** computed over the stored name bytes. It matched all 55,187 entries of the client.
- **Data CRC:** computed over the first [smaller size] bytes of data. It matched about 95% of entries; whole archives (`ETC`, `patch2`) and a few patch entries match nothing.
  - So openm2 never checks it when reading.
  - `openm2 vfs check` reports how many match, without failing.

## Reading (`m2-assets`)
- **Size limits.** An `.eix` may be at most 64 MiB, and one stored entry at most 256 MiB.
- **How much is read.** `Archive::read_stored` reads the first 16 bytes at the entry's offset, works out the data length by the rules under "Sizes" (`Entry::data_len`), then reads that many bytes.
- **Bounds.** An entry whose data runs past the end of the `.epk` is refused (`OutOfBounds`). A size field that points past the end does not matter if the data itself fits.
- **Positional reads.** The `.epk` stays open and is read at offsets, so one `Archive` can serve several threads.
- **Finding the data file.** The `.epk` is the `.eix` path with extension `epk`, or `EPK` if that is what exists.

## The VFS
`m2_assets::Vfs` looks files up by game path:
1. **Loose folders,** in the order they were added. A loose file overrides the archives. The game path is mapped to a file the same way as for extraction (below), so a folder written by `openm2 pack extract` can be mounted as loose files.
2. **Archives.** `Vfs::open_client(dir)` mounts every `pack/*.eix`:
   - first the archives named in the client's archive list (`pack/Index`, below), in its order;
   - then the rest, sorted by file name.

   If two archives hold the same name, the one mounted first wins and the clash is listed (`Vfs::shadowed`, `vfs check`). In the TMP4 client 2,375 names are in more than one archive, so the order matters.

### Archive list (`pack/Index`)
A text file in the client's `pack/` folder. The code is in `m2_formats::pack_list`.

```text
PACK
*
patch1
d:/ymir work/npc/
NPC
```

- **Lines.** The first line is `PACK`. Then come pairs of lines: a path prefix (`*` means any path), then an archive name without extension. Lines end in CRLF; openm2 also accepts LF and skips blank lines.
- **Order.** The TMP4 client lists 124 archives:
  - patches first (`patch1`, `season3_eu`, `patch2`, `metin2_patch_*`), most with the prefix `*` or `pack/`;
  - then the base archives, with the folders they hold (`d:/ymir work/npc/` → `NPC`).
  - Some listed archives do not exist (other locales). Six existing ones are not listed, among them `root`, which the client presumably opens on its own.
- **How openm2 uses it:**
  - It uses only the order: the first listed archive wins.
  - That is inferred: patches are listed first, and patches exist to override. It has not been confirmed from the client's behaviour.
  - What the prefix means for lookups is not known (`docs/roadmap.md`, "Open questions").

## Names on disk
Archive names are untrusted. `m2_assets::safe_relative_path` turns a name into a relative path that is safe to join to a folder, or refuses it.

**Mapping:**
1. Normalize the name, then drop a drive prefix: `d:/ymir work/a.gr2` → `ymir work/a.gr2`.
2. Skip empty components (`a//b` → `a/b`).

**Refused** (the entry is skipped and reported):
- absolute paths (a leading `/`, including `\\server\…`);
- `.` and `..` components;
- control characters, including NUL;
- names that end in `.` or a space;
- Windows device names (`con`, `prn`, `aux`, `nul`, `com0`–`com9`, `lpt0`–`lpt9`, `conin$`, `conout$`), with or without an extension;
- components longer than 255 bytes, and empty paths.

**Escaped** as `%XX`: `%`, `<`, `>`, `:`, `"`, `|`, `?`, `*` and every byte outside ASCII. This keeps the mapping one-to-one and the same on every OS.

**Extraction** (`m2_assets::extract`, `openm2 pack extract`):
- Folders are created one level at a time.
- If something other than a folder exists where a folder should be (a file or a link), the extraction stops. Nothing is written through a link.
- Files are created new: an existing file stops the extraction, so extract into an empty folder.
- Entries that cannot be decoded are skipped and reported.

Loose-file lookups use the same mapping. A game path that is refused is looked up only in the archives, never on disk.
