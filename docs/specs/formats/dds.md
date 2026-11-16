# DDS textures

The client's model, effect and most map textures are DirectDraw Surface files (`.dds`): a short header, then the image's mip levels, largest first. Most are block-compressed (DXT1, DXT3, DXT5); a few store plain pixels described by bit masks.

**Sources:**
- Microsoft's public DirectX documentation of the DDS header and pixel format ("DDS file format", `DDS_HEADER`, `DDS_PIXELFORMAT`).
- The block compression formats as documented by Microsoft ("Block compression", BC1–BC3) and the Khronos Data Format Specification (S3TC). Facts only.

**Verified against:** a TMP4 40250 client (`openm2 dds check`).
- **Files:** all 7,485 DDS files decode, every level.
- **Formats:** DXT1 5,961; DXT3 869; DXT5 194; A1R5G5B5 459; A8R8G8B8 2. No cube maps, volume textures or DX10 headers.
- **Sizes:** up to 2048 × 1024. 5 files are not powers of two.
- **Extra bytes:** 3 files have bytes after the last level.
- **Header values:** the client's textures carry the header flags `0xa1007` (caps, height, width, pixel format, mip count, linear size) and caps `0x401008` (complex, texture, mipmap), as our writer does.

**Code:** `crates/m2-formats/src/dds.rs` (reader, decoders, writer for tests), `image.rs` (`RgbaImage`).

## File layout
All integers are little-endian `u32`.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `DDS ` |
| 4 | 4 | header size, 124 |
| 8 | 4 | flags: 0x1 caps, 0x2 height, 0x4 width, 0x8 pitch, 0x1000 pixel format, 0x20000 mip count, 0x80000 linear size, 0x800000 depth |
| 12 | 4 | height |
| 16 | 4 | width |
| 20 | 4 | pitch (plain formats: bytes per row) or linear size (compressed: bytes of level 0) |
| 24 | 4 | depth (volume textures) |
| 28 | 4 | mip level count |
| 32 | 44 | reserved |
| 76 | 32 | pixel format (below) |
| 108 | 4 | caps: 0x8 complex, 0x1000 texture, 0x400000 mipmap |
| 112 | 4 | caps 2: 0x200 cube map, 0x200000 volume |
| 116 | 12 | caps 3, caps 4, reserved |
| 128 | | level 0, level 1, … without padding |

**Pixel format** (32 bytes):

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | size, 32 |
| 4 | 4 | flags: 0x1 alpha pixels, 0x2 alpha only, 0x4 FourCC, 0x40 RGB, 0x200 YUV, 0x20000 luminance |
| 8 | 4 | FourCC: `DXT1` … `DXT5`, `DX10`, or a number |
| 12 | 4 | bits per pixel (plain formats) |
| 16 | 16 | red, green, blue, alpha bit masks |

## Levels
- **Count:** the mip count field if the mip count flag is set (at least 1), otherwise 1. Never more than a full chain down to 1 × 1.
- **Size:** each level is half the one before, rounded down, at least 1.
- **Bytes per level:**
  - DXT1: 8 bytes per 4 × 4 block;
  - DXT3, DXT5: 16 bytes per block;
  - a level smaller than a block still takes one block;
  - plain formats: `(width × bits + 7) / 8` bytes per row, no row padding.

## Block formats
A block covers 4 × 4 texels, row by row. Texel `i` (0–15, `y × 4 + x`) uses bits from the lowest up.

**DXT1 (BC1), 8 bytes:**

| Offset | Size | Field |
|---|---|---|
| 0 | 2 | colour 0, RGB565 |
| 2 | 2 | colour 1, RGB565 |
| 4 | 4 | 2-bit index per texel |

- RGB565 widens to 8 bits by repeating the top bits: `r8 = r5 << 3 | r5 >> 2`, `g8 = g6 << 2 | g6 >> 4`.
- **Colour 0 greater than colour 1** (compared as `u16`): four colours, entries 2 and 3 at ⅓ and ⅔ from colour 0 to colour 1.
- **Otherwise:** entry 2 is halfway, and entry 3 is transparent black.
- **Rounding:** openm2 rounds interpolated values to the nearest integer; graphics hardware may differ by one.

**DXT3 (BC2), 16 bytes:** 8 bytes of alpha, 4 bits per texel (×17 gives 8 bits), then a DXT1 colour block that always uses four colours. `DXT2` is the same layout with premultiplied colour.

**DXT5 (BC3), 16 bytes:**

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | alpha 0 |
| 1 | 1 | alpha 1 |
| 2 | 6 | 3-bit index per texel |
| 8 | 8 | DXT1 colour block, always four colours |

If alpha 0 > alpha 1, entries 2–7 step evenly from alpha 0 to alpha 1 in sevenths. Otherwise entries 2–5 step in fifths, entry 6 is 0 and entry 7 is 255. `DXT4` is the same layout with premultiplied colour.

## Plain formats
Each channel is the masked bits, shifted down and scaled to 0–255 (rounded to nearest).
- **RGB:** red, green and blue masks; alpha from the alpha mask if the alpha pixels flag is set, otherwise 255.
- **Luminance:** the red mask holds a grey value, used for all three colours.
- **Alpha only:** colour is black.
- 8, 16, 24 and 32 bits per pixel are read.

## Reading rules (openm2)
- **Refused by name:** cube maps, volume textures, the DX10 header, YUV, and other FourCC codes.
- **Limits:** width and height are at most 8192.
- **Short files:** a file too short for level 0 is refused. Later levels it is too short for are left out and counted (`declared_levels`); `dds check` reports such files.
- **Trailing bytes:** bytes after the last level are counted and ignored.

## Alpha in the client's textures
The client's DXT3 character textures (`warrior_4-1.dds` and others) carry a soft gradient in their alpha, mostly low values. It is a mask for a gloss effect, not transparency: used as transparency, it punches holes into armour. DXT1's 1-bit alpha is real coverage (hair edges). The renderer therefore uses alpha as coverage only for DXT1 and 1-bit masks (`docs/specs/render.md`). openm2 draws the gloss as the shine of upgraded items ([`../render.md`](../render.md), "Shine"): the item table's `specular` and the client's sphere map, where the mask says.

## Tools
| Command | What it does |
|---|---|
| `openm2 dds info <file>` | Size, pixel format and levels |
| `openm2 dds png <file> --out x.png [--level N]` | One level as a PNG, to look at (keep it out of the repo) |
| `openm2 dds check --client <dir>` | Decode every level of every DDS file of a client; formats, sizes, oddities |

Each command takes `--client <dir>` to read a game path from the client's archives, with keys from `--keys` or `OPENM2_KEYS`.

**Our writer** (`dds::write`) wraps given level bytes in a header with the client's flags, for tests. Block data is written by hand in the tests.
