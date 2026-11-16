# GR2 (Granny) models and animations

The client's 3D data is in Granny GR2 files (RAD Game Tools' "Granny 3D"): character and monster models, their skeletons, animations, weapons, buildings and map objects. A GR2 file is a memory image: structures with 32-bit pointers between them. It describes its own layout, so a reader can walk it without knowing every structure in advance.

**Sources:**
- LSLib (MIT): the file header, section table, relocations and member kinds. Facts only.
- Community documentation of the format (arves100's Granny2-research wiki), for the header fields. Facts only.
- liboodle (The Unlicense), for the section compression ([oodle1.md](oodle1.md)).
- The member names openm2 reads (`Skeletons`, `Bones`, `Position`, …) are stored in every file. They were read from the client's own files.

**Verified against:** a TMP4 40250 client (`openm2 gr2 check`).
- **Files:** of its 8,831 GR2 files, 8,829 are format 6 and all of them read as models. That is 4,050 skeletons, 6,982 meshes and 5,185 animations, with every CRC matching. The other 2 are format 7 (below).
- **Exporters:** "Granny Standard Exporter, SDK version 2.4.0.7", from 3D Studio MAX, with exporter tags `0x8000000f`, `0x80000010` and `0x80000011`.
- **Data checks:** knots never go backwards, and every vertex's bone weights add up to 1. 177 files have zero-length normals (degenerate vertices). Two animations (`monster2/redthief_general/back_damage.gr2` and `front_damage.gr2`) have NaN rotation controls. Both are in the files themselves.
- **Skeletons:** the largest has 163 bones (`monster/misterious_diseased_bosshost`).
- **Textures:** of 5,891 texture references, 5,645 are game paths as written and 184 are found in the model's folder; 62 are missing (`docs/specs/render.md`, "Textures").

**Code:** `crates/m2-formats/src/gr2/` (`mod.rs` container, `object.rs` layouts, `model.rs` models, `write.rs` writer for tests).

## File layout
All integers are little-endian.

| Offset | Size | Field |
|---|---|---|
| 0 | 16 | magic `b8 67 b0 ca f8 6d b1 0f 84 72 8c 7e 5e 19 00 1e` (little-endian, 32-bit pointers, format 6) |
| 16 | 4 | header size (header + file information + section table) |
| 20 | 4 | header format, 0 |
| 24 | 8 | reserved |
| 32 | 56 | file information (below) |
| 88 | 44 × n | section table |

**File information** (format 6):

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | format version, 6 |
| 4 | 4 | total file size |
| 8 | 4 | CRC-32 of the file from the section table (offset 88) to the end |
| 12 | 4 | offset of the section table, counted from this structure (56) |
| 16 | 4 | number of sections (8 in the client's files) |
| 20 | 8 | root layout: section, offset |
| 28 | 8 | root object: section, offset |
| 36 | 4 | exporter tag (a Granny version marker) |
| 40 | 16 | four user tags |

**Section header** (44 bytes):

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | compression: 0 none, 1 Oodle0, 2 Oodle1, 3 BitKnit, 4 BitKnit 2 |
| 4 | 4 | offset of the stored data in the file |
| 8 | 4 | stored size |
| 12 | 4 | size in memory |
| 16 | 4 | alignment in memory (4 or 32) |
| 20 | 4 | first16: where the 16-bit data starts (end of the first Oodle1 stream) |
| 24 | 4 | first8: where the 8-bit data starts |
| 28 | 4 | offset of the relocation table in the file |
| 32 | 4 | number of relocations |
| 36 | 4 | offset of the mixed-marshalling table |
| 40 | 4 | number of mixed-marshalling entries |

**In the client:**
- Sections are Oodle1 (26,074 of them) or stored (186, all empty).
- Relocation tables are stored as is, not compressed.
- Section 6 holds the layouts and section 0 the main objects; the reader does not depend on that.

**Relocation** (12 bytes): offset of a pointer slot in this section; target section; target offset. Each pointer in the image has one, and the slot's own bytes are meaningless. They are whatever the exporter's memory held, often old addresses. A word without a relocation is a null pointer.

**Mixed marshalling** tells a big-endian reader which data to byte-swap. A little-endian reader ignores it.

**Reading rules (openm2):**
- **Image:** all sections go into one image, each at its alignment, starting at address 16 so that no target is 0.
- **Pointers:** each pointer is looked up in the relocation table. Its target must be inside the target section; the end of a section is allowed, which empty arrays use.
- **Magic:** only the format-6 little-endian 32-bit magic is read. The format-7 magic (`29 de 6c c0 ba a4 53 2b 25 f5 b7 a5 f6 66 e2 ee`) is named in the error and counted as skipped.
- **Compression:** only none and Oodle1 are read.
- **CRC:** a mismatch is reported (`crc_ok`), not refused. Every file in the client matches.
- **Limits:** at most 64 sections and 64 MiB of image; layouts nest at most 16 deep and have at most 1024 members; strings are at most 64 KiB.

**Format 7** (Granny 2.7 and later) has 16 more bytes of file information, and its relocation tables sit at the end of the file, probably compressed. Two costume models in the client use it: `pc/shaman/shaman_lord.gr2` and `pc/warrior/warrior_rabbit1.gr2`. Not read (`docs/roadmap.md`, "Open questions").

## Layouts
A layout is a list of 32-byte member records, ended by a record of kind 0:

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | kind (below) |
| 4 | 4 | pointer to the name (NUL-terminated) |
| 8 | 4 | pointer to a nested layout (inline structures, references) |
| 12 | 4 | array length, 0 if not an array |
| 16 | 12 | three extra words (unused here) |
| 28 | 4 | unused |

Members follow each other without padding. An array member repeats its element `array length` times in place.

| Kind | Name | Size in the structure |
|---|---|---|
| 0 | end | — |
| 1 | inline structure | the nested layout's size |
| 2 | reference | 4: pointer to a structure of the nested layout |
| 3 | reference to array | 8: count, then pointer to that many structures |
| 4 | array of references | 8: count, then pointer to that many pointers |
| 5 | variant reference | 8: pointer to a layout, then pointer to a structure of it |
| 7 | reference to variant array | 12: pointer to a layout, count, pointer to the structures |
| 8 | string | 4: pointer to a NUL-terminated string |
| 9 | transform | 68: flags `u32` (bit 0 position, bit 1 orientation, bit 2 scale/shear), position 3 × `f32`, orientation quaternion x y z w, scale/shear 3 × 3 `f32` by rows |
| 10 | real32 | 4 |
| 11, 12 | int8, uint8 | 1 |
| 13, 14 | binormal int8 (−1…1), normal uint8 (0…1) | 1 |
| 15, 16 | int16, uint16 | 2 |
| 17, 18 | binormal int16, normal uint16 | 2 |
| 19, 20 | int32, uint32 | 4 |
| 21 | real16 (IEEE half) | 2 |
| 22 | empty reference | 4, always null |

Kind 6 does not occur and is refused. Lists of numbers (indexes, knots, controls) are stored as references to arrays of one-member structures, for example `Indices` → `{ Int32 }`.

## What openm2 reads
`Gr2File::read` starts at the root object, which is Granny's file information:

| Member | What openm2 takes |
|---|---|
| `ArtToolInfo` | `FromArtToolName`, `UnitsPerMeter` (100: centimetres), `Origin`, `RightVector`, `UpVector` (0, 0, 1: Z up), `BackVector` |
| `FromFileName` | the exporter's source file |
| `Textures` | `FromFileName` of each: a `.dds` or `.tga` path such as `D:\ymir work\pc\warrior\warrior_4-1.dds` |
| `Materials` | `Name`; `Texture`; `Maps` (`Usage`, `Map` → another material). A model's mesh material often has no texture of its own but a "Diffuse Color" map to a material that has one. |
| `Skeletons` | `Name`; `Bones`: `Name`, `ParentIndex` (−1 for the root; parents come first), `Transform` (rest pose, relative to the parent), `InverseWorldTransform` (16 `f32`, column-major, translation in elements 12–14) |
| `Meshes` | `Name`; `PrimaryVertexData` → `Vertices` (a variant array); `PrimaryTopology` → `Groups` (`MaterialIndex`, `TriFirst`, `TriCount`) and `Indices` (or `Indices16`); `MaterialBindings` (`Material`); `BoneBindings` (`BoneName`) |
| `Models` | `Name`, `Skeleton`, `InitialPlacement`, `MeshBindings` (`Mesh`) |
| `Animations` | `Name`, `Duration` (seconds), `TimeStep`, `TrackGroups` → `TransformTracks`: `Name` (a bone), `PositionCurve`, `OrientationCurve`, `ScaleShearCurve` |

References between these structures (a material's texture, a mesh's materials, a model's skeleton and meshes) become indexes into the file's lists, matched by address.

**Vertices** have the members `Position` (3 × `f32`), `Normal` (3 × `f32`), `TextureCoordinates0` (2 × `f32`), `BoneWeights` (4 × normal `u8`) and `BoneIndices` (4 × `u8`). A rigid mesh has no bone members and follows its first bone binding. Bone indexes index the mesh's `BoneBindings`, which name skeleton bones.

**Limits (openm2):** at most 4 Mi vertices, indexes or knots per list. An index past the vertices, a group past the triangles, or a bone whose parent is not an earlier bone is refused as damage.

## Curves
Animation curves in the client's files are B-splines: `Degree` (0 or 2), `Knots` (times in seconds, from 0 to the duration) and `Controls` (3 values per knot for position, 4 for orientation, 9 for scale/shear). A curve without knots holds nothing: the bone keeps its rest value for that part. Degree 0 with one knot is a constant.

The exporter places knots on a regular grid (1/32 or 1/24 of a second in the client's files). The first gap is two steps wide, and later fitting removes some knots.

**Evaluation (openm2's reading):**
- Knot `i` goes with control `i`. For time `t`, find the first knot after `t`, or the last knot if there is none.
- Take the `degree + 1` controls ending there and the `2 × degree` knots around it. Before the first and after the last knot, the end knot and control repeat.
- Run de Boor's algorithm on that window.

So the curve passes through its first and last controls, and degree 1 is linear interpolation between (knot, control) pairs. That fits a fitter that starts and ends on sampled values. **Confirmed by eye:** characters and monsters animate correctly with it (`docs/specs/render.md`, "Posing").

Orientation curves give quaternions, which need normalising after sampling. A few controls in the client have a length of 1.5 to 1.8, and two animations have NaN controls.

## Into the model layer
Renderers don't draw `Gr2File` itself but the model layer (`m2_formats::model`), which glTF files will fill too.
- **A model** (`Gr2File::model`) is one model's skeleton and meshes:
  - each mesh's triangle groups become ranges of indexes (three per triangle), each with the texture its material shows (its own, or the first map's, the diffuse one first);
  - a mesh without groups is drawn whole with its first material;
  - the textures are listed once each, in the order first shown;
  - a file without models gives all its meshes, bound to no skeleton.
- **An animation** (`Animation::to_animation`) keeps its name, length and track groups; each curve becomes keys, as glTF has them (`Curve::to_keys`):
  - **Degree 0: steps.** The value at `t` is the control of the first knot after `t`, so the first control holds before the first knot, and control `j` from knot `j − 1` on. The keys are (knot 0, control 0), then (knot `j − 1`, control `j`).
  - **Degrees 1 and up: cubic splines,** a key at each distinct knot. Between two knots the curve's window doesn't change, so the curve there is one polynomial of at most degree 3: a cubic Hermite piece holds it exactly. Each piece's tangents come from four samples of the window (at its ends and thirds) and the derivatives of the cubic through them.
  - **Jumps:** where a knot repeats more often than the degree, the curve can jump. The value from the left then gets a key of its own at the same time, and the later key holds from that time on.
  - **Knots that go backwards** (never in the client's files) are sampled four times per knot over their range, with straight lines between.
  - Before the first key the first value holds, after the last key the last, as the curve does.

The keys sample to the curve's values up to rounding: a property test compares them on random curves (degrees 0 to 4, repeated knots, 1 to 9 values), and `gr2 check` compares every animation of a client at its knots, around them and between them.

**Checked against a TMP4 client:** all 5,185 animations of the 8,829 files, 245 million values. The keys differ from the curves by at most 2.2 × 10⁻⁵ of the curve's largest control (`dance_4`, 28.6 s in), which is rounding. In centimetres, the most is 0.047, on a `weaponDummy` bone of the triton's animations that sits 660 m out, where a float's step is 0.004 to 0.008 cm. A warrior's run, the wolf's run and a spot of `metin2_map_a1` (375 buildings) draw as from the curves: two byte for byte, the wolf with 32 of its 442,368 pixels one level apart.

## Tools
| Command | What it does |
|---|---|
| `openm2 gr2 info <file>` | Header, sections, top-level members |
| `openm2 gr2 dump <file> [--depth N] [--items N]` | The whole data tree, through the stored layouts |
| `openm2 gr2 model <file> [--json]` | What a renderer gets: textures, materials, skeletons, meshes, models, animations |
| `openm2 gr2 check --client <dir>` | Read every GR2 file of a client as a model; counts, the largest skeleton, where the named textures are, oddities; every model and animation through the model layer, and the largest difference between the keys and the curves |
| `openm2 view model <file>` | Draw a model, posed by an animation, in a window or a PNG (`docs/specs/render.md`) |

Each command takes `--client <dir>` to read a game path from the client's archives instead of a file, with keys from `--keys` or `OPENM2_KEYS`.

**Our writer** (`gr2::write`) builds format-6 files for tests:
- data goes in section 0 and layouts in section 1, so relocations cross sections;
- sections are stored or Oodle1-compressed;
- `write_model` writes a `Gr2File` with layouts named like Granny's, a subset of their members.
