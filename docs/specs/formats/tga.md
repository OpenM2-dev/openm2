# TGA images

Most of the client's interface art (boards, bars, gauges, icons) and a few model textures are Truevision TGA files (`.tga`): an 18-byte header, an optional image id and colour map, then the pixels, plain or run-length encoded.

**Sources:**
- The Truevision TGA file format specification, version 2.0 (Truevision, 1991), a public description of the format. Facts only.
- The client's own files, for which layouts occur and how their alpha is meant.

**Verified against:** a TMP4 40250 client (`openm2 tga check`).
- **Files:** all 4,022 TGA files decode.
- **Layouts:** all are true colour: 32-bit with 8 alpha bits 2,669 (plus 654 run-length encoded); 32-bit announcing no alpha bits 393; 16-bit with 1 alpha bit 249; 24-bit 57 (36 of them stored top down). No colour-mapped or grey files, no right-to-left rows.
- **Sizes:** up to 512 × 512.
- **Alpha:** 3,939 files have pixels that are not opaque. Of the 393 32-bit files whose descriptor announces no alpha bits, 391 have alpha that varies, so the alpha is real (below).

**Code:** `crates/m2-formats/src/tga.rs` (reader, `write` and `encode` for tests).

## File layout
All integers are little-endian.

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | id length: bytes of image id after the header |
| 1 | 1 | colour map type: 0 none, 1 present |
| 2 | 1 | image type: 1 colour-mapped, 2 true colour, 3 grey; +8 run-length encoded (9, 10, 11); 0 no image |
| 3 | 2 | colour map: index of the first entry |
| 5 | 2 | colour map: number of entries |
| 7 | 1 | colour map: bits per entry (15, 16, 24, 32) |
| 8 | 2 | x origin (not used for decoding) |
| 10 | 2 | y origin (not used for decoding) |
| 12 | 2 | width |
| 14 | 2 | height |
| 16 | 1 | bits per pixel: 8, 15, 16, 24 or 32 |
| 17 | 1 | descriptor: bits 0–3 alpha bits per pixel; bit 4 rows right to left; bit 5 rows top to bottom |
| 18 | | image id, colour map entries, then the pixels |

A TGA 2.0 footer (`TRUEVISION-XFILE.` and extension areas) may follow the pixels; it is ignored.

## Pixels
- **Order:** rows run from the bottom up unless descriptor bit 5 is set; pixels run left to right unless bit 4 is set. openm2 turns every image to rows from the top, left to right.
- **True colour:** little-endian B, G, R (24-bit) or B, G, R, A (32-bit); 15- and 16-bit pixels are `A RRRRR GGGGG BBBBB` in a little-endian `u16`, each 5-bit channel widened to 8 bits by repeating its top bits.
- **Grey:** one byte of grey, or grey then alpha (16-bit).
- **Colour-mapped:** each pixel is an index (8 or 16 bits) into the colour map, counted from the map's first index; an index outside the map is an error.

## Run-length packets
Each packet starts with a byte: if its top bit is set, the next pixel value repeats (low 7 bits + 1) times; otherwise (low 7 bits + 1) raw pixel values follow. Packets may run on from one row into the next. A packet that runs past the last pixel is cut there; a file that ends before the last pixel is an error.

## Alpha
- **32-bit pixels always carry alpha,** whatever the descriptor says: the client has 393 such files announcing no alpha bits, and the alpha of all but two of them varies.
- **16-bit pixels** use their top bit as alpha (0 transparent) only if the descriptor announces an alpha bit; otherwise they are opaque.
- 24-bit and 8-bit pixels are opaque, grey 16-bit pixels use their second byte only with alpha bits announced.

## Limits
- An image of more than 8192 × 8192 pixels is refused before anything is allocated (`image::MAX_PIXELS`).
- Image type 0, colour map types other than 0 and 1, and depths that do not fit the image type are refused as unsupported.

## Writer (tests)
`tga::write` puts a parsed file back together field by field; `tga::encode` stores an `RgbaImage` as 24- or 32-bit true colour, bottom up or top down, plain or run-length encoded (repeats of 2 or more become runs, packets never cross rows).
