# Oodle1

Oodle1 is the compression of GR2 sections ([gr2.md](gr2.md), compression type 2). It is an LZ77 scheme: the output is built from literal bytes and repeats of earlier output, and every number it needs goes through an adaptive range coder. It is not related to RAD's later "Oodle" product line (Kraken and friends).

**Sources:**
- liboodle's description of Oodle1 and its reference decoder (github.com/LunaticInAHat/liboodle, commit of 2024-01-21; The Unlicense, public domain). Facts only; the code here is our own.
- One correction found on the TMP4 client: when a coder decays (see "Rescale and decay"). liboodle's text and code use a different rule, which breaks 1.4% of the client's files.

**Verified against:** a TMP4 40250 client. All 8,829 format-6 GR2 files decode, with about 26,000 compressed sections. Their CRCs match. Every file then reads as a model ([gr2.md](gr2.md)), and the data makes sense: knots never go backwards, and bone weights add up to 1.

**Code:** `crates/m2-formats/src/oodle1.rs`.

## A section: three streams, one bitstream
A compressed GR2 section holds up to three streams, one after another in the output: the section's 32-bit data, then its 16-bit data, then its 8-bit data. The section header gives the output offsets where the 16-bit and the 8-bit data start (`first16`, `first8`). The section's size ends the third stream.

```
36 bytes: three 12-byte stream headers (stream 0, 1, 2)
then:     one bitstream, read by all three streams in turn
```

Each stream has its own header and starts with fresh coders. The bitstream carries on from where the previous stream stopped. A stream may be empty. Decoding stops at the expected size; there is no end marker.

## Stream header
Three little-endian `u32` words:

| Word | Bits | Field |
|---|---|---|
| 0 | 31–9 | window: how far back a repeat may reach |
| 0 | 8–0 | literal alphabet size (256 for bytes) |
| 1 | 31–19 | largest 1 KiB offset part used |
| 1 | 18–9 | reserved |
| 1 | 8–0 | number of distinct literals |
| 2 | 31–24 | distinct length codes seen by length coders 0–15 |
| 2 | 23–16 | the same for length coders 16–31 |
| 2 | 15–8 | the same for length coders 32–47 |
| 2 | 7–0 | the same for length coders 48–63 |

Length coder 64 gets 0. liboodle's text groups the coders differently (0–15, 16–23, 24–31, 32–64). Its code uses groups of 16, and only that decodes the client's files.

## Bitstream
A range decoder. It keeps a value register `v` and a range `r`, both 32-bit.

- **Start:** `v` = first byte >> 1, `r` = 0x80. The first byte's lowest bit is *held*.
- **Refill:** while `r` ≤ 0x800000, read a byte `b`, then `v = (((v << 1) | held) << 7) | (b >> 1)`, hold `b & 1`, and `r <<= 8`. Each byte's top seven bits go in now and its lowest bit with the next byte. Past the end of the input, bytes read as 0.
- **Peek** a value below `n`: refill; `s = r / n`; the value is `min(v / s, n - 1)`.
- **Consume** the interval `[lo, lo + span)` of `[0, n)`: `v -= lo × s`. Then `r = span × s` if `lo + span < n`, else `r -= lo × s`.
- **Get** a value below `n`, all equally likely: peek it, then consume `[value, value + 1)`.

## Symbol coder
Each coder learns the symbols of one context. It works in fixed point, with 1.0 = 0x4000.

It keeps a table:
- **Index 0** is the escape.
- **Indexes 1 to `active`** each have a symbol, a count and a span of `[0, 0x4000)`, sized by the count at the last rescale.
- **Indexes `active + 1` to `learned`** were learned since the last rescale. They have counts but no span yet, so they are coded as equally likely.

The coder also tracks the total of all counts.

**Start**, for an alphabet of `a` symbols of which `u` are expected:
- escape count 4, total 4;
- next rescale at 8;
- decay threshold `clamp((a − 1) × 32, 256, 15160)`;
- fast step 4;
- step `max(128, min((a − 1) × 2, threshold / 2 − 32))`.

