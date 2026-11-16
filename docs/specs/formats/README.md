# Game file formats

How openm2 reads the TMP4 "40250" client's files, and glTF, openm2's own format for models (the asset folders that hold it: [`../assets.md`](../assets.md)). The code is in `crates/m2-formats` (parsers and writers, no I/O) and `crates/m2-assets` (files on disk).

| Spec | What | Code |
|---|---|---|
| [compression.md](compression.md) | `MCOZ` containers: LZO1X compression and XTEA encryption | `m2_formats::{container, lzo, xtea}` |
| [eterpack.md](eterpack.md) | Archives (`.eix` index, `.epk` data), entry types, names, CRCs, the VFS, names on disk | `m2_formats::eterpack`, `m2_assets` |
| [proto.md](proto.md) | `item_proto` and `mob_proto` | `m2_formats::proto` |
| [keys.md](keys.md) | `keys.toml`, the user's keys | `m2_formats::keys`, `m2_assets::keys` |
| [gr2.md](gr2.md) | Granny GR2 models, skeletons and animations: container, layouts, curves | `m2_formats::gr2` |
| [oodle1.md](oodle1.md) | Oodle1, the compression of GR2 sections | `m2_formats::oodle1` |
| [dds.md](dds.md) | DDS textures: header, mip levels, DXT1/3/5 and bit-mask formats | `m2_formats::dds` |
| [tga.md](tga.md) | TGA images (most interface art): header, pixel layouts, run-length packets, alpha | `m2_formats::tga` |
| [jpeg.md](jpeg.md) | JPEG, baseline and progressive (some effects' textures, the sphere maps): markers, Huffman tables, progressive scans, the inverse DCT, chroma sampling | `m2_formats::jpeg` |
| [script.md](script.md) | Text scripts: the `Group`/`List` syntax, model scripts (`.msm`), motion scripts (`.msa`), motion lists, environments (`.msenv`), sub-images (`.sub`), the monster and item lists, the skill descriptions and the skill table | `m2_formats::script` |
| [map.md](map.md) | Maps: settings, area grids (heights, tiles, attributes, water, shadows), placed objects, texture sets, properties, the atlas | `m2_formats::map`, `m2_assets::map` |
| [mdatr.md](mdatr.md) | Model attributes (`.mdatr`): the collision shapes and the floors (height planes) of buildings' and dungeon blocks' models | `m2_formats::mdatr`, `m2_assets::map` |
| [effect.md](effect.md) | Effects: effect scripts (`.mse`) with their particles and meshes, mesh effects (`.mde`), the files they name | `m2_formats::{script::effect, mde}`, `m2_assets::effect` |
| [sound.md](sound.md) | Sounds: WAV and MP3 files, sound scripts (`.mss`), the maps' sound spots and `Ambience` properties, the music, `metin2.cfg`'s volumes | `m2_formats::{wav, script::sound, map::ambience}`, `m2_assets::sound`, `m2_audio::decode` |
| [gltf.md](gltf.md) | glTF 2.0 (`.glb`), openm2's own format for models and animations: what is read, refused and ignored, the axes and units, into the model layer and back, openm2's own extension and extras | `m2_formats::gltf` |

**Verification:** every reader is checked against a TMP4 40250 client; each spec's "Verified against" line says what was checked. Facts come from that client's files, community documentation (facts only, [`../../source-policy.md`](../../source-policy.md)) and, for the proto tables, the container header and the skill table's columns, Quantum Core X's readers at `007c7a6`. In that client:
- every entry of its 107 archives decodes, and its proto tables parse; an entry's data CRC cannot be relied on ([eterpack.md](eterpack.md));
- every format-6 GR2 file reads as a model; one Oodle1 rule differs from its published description ([oodle1.md](oodle1.md));
- every DDS texture, all 4,022 TGA files and all 501 JPEG files (452 baseline, 49 progressive) decode, and its 1,955 sub-images find their images;
- all 15,334 text scripts parse ([script.md](script.md)), and the English skill table's 70 skills read with every formula;
- all 86 maps, 1,192 areas, 1,328 property files and 204 atlas files read, checked against each other and the client's minimaps ([map.md](map.md));
- all 1,148 effect scripts and 112 mesh effects read; what many of the scripts' numbers mean is openm2's reading ([effect.md](effect.md));
- all 1,578 WAV and 34 MP3 files decode, and its 3,509 sound scripts and 1,177 area sound files read ([sound.md](sound.md));
- all 1,170 model attribute files read and write back byte for byte ([mdatr.md](mdatr.md)).

## How a file is read
```
game path, e.g. d:/ymir work/pc/warrior/warrior_novice.gr2
   │  m2_assets::Vfs: loose folders first, then the archives in pack/*.eix
   ▼
archive entry: one .eix row (type, offset, sizes, CRCs)
   │  m2_assets::Archive: positional read of the stored bytes from the .epk
   ▼
stored bytes ──── type 0 ─────────────────────────────────────────▶ the file
   │ type 1 or 2
   ▼
MCOZ container ── type 2: XTEA-decrypt (key eterpack.entries) ── LZO1X ──▶ the file
```

`item_proto` and `mob_proto` are files like any other (`locale/<name>/item_proto`). Inside, their records sit in another `MCOZ` container, encrypted with the `proto.item` or `proto.mob` key ([proto.md](proto.md)).

## Tools
| Command | What it does |
|---|---|
| `openm2 pack ls <file.eix>` | List an archive's entries |
| `openm2 pack cat <file.eix> <name> --out <file>` | Write one decoded entry |
| `openm2 pack extract <file.eix> --out <dir>` | Write all entries under a folder |
| `openm2 vfs ls --client <dir> [prefix]` | List every file in the client's archives |
| `openm2 vfs cat --client <dir> <path> --out <file>` | Write one file, found by game path |
| `openm2 vfs check --client <dir>` | Decode everything, check CRCs, parse the proto tables |
| `openm2 proto dump <file>` | Print a proto table as JSON Lines |
| `openm2 gr2 info\|dump\|model <file>` | Look inside a GR2 file: header and sections, the data tree, or the model a renderer gets |
| `openm2 gr2 check --client <dir>` | Read every GR2 file of the client as a model |
| `openm2 dds info\|png <file>`, `dds check --client <dir>` | A texture's format and levels, one level as a PNG; decode every texture |
| `openm2 tga info\|png <file>`, `tga check --client <dir>` | A TGA image's layout, the image as a PNG; decode every TGA file |
| `openm2 jpeg info\|png <file>`, `jpeg check --client <dir>` | A JPEG image's size and components, the image as a PNG; decode every JPEG file |
| `openm2 script dump <file>` | A text script, one value per line |
| `openm2 script check --client <dir>` | Parse every script and motion list, and follow the files that model and motion scripts name |
| `openm2 map ls\|info <map>` | The client's maps; what one map is made of |
| `openm2 map check --client <dir>` | Parse every map, area and property file, and follow what they name (models and their attributes too) |
| `openm2 assets import --client <dir> --out <dir>` | Convert the client into an asset folder of openm2's own formats, which `play` and the `view` commands read ([`../assets.md`](../assets.md)) |

Keys come from `--keys <file>` or `OPENM2_KEYS` ([keys.md](keys.md)).
