# MCOZ containers, LZO1X and XTEA

The client stores most data compressed with LZO1X, and some of it also encrypted with XTEA, inside a small container with the magic `MCOZ`. Archive entries of types 1 and 2 ([eterpack.md](eterpack.md)), encrypted archive indexes, and the proto tables ([proto.md](proto.md)) all use it.

**Verified against:**
- A TMP4 40250 client: every container in its 107 archives decodes (the 107 encrypted indexes, about 30,000 plain and 4,400 encrypted entries), and so do its proto tables.
- Quantum Core X `007c7a6`, its proto table loader: container header, inner magic, XTEA decryption.
- Community documentation, for the LZO1X bitstream.

**Code:** `crates/m2-formats/src/container.rs`, `lzo.rs`, `xtea.rs`.

## Container
All integers are little-endian.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `MCOZ` |
| 4 | 4 | encrypted size: bytes of encrypted payload, or 0 if not encrypted |
| 8 | 4 | compressed size: bytes of the LZO1X stream |
| 12 | 4 | real size: bytes after decompression |
| 16 | … | payload |

**Payload:**
- The payload is the magic `MCOZ` again, then the LZO1X stream (compressed size bytes).
- **Encrypted** (encrypted size > 0): the payload is that many bytes, XTEA-encrypted in 8-byte blocks with the key the structure needs. After decryption it must start with `MCOZ`; anything else means the key is wrong.
- **Not encrypted:** the payload follows the header directly, also starting with `MCOZ` (confirmed on the client's archive entries).
- **Length:** a container is self-delimiting. It is 16 bytes of header plus the encrypted size, or plus 4 + the compressed size if it is not encrypted (`Header::total_len`). Archive entries rely on this ([eterpack.md](eterpack.md), "Sizes").

**Reading rules (openm2):**
- Bytes after the payload are ignored. Encrypted containers in the client usually have 4 extra bytes there. Archive indexes and proto headers count them in their sizes; QCX's test tables have them too.
- A real size over 256 MiB is refused before anything is allocated.
- The decompressed length must equal the real size exactly.
- `MCSP` instead of `MCOZ` marks a Snappy container, used by later clients. It is reported as unsupported until one turns up in a TMP4 client.

**Our writer** (`container::encode`, for test data) stores the data as one LZO1X literal run. It pads the encrypted payload with zeros to a multiple of 8. Real files pad differently, and the reader does not depend on how.

## LZO1X
A byte-oriented LZ77 format: instructions either copy *literals* (bytes from the stream) or copy a *match* (bytes already written, some distance back).

The meaning of instructions 0–15 depends on the **state**: how many literals the previous instruction copied (0; 1 to 3; or 4, meaning 4 or more).

| Bytes | Meaning |
|---|---|
| First byte 18–255 | Copy (byte − 17) literals. State = that count (4 if 4 or more). Only at the very start. |
| `0000LLLL`, state 0 | Copy L + 3 literals; L = 0 means an extended length from 15. State 4. |
| `0000DDSS` + H, state 1–3 | Match of 2 bytes, distance (H × 4) + D + 1. |
| `0000DDSS` + H, state 4 | Match of 3 bytes, distance (H × 4) + D + 2049. |
| `0001HLLL` + W | Match of L + 2 bytes (L = 0: extended from 7). W is a little-endian u16; distance 16384 + (H × 16384) + (W >> 2), S = W & 3. If (H × 16384) + (W >> 2) is 0, the stream ends. |
| `001LLLLL` + W | Match of L + 2 bytes (L = 0: extended from 31); distance (W >> 2) + 1, S = W & 3. |
| `01LDDDSS` + H | Match of L + 3 bytes, distance (H × 8) + D + 1. |
| `1LLDDDSS` + H | Match of L + 5 bytes, distance (H × 8) + D + 1. |

- H is the next byte.
- **Extended length:** the base, plus 255 for each zero byte, plus the first non-zero byte.
- **After every match**, S literals (0–3) follow, and S becomes the state. For the short forms, S is the instruction's low two bits.
- **Overlap:** a match may overlap the bytes it is writing (distance < length repeats a pattern).
- **End of stream:** the stream ends with `11 00 00`.

Source of the bitstream facts: the LZO1X description in the Linux kernel (`Documentation/staging/lzo.rst`) and the lzokay project (MIT). Facts only; the decoder was written from this table.

**Decoder rules (openm2):**
- Every length and distance is checked against the input left, the bytes written so far and the expected size.
- Output never exceeds the expected size.
- Bytes after the end marker are ignored.
- Tests compare the decoder with an independent encoder, lzokay-native (MIT), used only as a test dependency.

## XTEA
Standard XTEA:
- 64-bit blocks, read as two little-endian u32 words;
- a 128-bit key of four u32 words;
- 32 cycles with delta `0x9E3779B9`.

Data is processed in whole 8-byte blocks. A shorter tail is left as it is.

Keys:
- The client has separate keys for archive indexes, encrypted archive entries, `item_proto` and `mob_proto`.
- They are not in this repo: the user supplies them ([keys.md](keys.md)).
- Tests use our own keys and the published XTEA test vectors.
