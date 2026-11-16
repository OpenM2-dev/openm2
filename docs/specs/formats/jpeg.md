# JPEG images

Some of the client's effects name `.jpg` textures with no `.dds` beside them: the glows of upgraded items among them (`pc/common/effect/armor/` and `sword/`). The client also keeps the sphere maps shining items reflect (`special/spheremap.jpg`, `spheremap01.jpg`) and its loading pictures as JPEG files. openm2 reads baseline and progressive JPEG (`openm2 jpeg`, and `openm2 assets import`, which converts them to PNG); the armours' sphere map (`special/spheremap.jpg`) is progressive.

**Sources:**
- ITU-T Recommendation T.81 (1992), *Digital compression and coding of continuous-tone still images*: the public standard, for the markers, the Huffman decoding procedure, the zig-zag order, the inverse DCT and progressive scans (its Annex G). Facts only.
- JFIF 1.02 for the YCbCr to RGB conversion, and Adobe's `APP14` marker's transform byte for files stored as RGB.
- The client's own files, for which layouts occur.

**Verified against:** a TMP4 40250 client (`openm2 jpeg check`).
- **Files:** all 501 JPEG files decode: 452 baseline, 49 progressive (the locales' `ui/login.jpg` and `ui/select.jpg`, loading pictures, `special/spheremap.jpg`).
- **Layouts:** all three components, YCbCr: full-size chroma 427, chroma half as wide 1, half as wide and tall 73.
- **Against another decoder:** 3 baseline and 4 progressive files (the two sphere maps among them) decoded within 3 levels of Pillow's, 0.01 to 0.04 on average (its chroma upsampling is smoother than openm2's nearest sample).
- **Sizes:** up to 1024 × 1024.

**Code:** `crates/m2-formats/src/jpeg.rs` (reader; `encode` and `encode_progressive`, writers for tests).

## What is read
- **Markers:** `SOI`, then segments, each `FF`, the marker and (except `SOI`, `EOI`, `TEM`, `RST0`–`RST7`) a big-endian length that counts itself. Fill bytes (`FF FF`) before a marker are skipped; `APPn`, `COM` and markers openm2 doesn't use are skipped by their length.
- **Frames:** `SOF0` (baseline), `SOF1` (extended sequential, Huffman) and `SOF2` (progressive, Huffman), 8-bit samples, one component (grey) or three, sampling factors 1 to 4, quantization tables 0 to 3. Differential progressive (`SOF6`, `SOF14`), lossless (`SOF3`, …), hierarchical (`SOF5`) and arithmetic-coded (`SOF9`, `SOF10`, `SOF13`) files, 12-bit samples, four components and a height given after the data (`DNL`) are refused by name.
- **Tables:** `DQT` (8- or 16-bit values, in zig-zag order) and `DHT` (16 counts of codes by length, then the values; at most 256), defined anywhere before the scan that uses them, and redefined between scans. `DRI` sets the restart interval.
- **Scans:** `SOS` names its components and their DC and AC tables; a sequential file's scans are whole (`Ss` 0, `Se` 63, no approximation). With one component its blocks run over that component alone, as many as cover it; with several, MCUs of every component's blocks (at most 10). Each block: its DC difference (a size, then that many bits, added to the component's last DC value), its AC run-lengths (`F0` skips 16, `00` ends the block), each value times its quantization step, then the inverse DCT and a level shift of 128. After each restart interval the bits left of the byte are dropped, the `RSTn` marker passed, and the DC values start from 0 again.
- **Progressive scans:** a scan carries the DC of every block of its components (interleaved, as MCUs) or a band of one component's AC coefficients (`Ss` to `Se`, over that component's own blocks), first or refining (`Ah`, `Al`): a first DC scan's values times 2 to the `Al`, a refining one the next bit down; a first AC scan's values times 2 to the `Al`, with runs of empty blocks (the symbols with size 0 and a run of 0 to 14: 2 to the run, plus that many bits, less 1, blocks more); a refining AC scan gives each coefficient already there its bit (away from 0) and brings new ones of plus or minus 2 to the `Al` in their runs of zeros, and empty blocks only refine. Restart intervals reset the DC values and the runs. The coefficients are kept, per component, until the file ends, then dequantized and turned to samples as a baseline file's; such files are read up to 4,096 × 4,096 pixels.
- **Entropy-coded data:** `FF 00` is the byte `FF`; a marker ends the data (what is left decodes as zeros).
- **Colour:** each component is sampled at the pixel's place scaled by its sampling factor over the largest (the nearest sample); grey as it is; YCbCr to RGB by JFIF (R = Y + 1.402 Cr, G = Y − 0.344136 Cb − 0.714136 Cr, B = Y + 1.772 Cb, with Cb and Cr less 128); three components after an Adobe marker of transform 0 as RGB. The image is opaque.

## Limits and damage
Every length is checked against the bytes left, and an image past 8,192 × 8,192 pixels is refused before anything is allocated. A Huffman table with more codes than fit, a code no table has, a scan naming a table or component that isn't there, a block of more than 64 coefficients or a DC size past 11 is a damaged file; a file cut short decodes what it has after its first scan.
