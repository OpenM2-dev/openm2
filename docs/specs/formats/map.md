# Maps

A map is a folder of files: settings, then a grid of areas, each with its height map, terrain textures, attributes, water, baked shadows and placed objects. Objects name property files, which say what the object is (a building's model, a tree, an effect). This page covers the files; how openm2 draws them is in [`../render.md`](../render.md), "Terrain". An asset folder holds maps in openm2's own formats instead ([`../assets.md`](../assets.md), "Maps"), which `openm2 assets import` writes from these files.

**Code:** `m2_formats::map` (`setting`, `grid`, `area`, `ambience`, `texture_set`, `property`, `atlas`), `m2_assets::map` (a map read from the client, the property index). Commands: `openm2 map ls|info|check`, `openm2 view map`.

**Source:** the files of a TMP4 40250 client, examined with `openm2 vfs cat`, `script dump` and counting scripts (`docs/source-policy.md`, "Allowed inputs"). The layouts were worked out from the files' sizes and contents and checked against each other: object heights against height maps, water against the terrain under it, the drawn map against the client's own `minimap.dds` pictures. No other source.

**Verified against:** that client. `map check` reads all 86 map folders, their 1,192 areas and 1,328 property files; the counts below come from it.

## Files
| File | Where | Syntax | What |
|---|---|---|---|
| `atlasinfo.txt` | root | lines | where each map is in the world ([atlas](#atlas)) |
| `setting.txt` | map folder | script, `MapSetting` | sizes, scales, texture set, environment ([settings](#settings)) |
| `mapproperty.txt` | map folder | script, `MapProperty` | `MapType`: `Outdoor` in 85 maps, `Invalid` in one; not used yet |
| `monsterarrange.txt`, `monsterareainfo.txt` | map folder | lines with `//` comments | where monsters stand; not read (the server places monsters) |
| `height.raw` | area | binary | terrain heights ([grids](#area-grids)) |
| `tile.raw` | area | binary | which terrain texture covers each tile |
| `attr.atr` | area | binary | movement attributes |
| `water.wtr` | area | binary | water cells and their heights |
| `shadowmap.raw` | area | binary | shadows baked onto the terrain |
| `shadowmap.dds` | area | DDS | the shadows again; not read |
| `minimap.dds` | area | DDS | the area from above, north up: 256 × 256 DXT1 in every area that has one (1,129 of 1,192), a texel a metre; the minimap draws them (`ui.md`, "The minimap") |
| `areadata.txt` | area | blocks | objects placed on the area ([objects](#objects-areadatatxt)) |
| `areaambiencedata.txt` | area | blocks | sound spots (7 in the client): [sound.md](sound.md), "Sound spots" |
| `areaproperty.txt` | area | script, `AreaProperty` | `AreaName` (empty in the areas looked at) and `NumWater` (as in `water.wtr`); not read |
| `textureset/*.txt` | root | blocks | terrain textures ([texture sets](#texture-sets)) |
| `property/**/*.prb` etc. | root | lines | what an object is ([properties](#properties)) |
| `d:/ymir work/ui/atlas/<map>/atlas.sub` | interface | sub-image | the whole map from above ([atlas pictures](#atlas-pictures)) |
| `locale/<locale>/map/<map>_point.txt` | locale | lines with `//` comments | the points the atlas marks ([atlas points](#atlas-points)) |

A few areas also have a `splat.raw`; nothing reads it.

## Coordinates
- **Cells and areas:** a map is cut into areas of 128 × 128 cells. A cell is `CellScale` cm wide (200 in every map), so an area is 256 m wide.
- **Area folders:** area (x, y) is the folder `{x:03}{y:03}`, x first: `001002` is x 1, y 2.
- **Map coordinates:** x grows east and y grows south, in centimetres from the map's top left corner.
- **The world:** objects store their position as (x, −y, z), with z up. openm2 draws in that world: x east, y north, z up, so a map lies in negative y. Seen from above with north up, area (0, 0) is the top left one, as in the client's minimaps.
- **World positions** (what the server sends) are map coordinates plus the map's `BasePosition` (or its atlas position).

## Settings
`setting.txt` is a script ([`script.md`](script.md)) of type `MapSetting`:

| Key | Values | What |
|---|---|---|
| `CellScale` | 1 number | side of a cell in cm (200 in all 86 maps) |
| `HeightScale` | 1 number | cm per step of `height.raw` and of water heights (0.5 in all 86 maps) |
| `ViewRadius` | 1 number | how far the client draws, in cells (128 in 82 maps, 256 in 3, 512 in 1); not used yet |
| `MapSize` | 2 numbers | areas along x and y (1 to 7 per side in the client) |
| `BasePosition` | 2 numbers | the map's top left corner in the world, cm. The oldest map has none |
| `TextureSet` | 1 value | the texture set, as a game path (`textureset\metin2_A1.txt`, any case). The oldest map writes only a file name; openm2 then looks in `textureset/` |
| `Environment` | 1 value | the environment script, a file name in `d:/ymir work/environment/` ([`script.md`](script.md), "Environment") |

`MapSize` must be 1 to 1000 per side (area folders have three digits) and `CellScale` 1 to 100,000; other values are refused as damaged.

## Area grids
All grids are little-endian, stored row by row from the area's north-west corner, x first.

| File | Size (bytes) | Layout |
|---|---|---|
| `height.raw` | 34,322 | 131 × 131 `u16` |
| `tile.raw` | 66,564 | 258 × 258 `u8` |
| `shadowmap.raw` | 131,072 | 256 × 256 `u16` |
| `attr.atr` | 6 + width × height | `u16` magic `0x0A4A`, `u16` width, `u16` height (256 and 256 in the client), then a `u8` per square |
| `water.wtr` | 7 + width × height + 4 × layers | `u16` magic `0x1532`, `u16` width, `u16` height (128 and 128), `u8` layer count, a `u8` per cell, then an `i32` height per layer |

Every file of the client has exactly these sizes.

### Heights (`height.raw`)
- An area has 129 × 129 vertices (128 cells and one more). The file has one more row and column on every side, so value (i + 1, j + 1) is vertex (i, j); the border repeats the neighbouring areas' edges and lets normals be computed at the edges.
- Vertex (i, j) is at map coordinates (area x × 25,600 + i × 200, area y × 25,600 + j × 200), and its height is the value × `HeightScale` cm.
- **Checked:** an object's z is the terrain's height at the vertex nearest to it. Over 975 objects of one map and 97 of another the median difference is 0 with this layout; without the border it is not.

### Tiles (`tile.raw`)
- A tile is half a cell (100 cm): 256 × 256 tiles per area, plus a border of one on every side as in `height.raw`. Value (x + 1, y + 1) is tile (x, y).
- The value is a texture number of the map's texture set ([below](#texture-sets)); 0 is no texture.
- **In the client:** 45 areas use numbers their set has no texture for (one up to 19 in a set of 17), a few tiles each: leftovers of textures deleted in the editor. openm2 draws nothing for those tiles; their neighbours blend over them.

### Shadows (`shadowmap.raw`)
- One texel per tile (256 × 256), a 16-bit colour with 5 bits per channel: bits 10–14 red, 5–9 green, 0–4 blue, bit 15 unused.
- `0x7FFF` (white) is no shadow; the client's shadows are greys such as `0x4210` (half). The shadows of buildings and trees are baked in, so the original's trees leave their shadows under openm2's own.
- `shadowmap.dds` holds the same shadows as a texture; openm2 reads the `.raw` one.

### Attributes (`attr.atr`)
- One byte of flags per square half a cell wide (256 × 256 per area: a metre with the client's 200 cm cells).
- Every bit is used in the client, often together. The most common values are `0x01` (34 million squares), `0x00` (18 million), `0xC9` (12 million) and `0xC8` (6 million).
- **Meaning**, from `openm2 map attr`'s pictures of the flags over the areas' own pictures (`minimap.dds`) and its counts against `water.wtr`, on `metin2_map_a1` and `metin2_map_c1`:
  - **`0x01` blocked:** mountains and cliffs, deep water, every building's footprint, tree trunks and the map's edges; bridges and piers across blocked water stay open. openm2 blocks these squares.
  - **`0x02` water:** every square with it lies under a water layer (all of them on both maps); deep water has `0x01` too, shallow water not.
  - **`0x04`:** a few thousand squares in circles: the town's square and the gates on a1. Community documentation calls it a no-fight zone; not confirmed, and not used.
  - **`0x08`, `0x40`, `0x80`:** not on a1. On c1, `0x40` is on every square, and `0x08` with `0x80` cover a coarser outline of the outside that takes in the bridges; on many maps almost every square has `0xC8` or `0xC9`. They cannot mean blocked; their meaning is unknown and openm2 does not read them.
- **The server's copy:** Quantum Core X `007c7a6` reads its own file per map (`server_attr`: LZO-compressed blocks of 32-bit flags, not the client's `attr.atr`) and names `0x01` blocked, `0x02` water, `0x04` no fighting and `0x80` a building. It refuses a jump into a blocked or building square, and keeps monsters out of them; it does not check walks.
- `openm2 content import` turns the attributes into walk masks ([`../server/content.md`](../server/content.md), "Walk masks").

### Water (`water.wtr`)
- One byte per cell (128 × 128): the water layer the cell is under, or `0xFF` for none. A layer number past the layer count is none too.
- Each layer's height is an `i32` in `HeightScale` steps, like `height.raw`.
- **Checked:** with this unit the water lies a little above the terrain under it in most areas (for example 16,384 cm over a lake bed at 16,143 cm). A few layers lie under their terrain, and one in the client has a height of −2,047,836,160; nothing shows there.

## Objects (`areadata.txt`)
A block file (below): the header `AreaDataFile`, one block per object, and `ObjectCount` at the end.

```text
AreaDataFile

Start Object000
    1200.000000 -2600.000000 15000.000000
    123456789
    0.000000#0.000000#90.000000
    0.000000
End Object

ObjectCount 1
```

| Line | What |
|---|---|
| 1 | position: x, −y, z in cm, map coordinates. z is the terrain's height there |
| 2 | the id of the object's property ([below](#properties)) |
| 3 | rotations in degrees, written `x#y#z`. The third turns the object around the vertical axis (positive counter-clockwise seen from above); the first two tilt it (1,498 objects tilt) |
| 4 | centimetres added to the height (37,153 objects have an offset, from −5,000 to +3,500) |
| 5… | 427 objects have more lines: small numbers (1 to 13, a few in pairs such as `11 21`) or values written `#1.310000`. Their meaning is unknown; openm2 keeps them |

- `ObjectCount` matches the number of blocks in every file; openm2 refuses a file where it does not.
- **Rotation, checked:** buildings drawn with z, then y, then x rotations sit and face exactly as in the client's minimaps. The order of the two tilts is not confirmed (tilts are small).
- **In the client:** 51,427 objects: 20,112 buildings, 12,780 trees, 16,378 effects, 2,072 dungeon blocks; 85 name a property the client does not have. Effects are placed like buildings (position, height offset, rotations) and play their script there ([effect.md](effect.md)); most of them are in the Skipia dungeons (3,000 and more each), the other maps have a few dozen at most (fires, smoke, waterfalls, warp gates).

### Block files
`areadata.txt`, `areaambiencedata.txt` and texture sets share a syntax:
- The first line is one word, the header. Lines are split like script lines (values separated by spaces or tabs, `"…"` as one value).
- `Start Name` opens a block and a line starting with `End` closes it (`End Object`, `End Texture001`); blocks do not nest. The lines inside mean something by their position.
- Other lines outside blocks are `Key value` lines (`ObjectCount 30`, `TextureCount 17`).

## Texture sets
`textureset/*.txt`, named by a map's `TextureSet`:

```text
TextureSet

TextureCount 2
Start Texture002
    "d:\ymir work\terrainmaps\test\grass.dds"
    5.000000
    5.000000
    0.000000
    0.000000
    0
    0
    0
End Texture002
```

| Line | What |
|---|---|
| 1 | the texture file (a game path, any case, `\` or `/`) |
| 2, 3 | how often it repeats along x and y (the unit is in [`../render.md`](../render.md)) |
| 4, 5 | offset along x and y (0 everywhere) |
| 6–8 | three integers, the editor's settings (`0 0 0` in most sets); not used for drawing |

- **Numbers:** a block's name is `Texture` and its number, and tile value n means the block named with n. Numbers can be missing: one set has no `Texture001` and its tiles never use 1, another has no `Texture003` and its tiles use 3 only 68 times. `TextureCount` is the highest number, not how many blocks there are.
- Numbers run from 1 to 255 (tile values are bytes, 0 is none).
- **In the client:** 86 maps name 37 different sets with 405 textures; 16 textures are missing. Scales run from 1 to 15. Four files in `textureset/` are not texture sets (three packing scripts and one damaged file); no map uses them.

## Properties
`property/**/*.prb` (buildings), `.prt` (trees), `.prd` (dungeon blocks), `.pre` (effects), `.pra` (sounds):

```text
YPRT
123456789
buildingfile        "d:/ymir work/zone/test/house.gr2"
propertyname        "house"
propertytype        "Building"
shadowflag          "1"
```

- The first line is `YPRT`, the second the property's id, which objects name it by. The rest are `key "value"` lines.
- The id is not the CRC-32 of the file's path or of `propertyname` (both tried); where it comes from is unknown, and nothing needs it.
- `propertytype` says which keys follow:

| `propertytype` | Files | Keys |
|---|---|---|
| `Building` | 1,083 | `buildingfile` (a GR2 model), `shadowflag`, sometimes `isattributedata` (always `0`, not read: [mdatr.md](mdatr.md)) |
| `Tree` | 85 | `treefile` (a SpeedTree `.spt`; openm2 reads only its name), `treesize`, `treevariance` |
| `DungeonBlock` | 83 | `dungeonblockfile` (a GR2 model) |
| `Effect` | 75 | `effectfile` (an effect script, `.mse`, [effect.md](effect.md)) |
| `Ambience` | 2 | `ambiencesoundvector` (a sound), `playtype`, `playinterval`, `playintervalvariation`, `maxvolumeareapercentage` ([sound.md](sound.md)) |

- **Duplicates:** 14 files repeat an id that another file already has. openm2 keeps the first by file name; which one the client keeps is unknown.
- `property/reserve` is a list of numbers, not a property; nothing reads it.
- **In the client:** the buildings and dungeon blocks name 871 models; 11 are missing. 693 of the models have attributes beside them: collision shapes and the floors characters stand on ([mdatr.md](mdatr.md)).

## Atlas
`atlasinfo.txt`, one map per line: its folder, the world position of its top left corner (cm) and its size in areas, separated by tabs or spaces (some lines have extra tabs).

```text
metin2_map_test    102400    51200    2    3
```

- A world position (x, y) is on a map when base ≤ position < base + size × 25,600 on both axes.
- **In the client:** 110 lines; 94 name a map folder the client has, 16 do not, and some folders are on no line. One map is listed four times at different places.

### Atlas pictures
`d:/ymir work/ui/atlas/<map>/atlas.sub`, `<map>` the last part of the map's folder: a sub-image ([`script.md`](script.md), "Sub-images"; version 1.0, so its image is in `d:/ymir work/ui/`, as `<map>_atlas.dds`) of the whole map from above, north up, its top left corner the map's.

- **In the client:** 23 of the 94 maps the atlas and the client both have; the others (dungeons, event maps) have none. About 6 m a pixel (171 × 214 for a map of 4 × 5 areas, 252 × 252 or 256 × 256 for 6 × 6; 3 m for a map of 2 × 2), at most 256 pixels a side.
- **Checked:** on `metin2_map_c1` the warp points below fall at the ends of the picture's roads, where the map's warp gate effects stand.

### Atlas points
`locale/<locale>/map/<map>_point.txt`: the NPCs and warps a map's atlas marks. One point per line, five values separated by tabs: a number, a kind, the map position in centimetres (x east, y south of the map's top left corner) and a quoted name. Lines starting with `//` are comments.

```text
//num	type	x	y	name
0	NPC	43000	60800	"Blacksmith"
5	WARP	13700	12600	"North gate"
```

- **Kinds:** `NPC` and `WARP`; the files' comment lists `OPC`, `OPCPVP`, `OPCPVPSELF`, `NPC`, `MONSTER`, `WARP` and `WAYPOINT`, but no file uses the others.
- **Names:** in Korean (codepage 949) in every locale's files, the English and German ones too; `d:/ymir work/ui/atlas/936_<map>_point.txt` and `949_<map>_point.txt` hold the same points with Chinese (936) and Korean names.
- **In the client:** 204 files (12 maps in each of 15 locales and the two codepage sets), 1,445 points (935 NPCs, 510 warps). Every file starts with three comment lines, ends its lines with CRLF and numbers its points from 0 in order; a map's points are the same in every file, and all lie on their map.
- **Checked:** the two warps of `metin2_map_c1` are at 137,126 m and 125,1,118 m; the map's warp gate effects stand at 135,124 m and 124,1,119 m (`map info`).

## Tools
| Command | What it does |
|---|---|
| `openm2 map ls --client <dir>` | Every map folder: size, areas present, atlas position |
| `openm2 map info <map> --client <dir>` | A map's settings, texture set, heights, water and objects by kind; the effect scripts its objects play, with where the first of each is (metres east and south of the map's corner); its sound spots; its atlas picture and the locale's atlas points, each with where it is |
| `openm2 map check --client <dir>` | Parse every map, area and property file, and follow what they name; the areas' pictures by size, the atlas pictures and every atlas point file; `just client-check` runs it |
| `openm2 view map <map> --assets <dir>` | Draw a map `assets import` converted ([`../render.md`](../render.md), "Terrain") |
