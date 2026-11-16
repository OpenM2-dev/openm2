# Model attributes (`.mdatr`)

What a building's or a dungeon block's model is to the characters around it: collision shapes, and **height planes**, the floors they stand on (a raised platform, a bridge's deck, the ramps up to them, a dungeon's floors). How `play` stands characters on them is in [`../render.md`](../render.md), "Floors".

**Code:** `m2_formats::mdatr` (`ModelAttributes`, `Collision`, `Shape`, `HeightPlane`); `m2_assets::map::{model_attributes_path, read_model_attributes}`. Commands: `openm2 map check` counts the attributes of the models the maps use; `openm2 view map --floors` draws the floors; `play` stands characters on them.

**Source:** the files of a TMP4 40250 client, examined with `openm2 vfs cat`, hex dumps and counting scripts (`docs/source-policy.md`, "Allowed inputs"). The layout was worked out from the files' sizes and contents: the record sizes from files with one shape, the meaning of the numbers from the shapes' and planes' places against the models drawn with `view map`. No other source.

**Verified against:** that client. All 1,170 `.mdatr` files read and write back byte for byte (`crates/m2-assets/tests/real_client.rs`); the height planes of the a1 pavilion (`a1-001-house3`: its stone plinth 17 cm up, its platform 101 cm up and the ramps over its steps), a stone bridge turned 60° (`a1-024-10m-bridge`) and a suspension bridge turned 270° lie on the models' floors and decks when placed like their objects (`view map --floors`). A dungeon's planes are coarser: the devil tower's second floor (`deviltower2`) is one uneven plane 9 cm over most of its model's upward faces.

## Where they are
- Beside the model: the GR2 file's path with `.mdatr` for its extension (`d:/ymir work/zone/a/building/a1-022-10m-bridge.gr2` → `…/a1-022-10m-bridge.mdatr`).
- **In the client:** 1,170 files, most of them for buildings and dungeon blocks; a few for NPCs' models (`npc/seal_stone`) and guild buildings. The maps' objects name 871 models; 693 of them have attributes (`map check`).
- Some building properties have `isattributedata "0"` ([map.md](map.md), "Properties"), most of them for models that do have attributes; openm2 does not read it.

## Layout
Little-endian. Names are 32-byte fields, NUL-terminated and NUL-padded (nothing follows the NUL in the client's files).

| Offset | Size | What |
|---|---|---|
| 0 | 14 | the magic `AttributeData` and a NUL |
| 14 | 4 | `u32`: the number of collision shapes |
| 18 | 4 | `u32`: the number of height planes |
| 22 | … | the collision shapes, then the height planes |

**A collision shape** (68 or 72 bytes):

| Offset | Size | What |
|---|---|---|
| 0 | 4 | `u32` kind: 0 plane, 2 sphere, 3 cylinder |
| 4 | 32 | name (`collisionPlane04`, `collision plane`, `Collision`, …) |
| 36 | 12 | position: x, y, z as `f32`, cm, in the model's space (z up) |
| 48 | 8, 4 or 8 | sizes as `f32`, cm: a plane's two sides; a sphere's radius; a cylinder's radius and height |
| 56, 52 or 56 | 16 | rotation: a unit quaternion x, y, z, w as `f32` |

**A height plane** (36 bytes and 12 per corner):

| Offset | Size | What |
|---|---|---|
| 0 | 32 | name (`heightPlane01`, `Height01`, `heightstep03`, `heightBox01`, …) |
| 32 | 4 | `u32`: the number of corners, a multiple of 3 |
| 36 | 12 × n | the corners: x, y, z as `f32`, cm, in the model's space; every three are a triangle |

- **Placement, checked:** the corners placed with their object's transform (position raised by its height offset, turned about z, y and x, as the model: [map.md](map.md), "Objects") lie on the model's floors.
- Names say nothing reliable (any case, with or without spaces and numbers); openm2 keeps them and does not read them.
- **In the client:** 7,843 planes, 297 spheres and 456 cylinders; 270 files have height planes (557 of them, 35,029 triangles); 9 files are only the header. Every rotation is a unit quaternion. The largest file is 116,802 bytes, the largest height plane a floor of the devil tower's second floor with 9,102 corners; the widest triangle spans 272 m (`deviltower_1f`), and 479 triangles stand on edge (under 1 cm² seen from above).
- **Not known:** whether a plane's two sizes are its sides or half of them, and what kind 1 would be (perhaps a box; no file has it). openm2 does not use the collision shapes: the maps' attributes already block buildings' footprints ([map.md](map.md), "Attributes").

## Reading rules
- The magic must match, else `BadMagic`.
- Counts are checked against the bytes left before anything is allocated: a count that cannot fit is damaged.
- A shape of another kind is an unknown layout, naming the kinds known.
- A corner count that is not a multiple of 3 is damaged, and so are bytes after the last height plane.
- The writer writes the same layout; a name longer than 31 bytes is an error, never cut.