The table has room for `a + 2` entries.

**Decode** a symbol below `n` (`n` is the alphabet size in the current context, which can be smaller than `a`):
1. Rescale if due (below).
2. Peek a value below 0x4000. Find the index whose span holds it, and consume that span. Add 1 to its count and to the total. If it is not the escape, return its symbol.
3. Escape. If any entries are waiting for a span, get a value below 2. If it is 1, get `w` below the number of waiting entries. The symbol is entry `active + 1 + w`: add 2 to its count and the total, and return it.
4. Otherwise it is a new symbol: get it as a value below `n`, append it with count 2, and add 2 to the total. If the table now holds `u` entries, set the escape count to 0 and take it off the total.

**Rescale and decay.** When the total reaches the scheduled point:
1. **Decay** first, but only if *the scheduled point* has reached the decay threshold. The total itself may be past the threshold without a decay. That happens when an escape and a new symbol's bonus (+3) carry the total from just below the point to above the threshold. liboodle checks the total here. On the client, 121 files decode only with the scheduled point. The correction was found by flipping single decay decisions in failing files until they decoded, then checked on all of them.

   Decay works like this:
   - halve the escape count;
   - for each entry: an entry with a count of 1 or less is replaced by the last entry (and the table shrinks), otherwise its count is halved;
   - the entry with the highest count moves to the end of the table;
   - if the table holds fewer than `u` entries and the escape count is 0, the escape count becomes 1;
   - the total is recomputed.
2. **Rescale:** with `q = 0x20000 / total`, each entry's span is `count × q / 8`, laid out from index 0 on. The last entry's span reaches 1.0. All entries now have spans (`active = learned`).
3. **Schedule the next rescale:** if twice the fast step is less than the step, double the fast step and schedule at total + fast step. Otherwise schedule at total + step.

## LZ layer
Per stream, coders for:
- **Literals:** 4 coders, chosen by the stream's output length mod 4. Alphabet and expected count come from the header.
- **Length codes:** 65 coders, chosen by the previous length code. Alphabet 65.
- **Offsets, low part:** one coder; alphabet `min(4, window + 1)`.
- **Offsets, 1 KiB part:** one coder; alphabet `window / 1024 + 1`, expecting the header's largest part + 1.
- **Offsets, 4-byte part:** up to 256 coders, chosen by the 1 KiB part; alphabet `min(256, window / 4 + 1)`.

Each step decodes a length code with the coder of the previous code (0 at the start):
- **Code 0:** a literal, from the literal coder for the current output length mod 4.
- **Codes 1–60:** a repeat of code + 1 bytes (2 to 61). Codes 61–64: 128, 192, 256 and 512 bytes.

A repeat's distance comes in three parts. In the current context, the effective window is `min(window, bytes this stream has output)`.
1. The low part: a value below the low-part alphabet, plus 1 (1 to 4).
2. The 1 KiB part: a value below `effective / 1024 + 1`.
3. The 4-byte part, from the coder of that 1 KiB part: a value below `min(256, effective / 4 + 1)`.

The distance is `1 KiB part × 1024 + 4-byte part × 4 + low part`. The copy may overlap what it writes. It may reach back into an earlier stream of the same section, because all three write one buffer.

**Reading rules (openm2):**
- A repeat that reaches before the start of the section, or past its expected size, is an error. So is a 1 KiB part over 255, a coder learning past its table, or an empty alphabet.
- The stream ends must be in order (`first16` ≤ `first8` ≤ size).
- Nothing allocates more than the section's size, plus each coder's table (its alphabet + 2 entries).

## Our encoder
`oodle1::compress` writes valid sections for tests:
- it matches greedily within each stream (up to 512 bytes, window up to 256 KiB);
- it puts the exact distinct-symbol counts in the headers;
- it codes with the decoder's own symbol coders, so the two cannot drift apart.

Real files compress better, and the decoder does not depend on how a stream was made.
