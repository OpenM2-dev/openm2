# Effects

How openm2 reads effect scripts (`.mse`), mesh effects (`.mde`) and flying effects (`.msf`): sparks, smoke, fire, rings, glows, and what flies with arrows and spells. How they are played and drawn is in [`../render.md`](../render.md), "Effects". An asset folder holds them in openm2's own formats, effect files in TOML and mesh effects in glTF, which `openm2 assets import` writes from these ([`../assets.md`](../assets.md), "Effects"; [gltf.md](gltf.md), "Mesh effects").

**Code:** `m2_formats::script::effect` (`EffectData`, `Curve`), `m2_formats::mde` (`MeshEffect`), `m2_formats::script::fly` (`FlyingData`); `m2_assets::effect` (where the files an effect names are, `read_effect`, `read_flying`, `find_fly_effect`). Commands: `openm2 script check` counts them; `openm2 view effect` plays one into a PNG; `view map` and `play` draw them.

**Source:** the files of a TMP4 40250 client, examined with `openm2 script dump`, `script check --census` and hex dumps of `.mde` files (`docs/source-policy.md`, "Allowed inputs"), and the original client's source where a row names it. The blend factor and texture operation numbers are Direct3D 9's (Microsoft's documentation of `D3DBLEND` and `D3DTEXTUREOP`).

**Verified against:** that client: all 1,148 effect scripts and all 112 mesh effects read (`script check`, `crates/m2-assets/tests/real_client.rs`). What the numbers that choose a behaviour mean is openm2's reading of how the client's files use them, looked at with `view effect` and `view map`, except where the original client's source is named.

## Where effects come from
| From | What | In the client |
|---|---|---|
| Map objects | a property of type `Effect` names the script (`effectfile`, [map.md](map.md)) | 16,378 objects, 75 property files; 3,000 and more in each Skipia dungeon |
| Motion events | types 1 (on a bone) and 10 (where the actor is) ([script.md](script.md), "Motion events") | 1,362 and 131 events, mostly skills and bosses' attacks |
| Model scripts | `AttachingData` blocks of type 2, and the metin stones' smoke ([script.md](script.md), "Race data") | 37 scripts; 20 with smoke (80 stages) |
| Item scripts | weapons' `List HitEffectFileName` | 57 scripts name `d:/ymir work/effect/hit-all/…`, which this client does not have |
| Monster scripts | `List DefaultHitEffectFileName` | 71 scripts, every list empty |
| The client's program | a few effects by path: `etc/dropitem/dropitem.mse`, `etc/direction/direction_land.mse`, `etc/compass/appear_*.mse`, … | — |

## Effect scripts (`.mse`)
The script syntax ([script.md](script.md)), without a `ScriptType`:

```text
BoundingSphereRadius   200.000000
BoundingSpherePosition 0.000000 0.000000 120.000000

Group Particle
{
    StartTime           0.000000
    List TimeEventPosition
    {
        0.000000 "MOVING_TYPE_DIRECT" 0.000000 -10.692772 120.174294
    }
    Group EmitterProperty  { MaxEmissionCount 10  CycleLength 0.100000  …  List TimeEventLifeTime { … } }
    Group ParticleProperty { SrcBlendType 5  DestBlendType 2  …  List TextureFiles { "yeongi.dds" } }
}
Group Mesh
{
    StartTime           0.000000
    List TimeEventPosition { … }
    MeshFileName               "icedumy.mde"
    MeshAnimationLoopEnable    1
    MeshAnimationLoopCount     15
    MeshAnimationFrameDelay    0.020000
    MeshElementCount           22
    Group MeshElement00 { BillboardType 0  BlendingEnable 1  …  List TimeEventAlpha { } }
}
```

- `BoundingSphereRadius` and `BoundingSpherePosition` (cm): 1,132 scripts have them. openm2 frames `view effect` with them.
- Then `Group Particle` and `Group Mesh` blocks, any number, in the order they are drawn. Other blocks are left out (the client has none).
- **Curves:** `List TimeEvent…` blocks hold rows of a time and a value. Between keys the value goes in a straight line, before the first and after the last it stays flat; a list without rows takes a default (openm2's reading). The time is:
  - seconds into the emitter's cycle for `EmitterProperty`'s curves (a 3 s cycle has keys up to 3.0, a 2 s cycle one at 1.806; 466 of 49,085 keys lie past their cycle's end);
  - the fraction of a particle's life, from 0 at birth to 1, for `ParticleProperty`'s (62,143 of 62,334 keys are at most 1);
  - seconds after the element starts for positions and a mesh element's alpha.
- **Positions:** `List TimeEventPosition` rows: a time, a type, then a position (cm, from the effect's origin, z up, forward −y): `"MOVING_TYPE_DIRECT" x y z` (5,200 rows) goes in a straight line to the next key; `"MOVING_TYPE_BEZIER_CURVE" x y z cx cy cz` (228) leaves the key along the vector (cx, cy, cz). openm2 draws that as a quadratic curve through the key plus the vector (openm2's reading). An element without keys sits at the origin.

### Particles: `EmitterProperty`
| Key | What |
|---|---|
| `MaxEmissionCount` | the most particles alive at once (openm2's reading) |
| `CycleLength` | seconds the emitter makes particles for |
| `CycleLoopEnable`, `LoopCount` | with the flag, the cycle runs `LoopCount` times, or for ever with 0 (and the −1 of one script); without it once |
| `EmitterShape` | where particles are born; see below |
| `EmitterAdvancedType` | which way they fly; see below |
| `EmitterEmitFromEdgeFlag` | born on the shape's edge, not inside it (1,112 emitters) |
| `EmittingRadius` | the radius of shapes 1 and 3 (cm) |
| `EmittingSize` | the box of shape 2 (cm) |
| `EmittingDirection` | how far a new particle's direction strays along x, y and z: a share of a thousand times each, either way, at random, added to the direction before the speed multiplies it; in the world's axes for a particle not attached (the original client's source). Three small numbers (0.001 to 0.1); 3,393 emitters have zeros |
| `TimeEventEmittingSize` | cm added to a circle's or sphere's particle's distance from the middle, and to each of a box's particle's coordinates, over the cycle (the original client's source; zero in most) |
| `TimeEventEmittingAngularVelocity` | degrees the emitter's particles turn about its up axis at each of the client's steps, 60 a second, round the points on the element's path they were born at; an attached particle about the effect's up, another about the effect's up as it is now (the original client's source) |
| `TimeEventEmittingDirectionX`, `Y`, `Z` | a new particle's direction… |
| `TimeEventEmittingVelocity` | …times this: its speed in cm/s (a hit's sparks: (0, −16, 0) × 6) |
| `TimeEventEmissionCountPerSecond` | particles born a second |
| `TimeEventLifeTime` | seconds a new particle lives |
| `TimeEventSizeX`, `TimeEventSizeY` | a new particle's size (cm): half its height and half its width facing the camera, half its width and half its height otherwise, as the original client draws it (its source) |

### Particles: `ParticleProperty`
| Key | What |
|---|---|
| `SrcBlendType`, `DestBlendType` | how it blends over what is behind it, by Direct3D's blend factor numbers; see below |
| `ColorOperationType` | how its texture and colour combine, by Direct3D's texture operation numbers; see below |
| `BillboardType` | how it turns; see below |
| `RotationType`, `RotationSpeed` (°/s), `RotationRandomStartingBegin`/`End` (°) | how it spins; see below |
| `AttachEnable` | it moves with the effect after it is born (2,344 emitters); otherwise it stays where it was born |
| `StretchEnable` | it lies along its last step (1,240): its Y size along the step, times one and the logarithm of one and the step's length in cm, its X size across as seen from the camera; straight up when it didn't move (the original client's source) |
| `TexAniType`, `TexAniDelay` (s), `TexAniRandomStartFrameEnable` | which of its textures it shows; see below |
| `TimeEventScaleX`, `Y` | its size times these over its life |
| `TimeEventColorRed`, `Green`, `Blue`, `TimeEventAlpha` | its colour and alpha over its life (0 to 1) |
| `TimeEventRotation` | degrees added over its life (rotation type 1) |
| `TimeEventGravity` | cm/s² it falls by (openm2's reading) |
| `TimeEventAirResistance` | the part of its speed it loses at each of the original client's steps, 60 a second (its source) |
| `List TextureFiles` | its textures, by file name beside the script or by game path; one in most (4,442 of 4,722 emitters) |

### Meshes: `Group Mesh` and `MeshElement##`
| Key | What |
|---|---|
| `MeshFileName` | the mesh effect file |
| `MeshAnimationFrameDelay` | seconds per frame of the file |
| `MeshAnimationLoopEnable`, `MeshAnimationLoopCount` | the frames play `LoopCount` times, for ever with 0, or once without the flag; then the element is gone |
| `MeshElementCount`, `Group MeshElement00`, … | how each mesh of the file is drawn, in the file's order. 4 scripts have fewer blocks than their file has meshes (`pc/assassin/effect/yeonsa_2.mse`: none for one mesh); openm2 draws those meshes blended by alpha in white |
| `BillboardType` | 0 (910 blocks) as made; 1 and 2 turned about the vertical to face the camera; 3 as made (openm2's reading) |
| `BlendingEnable`, `BlendingSrcType`, `BlendingDestType` | blending as for particles; without the flag drawn without blending (14 blocks) |
| `ColorOperationType`, `ColorFactor` | the texture combined with this colour (RGBA) |
| `TimeEventAlpha` | alpha over the element's time, times the frame's visibility and the colour's alpha |
| `TextureAnimationLoopEnable`, `TextureAnimationFrameDelay`, `TextureAnimationStartFrame` | kept; a mesh has one texture, and openm2 does not use them |

### Numbers that choose a behaviour
Counted over the client's 4,728 particle emitters and 982 mesh blocks. Except for the Direct3D numbers, the meanings are openm2's reading, from how the client's files use them (open questions: [`../../roadmap.md`](../../roadmap.md)).

**`EmitterShape`:**
| Value | Emitters | Reading |
|---|---|---|
| 0 | 2,520 | a point |
| 1 | 612 | a circle of `EmittingRadius` on the ground plane (all have a radius) |
| 2 | 557 | a box of `EmittingSize` (all have a size) |
| 3 | 1,039 | a sphere of `EmittingRadius` |

**`EmitterAdvancedType`:** 0 (3,752) flies along the direction curves; 1 (737) flies away from the shape's centre at 100 (a point's at random, up to 100 along each axis), 2 (239) towards it at 100, the direction curves added, all times the velocity (the original client's source).

**Where in the shape:** a circle's and a sphere's particle at a distance from the middle even between 0 and the radius (the radius from the edge), the growing added; a box's anywhere in it, the growing added to each coordinate (the original client's source).

**`BillboardType` (particles):**
| Value | Emitters | Reading |
|---|---|---|
| 0 | 570 | faces the camera. Used for smoke, glows and the metin stones' auras; no fixed orientation looked right for them |
| 1 | 2,837 | faces the camera |
| 2 | 282 | upright, turned about the vertical to face the camera |
| 3 | 886 | lies flat on the ground, turned with the effect: used by effects named for the floor (`bottom`, `badak`, `land`, `water_impact`) at heights near 0 |
| 4, 5 | 59, 94 | face the camera |

**`RotationType`:** 0 (2,625) does not spin; 1 (264) turns by `TimeEventRotation`; 2 (467) spins one way at `RotationSpeed`, 3 (95) the other way, 4 (1,277) either way at random. Each starts at a random angle between `RotationRandomStartingBegin` and `End`.

**`TexAniType`:** 0 (4,211) keeps the texture it starts with (the first, or a random one with `TexAniRandomStartFrameEnable`); 1 to 4 (54, 13, 188, 262) show the next texture every `TexAniDelay` seconds, round and round.

**Blend factors** (`SrcBlendType`, `DestBlendType`, `BlendingSrcType`, `BlendingDestType`), Direct3D's numbers: 1 zero, 2 one, 3 source colour, 4 one minus source colour, 5 source alpha, 6 one minus source alpha, 7 destination alpha, 8 one minus destination alpha, 9 destination colour, 10 one minus destination colour, 11 source alpha saturated, 12 source alpha for the source and one minus it for the destination (whatever the destination says), 13 the other way round. The client's particles use (5, 2) (added by alpha) for 3,954 emitters, (5, 6) (plain alpha blending) for 474, (3, 2) for 177, (5, 7) for 47, others for fewer than 25 each.

**`ColorOperationType`**, Direct3D's numbers: 2 the texture alone, 3 the colour alone, 4 texture times colour (4,531 emitters), 5 times 2 (188), 6 times 4, 7 added, 8 added less a half.

### What openm2 refuses
A value that is not a number where one is expected; `EmittingDirection`, `EmittingSize`, `BoundingSpherePosition` and `ColorFactor` with another count of numbers; a curve row that is not two numbers; a position row of an unknown type or the wrong count of numbers; a `TextureFiles` row that is not one name; `MeshElementCount` past the `MeshElement##` blocks there are. Each names the line and key.

## Mesh effects (`.mde`)
Small meshes animated frame by frame: rings, flags, waterfalls, the warp gates' swirls. Little-endian, two layouts told apart by the magic; z is up, as everywhere else.

**`EffectData`:**
| Offset | Size | What |
|---|---|---|
| 0 | 11 | `EffectData` and a NUL |
| 11 | 4 | meshes |
| 15 | 4 | frames, the same for every mesh |
| 19 | | the meshes, one after the other |

Each mesh:
| Size | What |
|---|---|
| 32 | its name in the modelling tool (`Sphere01`), NUL-terminated |
| 128 | its texture as the modelling tool saved it, NUL-terminated |
| 4, 4, 4 | positions, indexes and texture coordinates per frame |
| | the frames |

Each frame:
| Size | What |
|---|---|
| 4 | visibility (f32): 1 visible; the client's files go from −0.83 to 1.09, and below 0 is invisible |
| 12 each | positions (x, y, z, f32, cm) |
| 4 each | triangles: indexes into the positions, three per triangle |
| 8 each | texture coordinates (u, v, f32; v from the top) |
| 4 each | the triangles again, as indexes into the texture coordinates: a corner's position and texture coordinate are numbered separately |

**`MDEData002`:** the same, but with `MDEData002` and a NUL as the magic, no counts after the texture, and each frame starting with a flag byte (1 in every frame of the client), then the visibility and the frame's own three counts. The guild flags use it for cloth that moves. The name and texture fields of these files hold stray bytes after their NUL.

**Rules:**
- Counts are checked against the bytes left before anything is read; an index past its list, a count of indexes that is not a multiple of three, or bytes after the last mesh are damage.
- A frame flag other than 1 is a layout openm2 does not know.
- The writer refuses frames of one mesh with different counts in the first layout, and meshes with different numbers of frames.
- **In the client:** 112 files: 106 `EffectData`, 6 `MDEData002`; up to 25 meshes and 101 frames. Their textures are `.jpg`, `.tga` and `.dds` files named by the artist's path (`D:\Ymir Work\effect\etc\click\click_glow_select copy.jpg`); the client ships a `.dds` of the same name beside most of them.

## Flying effects (`.msf`)
What a motion's fly event shoots ([script.md](script.md), "Motion events", type 6): an arrow, a thrown axe, a spell; and what the client's scripts register for its server's flights, as the golden pieces a kill's experience flies as (`effect/etc/gathering/`). The script syntax, without a `ScriptType`; its keys at the top, then one or more `Group AttachData` blocks (up to 4 in one file), all with the same name:

```text
InitialVelocity         4000.000000
Acceleration            0.000000 0.000000 0.000000
BombRange               20.000000
BombEffect              "blow.mse"
HomingFlag              1
HomingMaxAngle          180.000000
Range                   5000.000000
Group AttachData
{
    Type          1
    FlyType       1
    AttachFile    "arrow.mse"
    TailFlag      1
    TailColor     1726079457d
    TailLength    0.180000
    TailSize      1.000000
    TailShapeRect 0
    Roll          0.000000
    Distance      0.000000
    Period        1.000000
    Amplitude     0.000000
}
```

| Key | In the client (of 98) | Values | openm2's reading |
|---|---|---|---|
| `InitialVelocity` | 98 | 3000 (30), 2000 (25), −100 (11), 500 (8), … | its speed forward when it leaves, cm/s; below 0 it first goes back |
| `Acceleration` | 98 | (0, −100, 0) (34), 0 (22), (0, −500, 0) (11), (0, −9000, 0) (8) | cm/s², in its own axes, where forward is −Y as in the client's models: (0, −9000, 0) speeds it forward |
| `Gravity` | 98 | 0 but in 2 (3, −5) | cm/s² added upwards to its velocity (the original client's source) |
| `HomingFlag`, `HomingStartTime`, `HomingMaxAngle` | 98, 97, 98 | homing 80; from 0 s (79) or 1 s; 180 (55), 3 (23), 5 (18) degrees | it turns its velocity and acceleration towards its target from that time, at most twice that many degrees in each of the client's steps, 60 a second; from 90 at once (the original client's source) |
| `Range` | 98 | 10,000 (49), 5,000, 100,000 (18) cm | how far it flies at most |
| `BombRange`, `BombEffect` | 98 | 10 (57), 40, 50, 20 cm; an effect in every file | it ends that near its target, and the effect plays there |
| `SpreadingFlag`, `ConeAngle`, `RollAngle` | 97, 98, 98 | spreading 11 (the `gathering` pieces, all with cone 90); cone 0 but 90 (11) and −90 (the shaman's `paeryong.msf`, with roll 90) | how it leaves turned: about its up axis by the cone, then about its own axis by the roll less 90°; spreading first turns it at random (the original client's source; [`../render.md`](../render.md), "Flying effects") |
| `MaintainParallelFlag` | 52 | mostly 0 | not read |
| `AngularVelocity` | 98 | 0 but (0, 1, 0), (0, 450, 0), (0, 300, 0) in 38 | degrees a second its attached effects turn about its own y, then x, then z (the original client's source) |
| `HitOnBackground`, `HitOnAnotherMonster`, `PierceCount`, `CollisionSphereRadius` | 93, 93, 93, 49 | 0 but 5 (1, 10) | not read: what it hits is the server's |
| `IgnoreZ` | 1 | 0 | not read |
| `AttachData` `Type` | 113 blocks | 1 in all | an effect |
| `AttachData` `AttachFile` | 113 | an effect script; 9 empty | the effect that flies with it, facing its way |
| `AttachData` `FlyType`, `Period`, `Amplitude`, `Distance`, `Roll` | 113 | types 1 (58), 2 (32), 3 (18), 4 (5); amplitudes 0 (64), 100, 150, 50 cm | where the effect is from the flying point (the original client's source): 1 on it; the others away from it along (−sin `Roll`, 0, −cos `Roll`) in its spun axes: 2 by `Distance` cm, 3 by `Amplitude` × sin(2π t / `Period`), 4 by `Amplitude` × u e^−u (u = t / `Period`) |
| `AttachData` `TailFlag` and `Tail…` | 113, 66 | tails 66; colours as ARGB in decimal with a `d`; lengths 0.15 to 1, sizes 1 to 7 | a tail behind the effect: a ribbon through where it was in the last `TailLength` seconds, `TailSize` cm to each side, narrowing to its end (not with `TailShapeRect`), in its colour added (the original client's source) |

- **Names:** most `AttachFile` and `BombEffect` values are a file name only; the effect is beside the flying effect (`find_fly_effect`: the name as written, else its file name beside the script). 201 of the 202 names are in the client (`trunk1.mse` of `monster2/trunk1_fly.msf` is not). An empty `BombEffect` is none.
- **Reading:** a missing key is 0 (an empty name none); `TailColor` is digits and a `d` (or digits alone); a value that is not the right count of numbers is an error naming its line. The tail's keys are read only with `TailFlag` 1.
- **Writing** (for test data): every key in the client's order, `IgnoreZ` only when set, the tail's keys only with a tail.
- **openm2's reading:** what the keys mean was read from their names and values; how it moves (velocity, acceleration, gravity, homing, the cone, roll and spreading as it leaves) was compared with the original client's source.

**In the client:** 98 files: firecrackers, the monsters' arrows and spells, and the characters' throwing skills. `script check` reads all of them.

## Files an effect names
`m2_assets::effect`:
- **Mesh files** (`MeshFileName`): the name as written (normalized), then its file name beside the script (`find_effect_mesh`). Some scripts name broken paths (`D:\pc\assassin\effect\energyup.mde`, without `ymir work`); the file beside the script is found. 47 of the 331 names are not in the client.
- **Textures** (`TextureFiles`, and the textures mesh files name): the same two places, and after each the same name ending in `.dds` if it ends otherwise (`find_effect_texture`). openm2 reads DDS and TGA textures, not JPEG. 27 of the 5,187 particle textures are not in the client (`affect/damagevalue/nontarget.mse` names `0.jpg` to `9.jpg`); every mesh texture is found.
- **Weapons' item scripts** (`item_script_path`): the item list's model with `.msm` in the folder above it (`d:/ymir work/item/weapon/00010.gr2` → `d:/ymir work/item/00010.msm`; openm2's reading).

## Tools
| Command | What it does |
|---|---|
| `openm2 script dump --client <dir> <file.mse>` | An effect script, one value per line |
| `openm2 script check --client <dir>` | Reads every effect script and mesh effect, counts the numbers above, and checks that the files they name are in the client |
| `openm2 view effect <file.effect.toml> --assets <dir> --screenshot <png> [--time S] [--frames N --duration S]` | Plays an effect `assets import` converted ([`../assets.md`](../assets.md), "Effects") and draws moments of it; a flying effect (`.fly.toml`) flies at a target `--target` metres ahead ([`../render.md`](../render.md), "Flying effects") |
| `openm2 map info <map> --client <dir>` | Which effect scripts a map's objects play, and where the first of each is |
