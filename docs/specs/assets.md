# Asset folders

An asset folder holds openm2's own formats, in place of a client folder: glTF models and animations ([`formats/gltf.md`](formats/gltf.md)), PNG images, openm2's TOML for actors, motions and maps, WAV and MP3 sounds. It has no archive and no key. `play` and the `view` commands read only asset folders (`--assets DIR`), so what they draw and play needs no Metin2 file at run time: a player's client reaches them only through `openm2 assets import`, which converts it on the player's machine (the launcher does it for players, [`update.md`](update.md)).

An asset folder holds **actors**: characters and monsters with their models, textures, motions and sounds, which `openm2 view model --assets` draws; **maps**: terrain, water, buildings with their floors, trees, the sky and the sound spots, which `openm2 view map --assets` draws; **effects**: particles and meshes played frame by frame, which `openm2 view effect --assets` draws; and **what `play` needs** besides them: the interface's skin and art, the item table, the skills, the monsters' names, the music, the sounds and effects of a hit. `openm2 play --assets` plays with it, `openm2 view ui --assets` draws its interface, a channel's version that names one starts without a client ([`client.md`](client.md), [`update.md`](update.md)), and the launcher imports a player's client into one and plays with that ([`update.md`](update.md), "What the launcher does").

**Code:** `m2_assets::folder` (paths and references, `AssetFolder`, `write_file`, `assets.toml`, the import's markers), `m2_assets::actor_file` (actor and motion files), `m2_assets::effect_file` (effect files), `m2_assets::map_file` (map and area files), `m2_assets::skin_file` (the skin and images), `m2_assets::items_file` and `m2_assets::skills_file` (the item and skill tables); `openm2 assets import`, `assets check` and `assets current` (`crates/m2-cli/src/assets.rs`, what `play` needs in `assets/play.rs`); `view model --assets` (`crates/m2-cli/src/view.rs`); `view map --assets` (`crates/m2-cli/src/mapview.rs`, the rasters in `map_folder.rs`); `play --assets` and `view ui --assets` through one source of the game's files (`crates/m2-cli/src/source.rs`).

## Paths and references
- **A path** inside the folder is text, names between `/`, as on disk. No name is empty, `.` or `..`, has a control character, `\` or one of `<>:"|?*`, ends in `.` or a space, is a Windows device name (`con`, `nul`, `com1`, …) or is over 255 bytes; a path is at most 1,024 bytes (`m2_assets::folder::check_path`).
- **A reference** is how a file names another: relative to its own folder (`run.glb`, `../common/x.png`), or from the folder's root when it starts with `/` (`/sound/common/walk.wav`). `.` and `..` are resolved; a reference that would leave the folder is refused (`resolve`).
- **glTF files** name their images by a relative `uri`, `%XX` encoded ([`formats/gltf.md`](formats/gltf.md)).

### Images
The skin, the item table and the skills name an image as a reference to a PNG, or to a **region** of one, written as in W3C's Media Fragments: `#xywh=` and its left and top edges, width and height in pixels.

```toml
icon = "icon/item/00010.png"                    # the whole picture
normal = "/ui/public.png#xywh=61,0,61,21"       # 61 × 21 pixels from (61, 0)
```

Width and height are at least 1. A region past the picture's edge is cut to it when drawn, as the client's sub-images are; `assets check` reports it.

## `assets.toml`
At the root: what the folder is, what `play` shows with, its actors and its maps.

```toml
format = 1
name = "imported"
import = 4                                 # only in an import: the number of the import that wrote it
skin = "skin.toml"                         # optional, each of these: the interface's skin
items = "items.toml"                       # the item table
skills = "skills.toml"                     # the skills
hit_sounds = ["sound/common/hit/hit_sword_1.wav"]   # the sounds of a hit, taken in turn
potion_sound = "sound/ui/eat_potion.wav"   # optional: our character drinking a potion

[music]                                    # optional: each screen's MP3 file, played streamed; none plays without
login = "bgm/login_window.mp3"
select = "bgm/characterselect.mp3"         # the select and create screens
game = "bgm/m2bg.mp3"                      # on every map

[effects]                                  # optional: the effects play shows of itself, each optional; none without
hit = "ymir work/effect/hit/blow_1/blow_1.effect.toml"        # on what a hit strikes
critical = "ymir work/effect/hit/blow_3/15.effect.toml"       # on what a critical hit strikes
item = "ymir work/effect/etc/dropitem/dropitem.effect.toml"   # over each item on the ground
poisoned = "ymir work/effect/hit/blow_poison/poison_loop.effect.toml"  # looping on a poisoned entity
slowed = "ymir work/effect/affect/slow.effect.toml"           # on a slowed one
stunned = "ymir work/effect/etc/stun/stun_loop.effect.toml"   # on a stunned one
burning = "ymir work/effect/hit/blow_flame/flame_loop.effect.toml"  # on a burning one
hovered = "ymir work/effect/etc/click/click_select.effect.toml"        # at the feet of what the pointer is over
targeted = "ymir work/effect/etc/click/click_glow_select.effect.toml"  # at the feet of our target
health_potion = "ymir work/effect/etc/recuperation/drugup_red.effect.toml"   # once on a character a potion gave health back
mana_potion = "ymir work/effect/etc/recuperation/drugup_blue.effect.toml"    # once on one it gave mana back
level_up = "ymir work/effect/etc/levelup_1/level_up.effect.toml"             # once on one that gained a level
fish_bite = "ymir work/effect/etc/emoticon/fish.effect.toml"                 # once over a fisher a fish bit

[numbers]                                  # optional: damage numbers, each optional; none show without
ours = ["ymir work/effect/affect/damagevalue/damage.effect.toml", "ymir work/effect/affect/damagevalue/damage_1.effect.toml"]
                                           # a digit of a hit on our character, the next effect for each hit
ours_digits = ["ymir work/effect/affect/damagevalue/damage_0.png", "…", "ymir work/effect/affect/damagevalue/damage_9.png"]
                                           # its pictures of 0 to 9, all ten or none, in place of the effect's own
target = "ymir work/effect/affect/damagevalue/target.effect.toml"      # a digit of a hit on our target
target_digits = ["ymir work/effect/affect/damagevalue/target_0.png", "…", "ymir work/effect/affect/damagevalue/target_9.png"]
miss = "ymir work/effect/affect/damagevalue/miss.effect.toml"          # a hit on our character that missed
target_miss = "ymir work/effect/affect/damagevalue/target_miss.effect.toml"  # a hit on another that missed

[flies]                                    # optional: what play flies of itself, each optional; nothing without
experience = "ymir work/effect/etc/gathering/ga_piece_yellow_small2.fly.toml"   # from a monster killed to who gained its experience

[shine]                                    # optional: the pictures shining items reflect, each optional; none shine without
armour = "ymir work/special/spheremap.png"    # on a body armour's own textures
weapon = "ymir work/special/spheremap01.png"  # on a weapon's

[[effects.upgraded]]                       # optional, repeated: the glow of worn items upgraded to `level` or more, each file optional
level = 7
armour = "ymir work/pc/common/effect/armor/armor_7.effect.toml"   # on a body armour, from the pelvis
sword = "ymir work/pc/common/effect/sword/sword_7.effect.toml"    # on a weapon, along it in each hand, by its kind:
dagger = "ymir work/pc/common/effect/sword/sword_7_s.effect.toml" # sword, dagger, bow, two_handed, bell, fan

[[player]]
race = 0                                   # as the server sends it: 0 warrior, 1 ninja, 2 sura, 3 shaman, 4 to 7 the other gender
actor = "warrior_m.actor.toml"

[[monster]]
vnum = 101                                 # the monster's number
name = "Wild Dog"                          # optional: as the target window shows it
level = 1                                  # optional
actor = "ymir work/monster/stray_dog/stray_dog.actor.toml"   # optional: without one, it isn't drawn

[[map]]
map = "metin2_map_a1/map.toml"
at = [4096.0, 8960.0]                      # optional: where its top left corner is in the world, metres east and south
                                           # (as the server gives positions); without it, the map is in no place of the world
size = [4, 5]                              # optional: the areas the world finds it by, when not the map's own size
```

A map can be in the world more than once (the client's atlas lists one map at four places): a `[[map]]` for each place.

`import` is written by `openm2 assets import` alone: its number, raised whenever it writes something else from the same client, so a launcher knows to import a player's client again (`assets current`, below).

Every file of the folder is TOML of `format = 1`; unknown keys are refused, with the line.

## Actors
An actor file (`*.actor.toml`) says what the client's model script (`.msm`) and motion list say ([`formats/script.md`](formats/script.md), "Race data" and "Motion lists"). **Lengths are metres and angles degrees, in the model's axes as the renderer has them: +Z up, forward −Y** (as Blender shows a model). glTF files have their own axes, turned when read.

```toml
format = 1
model = "warrior_novice.glb"            # the model drawn before a shape is chosen, or the only one

[[shape]]                               # a body, chosen by the shape index (armours)
index = 1
model = "warrior_novice.glb"
skins = [{ source = "warrior_novice_red.png", target = "warrior_novice_blue.png" }]
                                        # source: the file name of a texture the model shows (case ignored);
                                        # target: the texture drawn instead

[[hair]]                                # a hair style, chosen by the hair index; like a shape
index = 1
model = "hair/hair_1_1.glb"

[[collision]]                           # collision spheres
kind = 3                                # the client's CollisionType (1 and 3; what each is for is open)
bone = "Bip01"                          # placed from this bone; without it, from the model's origin
spheres = [{ radius = 0.9, at = [0.0, -0.1, 0.8] }]

[[effect]]                              # an effect playing for as long as the actor shows
bone = "Bip01 Head"                     # optional, as for collisions
file = "/ymir work/effect/glow.effect.toml"
at = [0.0, 0.0, 0.2]
turn = [0.0, 90.0, 0.0]                 # optional: degrees about x, y and z

[smoke]                                 # a metin stone's smoke: an effect per stage, on a bone
bone = "Bip01"
stages = [{ stage = 0, file = "/ymir work/effect/smoke_0.effect.toml" }]

[[motion]]
set = "general"                         # players: the motion's folder (general, onehand_sword, skill, …); monsters: the list's mode (GENERAL)
name = "run"                            # players: the motion script's name (run, combo_01); monsters: the list's (RUN, WAIT1)
file = "ymir work/pc/warrior/general/run.motion.toml"
weight = 100                            # optional, 1 by default: how often it is picked among a motion's numbered variants
```

Actors and motions name effect files ([below](#effects)).

## Motions
A motion file (`*.motion.toml`) says what a motion script (`.msa`) and its sound script (`.mss`) say ([`formats/script.md`](formats/script.md), "Motion data"; [`formats/sound.md`](formats/sound.md)). Times are seconds into the motion; lengths metres, in the model's axes.

```toml
format = 1
animation = "combo_01.glb"              # the first animation of the file
duration = 1.0                          # optional: its length, when not the animation's
moves = [0.0, -1.205, 0.0]              # optional: how far it moves the actor (walking, running, attacking)

[combo]                                 # optional: the combo window
pre_input = 0.167094
direct_input = 0.533333
input_limit = 0.602564
link = 0.7                              # optional

[attack]                                # optional: how it hits (what most numbers mean is open)
attack_type = 0                         # optional (the newer layout)
motion_type = 2                         # optional (the newer layout)
hitting_type = 2
stiffen_time = 0.0
invisible_time = 0.1
external_force = 8.0
hit_limit = 3                           # optional
                                        # attacking_type: the older layout's, with exactly one hit

[[attack.hit]]
start = 0.192308
end = 0.315385
bone = "equip_right_hand"               # optional: none for none
weapon_length = 1.3
positions = [[0.192308, -0.4699, -0.1397, 1.4668, -0.5427, 1.0874, 1.8898]]
                                        # rows: a time, the bone's point, the weapon's tip

[[event]]                               # what happens at given times, in order; `kind` and its keys:
kind = "effect"                         #   effect, spot-effect: time, file, at, bone?, following?, independent?, fishing?
time = 0.1                              #   shake: time, during, power, range
file = "/ymir work/effect/swing.effect.toml"   # attack: time, during, enable_hit?, attack_type?, hitting_type, stiffen_time,
at = [0.0, 0.0, 1.0]                    #           invisible_time, external_force, hit_limit?, collision_type, spheres
bone = "equip_right_hand"               #   fly: time, file, at, bone?
following = true                        #   other: time, number (the client's MotionEventType)

[[sound]]                               # sounds, in time order
time = 0.198
file = "/sound/common/walk_dirt_n.wav"
```

Both convert to the client's types and back (`ActorFile::to_race`, `MotionFile::to_motion`), so the import writes them from the client's scripts, and `play` and `view model` read them back into the types they draw. Numbers are written with the fewest digits that read back as the same value. A `fly` event names a flying effect file ("Flying effects"); the import leaves out one whose file didn't convert.

## Effects
An effect file (`*.effect.toml`) says what the client's effect script (`.mse`) and its sound script say ([`formats/effect.md`](formats/effect.md)): particle emitters and meshes played frame by frame, drawn in their order, and the sounds it starts. **Lengths are metres, speeds metres a second and angles degrees, in the effect's axes as the renderer has them: +Z up, forward −Y.** Colours and alphas go from 0 to 1. What the client's scripts choose by a number (shapes, blending, billboards) is a word, and numbers that draw the same are one word. Curves are rows of a time and a value, straight lines between them, flat before the first and after the last.

```toml
format = 1
radius = 2.0                            # optional: its bounding sphere, which `view effect` frames
centre = [0.0, 0.0, 1.2]                # optional: its centre, put where a hit strikes

[[element]]
kind = "particles"                      # an emitter of particles
start = 0.0                             # optional: seconds after the effect starts

[[element.path]]                        # optional: where the emitter is; at the origin without
time = 0.0                              # seconds after the element starts
at = [0.0, -0.1, 1.2]
bezier = [0.0, -0.5, 0.25]              # optional: it leaves here along this vector, on a curve to the next

[element.emitter]                       # its curves' times are seconds into its cycle
most = 10                               # the most particles alive at once
cycle = 0.5                             # seconds a cycle makes particles for
cycles = 0                              # optional: how many cycles, 0 for ever; 1 when left out
shape = "sphere"                        # point, circle (on the ground), box or sphere; point when left out
radius = 0.3                            # a circle's or a sphere's; size = [x, y, z] a box's
from_edge = true                        # born on the shape's edge
flight = "outward"                      # along the direction curves (when left out), outward or inward
rate = [[0.0, 80.0], [0.5, 0.0]]        # particles born a second
life = [[0.0, 0.75]]                    # seconds a new particle lives
width = [[0.0, 0.2]]                    # its size, metres; and height
direction_y = [[0.0, -16.0]]            # its direction: direction_x, direction_y, direction_z…
speed = [[0.0, 0.06]]                   # …times this, metres a second
spread = [0.005, 0.005, 0.0]            # optional: how far the direction strays, a thousand times it either way at most
                                        # and grow (metres added to the shape), turn (degrees its particles turn about
                                        # up at each of 60 steps a second)

[element.particle]                      # its curves' times are fractions of the particle's life, 0 to 1
blend = ["source_alpha", "one"]         # the source's and the destination's factors
combine = "modulate"                    # texture, colour, modulate, modulate_2x, modulate_4x, add or add_signed
facing = "camera"                       # camera, upright (turned about the vertical) or ground (flat, turned with the effect)
spin = "either"                         # none, curve, anticlockwise, clockwise or either
spin_speed = 180.0                      # degrees a second
start_angle = [-45.0, 45.0]             # degrees, picked between
textures = ["spark.png"]                # an empty name draws white
texture_every = 0.1                     # optional: the next texture every this many seconds; random_texture to start at any
alpha = [[0.0, 1.0], [1.0, 0.0]]        # and red, green, blue, scale_x, scale_y, rotation (degrees, with spin = "curve"),
                                        # gravity (metres a second squared), air_resistance (the part of its speed lost a second)
attached = true                         # it moves with the effect after it is born; stretched: along its last step,
                                        # longer the faster it goes (formats/effect.md, `StretchEnable`)

[[element]]
kind = "mesh"                           # meshes played frame by frame
file = "ring.glb"                       # a mesh effect: formats/gltf.md, "Mesh effects"
frame_time = 0.02                       # seconds a frame shows
plays = 0                               # optional: how many times its frames play, 0 for ever; 1 when left out

[[element.part]]                        # how each mesh of the file is drawn, in its order
facing = "upright"                      # fixed (as made, when left out) or upright (turned to face the camera)
blend = ["source_alpha", "one_minus_source_alpha"]   # optional: drawn without blending without
colour = [1.0, 0.5, 0.0, 0.75]          # optional: white when left out
alpha = [[0.0, 1.0], [0.4, 0.0]]        # over the element's time, seconds; times each frame's visibility

[[sound]]                               # sounds, in time order
time = 0.0                              # seconds after it starts
file = "/sound/effect/swish.wav"
```

- **Blend factors** are Direct3D's and wgpu's: `zero`, `one`, `source_colour`, `one_minus_source_colour`, `source_alpha`, `one_minus_source_alpha`, `destination_alpha`, `one_minus_destination_alpha`, `destination_colour`, `one_minus_destination_colour`, `source_alpha_saturated`. The client's 12 and 13, which set both from the source, are `source_alpha` with `one_minus_source_alpha` and the other way round.
- **One word for what draws the same** ([`formats/effect.md`](formats/effect.md), "Numbers that choose a behaviour"): billboards 0, 1, 4 and 5 face the camera; texture animations 1 to 4 show the next texture in turn; an unknown blend factor is `one` and an unknown texture operation `modulate`, as the renderer draws them.
- **Left out:** `EmittingDirection`, which openm2 doesn't use (what it does is unknown), and a mesh's texture animation, which has one texture.
- Meshes past an element's parts are blended by their alpha, in white, as the client's are.

It converts from the client's types and back (`EffectFile::from_effect`, `EffectFile::to_effect`), so the import writes them from the client's effect scripts, and `play`, `view map` and `view effect` read them back into the types they play.

## Flying effects
A flying effect file (`*.fly.toml`) says what the client's flying effect script (`.msf`, [`formats/effect.md`](formats/effect.md), "Flying effects") says that openm2 flies ([`render.md`](render.md), "Flying effects"). **Lengths are metres, speeds metres a second and angles degrees, in its own axes as the renderer has them: +Z up, forward −Y.**

```toml
format = 1
speed = 40.0                            # metres a second forward when it leaves (below 0: back)
acceleration = [0.0, -1.0, 0.0]         # optional: m/s², its own axes (−Y speeds it up)
gravity = 0.0                           # optional: the script's Gravity, as it is (not played)
range = 50.0                            # metres it flies at most
bomb_range = 0.2                        # metres from its target it ends
bomb = "blow.effect.toml"               # optional: the effect played where it ends
spread = 90.0                           # optional: it leaves turned at random, and by this cone, degrees (SpreadingFlag, ConeAngle)
cone = -90.0                            # optional, without spread: it leaves turned by this cone, degrees (ConeAngle)
roll = 90.0                             # optional: the script's RollAngle, degrees

[homing]                                # optional: it turns towards its target
from = 0.0                              # optional: seconds after it leaves
turn = 600.0                            # degrees a second at most: 120 times the script's HomingMaxAngle

[[attach]]                              # the effects that fly with it
effect = "arrow.effect.toml"
path = "beside"                         # optional: straight (default), beside, sway or swell
amplitude = 1.0                         # optional: metres it sways or swells out
period = 0.1                            # optional: seconds a sway takes, or a swell to its farthest
roll = 270.0                            # optional: degrees it is turned about the flying point's axis
distance = 0.2                          # optional: metres it is beside the flying point

[attach.tail]                           # optional: its tail
colour = [0.88, 0.88, 0.88, 0.4]        # red, green, blue, alpha, 0 to 1
length = 0.18
size = 1.0
rect = false                            # optional
```

- **Words:** fly types 1 `straight`, 2 `beside`, 3 `sway`, 4 `swell`, placed as the original client places them ([`render.md`](render.md), "Flying effects"). The tail's `length` is seconds and its `size` centimetres to each side, the script's as they are.
- **Spin:** `spin = [x, y, z]` (optional) is the script's `AngularVelocity`, degrees a second its attached effects turn about its own axes.
- **Turning:** `turn` is degrees a second; the client turns a homing one at most twice its `HomingMaxAngle` in each of its steps, 60 a second (the original client's source), so the import writes 120 times it (180 as 21,600: at once). How `spread`, `cone` and `roll` turn it as it leaves: [`render.md`](render.md), "Flying effects".
- **Left out:** what openm2 doesn't read: `MaintainParallelFlag`, `HitOnBackground`, `HitOnAnotherMonster`, `PierceCount`, `CollisionSphereRadius`, `IgnoreZ`, `AttachData`'s `Type` (1 in every file), and an attached effect without a file.
- Both convert to the client's type and back (`FlyFile::from_flying`, `FlyFile::to_flying`), what's left out as 0.

## Maps
A map is a folder: its `map.toml`, and a folder per area beside it, `{x:03}{y:03}` (x first: `001002` is area x 1, y 2), with the area's rasters as PNG and its `area.toml`. It says what the client's map folder says ([`formats/map.md`](formats/map.md)), the texture set and the environment script with it. **Lengths are metres and angles degrees, in the map's axes as the renderer has them:** x east and y north of the map's top left corner, so a map lies in negative y, and z up (as Blender shows a map put there). Colours are RGBA, from 0 to 1.

```text
metin2_map_a1/
  map.toml
  atlas.png               the whole map from above, north up (optional, map.toml names it)
  000000/
    heights.png  tiles.png  shadows.png  water.png  attributes.png  minimap.png  area.toml
  001000/
    …
```

### `map.toml`
```toml
format = 1
size = [4, 5]                           # areas along x (east) and y (south), 1 to 1,000 each
cell = 2.0                              # metres a cell's side; an area is 128 cells a side
height_step = 0.005                     # metres a step of heights.png
atlas = "atlas.png"                     # optional

[[texture]]                             # terrain textures, by the number tiles.png gives them (1 to 255, once each)
number = 1
file = "/ymir work/terrainmaps/a/grass.png"
repeat = [6.4, 6.4]                     # metres one repeat of it covers along x and y

[light]                                 # optional, like the tables below
direction = [0.5, 0.5, -0.707107]       # the way sunlight travels
background = { diffuse = [1.0, 1.0, 1.0, 1.0], ambient = [0.3, 0.3, 0.3, 1.0] }   # on the terrain and objects
character = { diffuse = [1.0, 1.0, 1.0, 1.0], ambient = [0.4, 0.4, 0.4, 1.0], enabled = false }
                                        # on characters; `enabled` is true when left out

[material]                              # what the light falls on
diffuse = [1.0, 1.0, 1.0, 1.0]
ambient = [1.0, 1.0, 1.0, 1.0]
emissive = [0.0, 0.0, 0.0, 1.0]

[fog]
near = 50.0                             # metres from the eye where it starts
far = 200.0                             # where everything is fog
colour = [0.7, 0.75, 0.85, 1.0]         # enabled = false switches it off

[sky]
size = [35.0, 35.0, 35.0]               # metres
upper_bands = 1                         # colour bands above the horizon
lower_bands = 1                         # and below it
gradient = [[0.09, 0.29, 0.66, 0.0], [0.85, 0.9, 1.0, 0.0], [0.85, 0.9, 1.0, 0.0], [0.53, 0.56, 0.62, 0.0]]
                                        # two colours a band, its top then its bottom, from the highest band down
textured = true                         # optional: drawn with the pictures below instead of the bands
faces = { front = "front.png", top = "top.png" }   # front, back, left, right, top, bottom; each optional
clouds = { file = "/ymir work/environment/clouds.png", height = 200.0, size = [5000.0, 5000.0], repeats = [4.0, 4.0], speed = [0.001, 0.002] }
                                        # metres, metres, repeats of the picture across the layer, repeats a second

[[mark]]                                # points the map's atlas marks
kind = "npc"                            # npc, warp, or another word
at = [430.0, -608.0]                    # metres, the map's axes
```

- **A texture's repeat** is in metres. The client's texture sets say how often a texture repeats with a unit of their own; openm2 draws scale s as a repeat of 32 m / s (`render.md`, "Terrain"), and the import writes that.
- **The light, material, fog and sky** are the client's environment script ([`formats/script.md`](formats/script.md), "Environment") but its lens flares and screen filter, which openm2 doesn't draw. A map with none of them is drawn with openm2's own light.
- **Marks** have no names (the client's are Korean in every locale, and nothing shows them).

### An area's rasters
Each is a PNG beside `area.toml`, named so; an area is there when it has `heights.png`. Rows go from the area's north-west corner, x first.

| File | PNG | Sides | What |
|---|---|---|---|
| `heights.png` | 16-bit grey | 131 × 131 | the terrain's height at each vertex, in steps of `height_step`: the area's 129 × 129 vertices and a border of one more on every side, which repeats the neighbouring areas' edges (normals at the edges come from it), as `height.raw` |
| `tiles.png` | 8-bit grey | 258 × 258 | which terrain texture covers each tile (half a cell), by its `number`; 0 none; the 256 × 256 tiles and a border, as `tile.raw` |
| `shadows.png` | any | (256 × 256) | the shadows baked onto the terrain, multiplied with it; white is none. Optional |
| `water.png` | 8-bit grey | (128 × 128) | the water layer each cell is under, from 0, as `area.toml`'s `water` lists them; 255 (or a layer past the list) is none. Optional |
| `attributes.png` | 8-bit grey | (256 × 256) | flags per square half a cell wide, as `attr.atr` ([`formats/map.md`](formats/map.md), "Attributes"): `0x01` blocked, `0x02` water. Optional; `play` will build the walls from it |
| `minimap.png` | any | (256 × 256) | the area from above, north up, for the minimap. Optional |

Sides in brackets are the client's; others are read too, up to 1,024 a side. Heights and tiles must have theirs, and must be grey of their depth: other kinds are refused by name.

### `area.toml`
Optional: an area without it has no water, objects or sound spots.

```toml
format = 1
water = [81.92]                         # each water layer's height, metres

[[object]]                              # a model: a building, a dungeon's walls, a rock
model = "/ymir work/zone/a/building/house.glb"
at = [12.0, -26.0, 150.0]               # where it stands, metres
turn = [0.0, 0.0, 90.0]                 # optional: degrees about x, y and z, turned about z first, then y, then x
raise = 0.5                             # optional: metres it stands over `at` (the client's height offset)

[[object]]                              # a tree, grown from one of openm2's recipes
tree = "beech"
season = "fall"                         # optional: summer, fall, winter or flowers; summer when left out
at = [30.0, -220.0, 10.0]

[[object]]                              # an effect, played round and round
effect = "/ymir work/effect/etc/fire.effect.toml"
at = [100.0, -40.0, 12.0]

[[sound]]                               # a sound heard around a spot, over and over
file = "/sound/ambience/waterfall.wav"
at = [128.0, -128.0, 10.0]
range = 64.0                            # how far it is heard, metres
full = 0.3                              # the part of the range, from the spot, at full volume
```

- **An object is one thing:** exactly one of `model`, `tree` and `effect`; a `season` only with a tree. Anything else is refused.
- **A model's floors** (what characters stand on: a platform, a bridge's deck) are in its glTF file, as nodes marked `openm2_floor` ([`formats/gltf.md`](formats/gltf.md), "Floors"). Every scene of a model's file is drawn where the object stands, as every model of a GR2 file is.
- **A tree's** shape and turn come from its recipe and the spot it stands on, as they do on the client's maps: the same spot always grows the same tree (`render.md`, "Maps: trees").
- **Effects** are effect files ([above](#effects)), drawn as the client's are (`render.md`, "Effects").

Both files convert from the client's (`MapFile::from_client`, `ObjectFile::placed`) and back (`MapFile::environment`, `AreaFile::water_steps`, …), so the import writes them from the client's maps, and `view map` and `play` read them back into the types they draw.

## What `play` needs
Besides actors and maps, `play` shows the interface, items, skills, monsters' names and the effects of hits and items, and plays music; `assets.toml` names the files (above). Their text is UTF-8. Each converts to what the client's own files give (`ItemsFile::to_items`, `SkillsFile::to_skills`, the skin to `m2_ui::Skins`), so `play` shows the client's and an asset folder's with the same code.

### The interface's skin
The skin file (`skin.toml` in an import) is the interface's skin file ([`ui.md`](ui.md), "The skin file") in TOML, with images (above) in place of game paths. It has every look openm2's layouts and the minimap use, by the same names ([`ui.md`](ui.md)); the layouts stay openm2's own. A look it lacks (a folder imported before the look was added) is the built-in skin's in its fallback colours, and `play` notes how many: the look's art comes with the next import.

```toml
format = 1
text_colour = [232, 228, 214, 255]      # RGBA, 0 to 255, as the built-in skin's
tooltip = "tooltip"                     # optional: the look behind tooltips, a panel's

[font]                                  # the first family installed is used; openm2 ships no fonts
families = ["Tahoma", "Verdana", "Arial"]
size = 12.0                             # pixels

[look.board]                            # a look: its kind and that kind's keys
kind = "frame"                          # colour, frame, bar, image, tile, button or gauge
fill = "ui/pattern/board_base.png"
fallback = [40, 32, 24, 240]

[look.board.corners]                    # top_left, top_right, bottom_left, bottom_right
top_left = "ui/pattern/board_corner_lefttop.png"

[look.board.edges]                      # top, bottom, left, right
top = "ui/pattern/board_line_top.png"

[look.button]
kind = "button"                         # normal, hover, down, text_colour, fallback
normal = "ui/public.png#xywh=0,0,61,21"
hover = "ui/public.png#xywh=61,0,61,21"
down = "ui/public.png#xywh=122,0,61,21"
text_colour = [255, 255, 255, 255]
fallback = [96, 72, 48, 255]

[look.chat_back]
kind = "colour"
colour = [0, 0, 0, 96]
```

| Kind | Keys | Needs |
|---|---|---|
| `colour` | `colour` | `colour` |
| `frame` | `corners`, `edges`, `fill`, `fallback` | `fallback` |
| `bar` | `left`, `middle`, `right`, `fallback` | `fallback` |
| `image`, `tile` | `image`, `fallback` | `fallback` |
| `button` | `normal`, `hover`, `down`, `text_colour`, `fallback` | `text_colour`, `fallback` |
| `gauge` | `slot` (another look), `fill`, `inset` (left, top, right, bottom), `fallback` | `fallback` |

An image left out draws the look's fallback colour, as the client's missing art does. Keys of another kind, a kind openm2 doesn't draw, a slot that isn't a look and a tooltip that isn't a panel's are refused. A folder whose `assets.toml` names no skin is drawn with the built-in skin's fallback colours.

### Items
The item table (`items.toml`) says what the client's item table and item list say of each item: what the interface shows and how characters wear and hold it.

```toml
format = 1
bag = "ymir work/item/etc/item_bag.glb" # optional: what an item without a model shows on the ground

[[item]]
vnum = 11299
name = "Black Steel Armour+9"
icon = "icon/item/11290.png"            # optional: an image
model = "ymir work/item/weapon/..."     # optional: a .glb, what lies on the ground and a weapon in its holder's hand
size = 2                                # cells tall in the bag, 1 to 3; 1 when left out
kind = 2                                # the client's numbering, as the content's items (server/content.md, "Item"): 1 a weapon, 2 an armour
subtype = 0                             # a weapon's: 0 sword, 1 dagger, 2 bow, 3 two-handed, 4 bell, 5 fan
wear_flags = 1                          # where it is worn: 1 body, 2 head, 4 shoes, …
shape = 3                               # optional, for what is worn on the body: the actor's [[shape]] it dresses in
level = 54                              # optional: the lowest level that may use it
buy_price = 4000                        # what a shop asks, without a price of its own
sell_price = 400
glow = 7                                # the level of the glow it shows worn; 0 when left out
shine = 100                             # how much it shines worn, 0 to 100; 0 when left out
points = 10                             # a fishing pole's most points, for its tooltip; 0 when left out
```

Kinds, subtypes and wear flags keep the client's numbering, as the content's items do: the server and the client must mean the same thing. Numbers left out are 0. An item number twice, an empty name, a size past 1 to 3 and a shape on what isn't worn on the body are refused.

### Skills
The skills file (`skills.toml`) says what the client's skill descriptions and skill table say of each skill: what the character window's Skills tab shows and `play` checks before using one ([`ui.md`](ui.md), "Skills").

```toml
format = 1

[[group]]                               # optional: a job's skill group's name, as the Skills tab shows it
job = "warrior"
group = 1                               # 1 or 2
name = "Body"

[[skill]]
vnum = 1
job = "warrior"                         # warrior, ninja, sura, shaman, or another word for any job's (support, horse, guild)
name = "Three-Way Cut"
description = "Slash three times."      # optional, like the rest
notes = ["Attack on Multiple Targets"]  # up to four lines
flags = ["attack_skill", "weapon_limitation"]   # the client's words in lower case: need_target, can_use_for_me, …
weapons = ["sword", "two_handed"]       # with weapon_limitation: sword, dagger, double_sword, bow, two_handed, bell, fan
icons = ["ui/skill/warrior.png#xywh=0,0,32,32", "ui/skill/warrior.png#xywh=32,0,32,32", "ui/skill/warrior.png#xywh=64,0,32,32"]
                                        # by grade: normal, master, grand master; a grade past the last shows the last
motion = 1                              # its motion number, as the server sends it in a move
motions = "samyeon"                     # its motions in a player actor's `skill` set: samyeon, samyeon_2 to samyeon_4
                                        # for master to perfect master, samyeon_me used on oneself
cooldown = "20+40*k"                    # seconds, a formula (server/content.md, "Formulas")
sp_cost = "40+100*k"                    # its mana, a formula
target_range = 8                        # metres its target may be; left out for a hit's reach

[skill.look]                            # optional: how it looks on a character while it is on
effect = "ymir work/pc/warrior/effect/geom_sword_loop.effect.toml"     # round and round
two_handed = "ymir work/pc/warrior/effect/geom_spear_loop.effect.toml" # optional: instead with a two-handed weapon
bone = "equip_right_hand"               # optional: the bone it plays on, turning with it; from the feet without
moving = false                          # optional: true to show it only while the character moves
```

A character plays, for a skill at a grade, the first of its motions its actor has: the grade's own, then the lower grades', then the higher grades', then `_me`, as the client's motion scripts are found.

### Monsters, music and sounds
- **Monsters:** `assets.toml`'s `[[monster]]` gives each monster's name and level (the target window's) and its actor; a monster with a name and no actor is named but not drawn.
- **Music:** `[music]` names each screen's piece; a screen without one plays none.
- **A hit** plays `hit_sounds` in turn; none without them. Our character's potions play `potion_sound`; none without it.
- **Effects:** `[effects]` names what a hit, a critical hit and each item on the ground show, the loops of a poisoned, slowed, stunned and burning entity (`poisoned`, `slowed`, `stunned`, `burning`), the rings under what the pointer is over and under our target (`hovered`, `targeted`), what a potion's health and mana and a level gained show once on a character (`health_potion`, `mana_potion`, `level_up`), and the fish over a fisher a fish bit (`fish_bite`) ([`render.md`](render.md), "Effects"); none without. `[flies]` names what `play` flies of itself: `experience`, from a monster killed to who gained it ([`render.md`](render.md), "Flying effects"); nothing flies without. `[numbers]` names the damage numbers' effects and their digits' pictures ([`render.md`](render.md), "Our fighting"); none show without. The skills file's `[skill.look]` names how each skill looks while it is on. `[[effects.upgraded]]` names the glows of worn items by level: an item shows the glow of the highest level it reached that names one for it.
- **Shine:** `[shine]` names the pictures worn armours and weapons reflect, by their item table's `shine`, where their textures' alpha says ([`render.md`](render.md), "Shine"); none shine without. `glow` picks an item's glow of `[[effects.upgraded]]`.
- **The volumes** are `play`'s `--music-volume` and `--sound-volume`, 0.5 and 1 by default: an asset folder holds no player's settings, and the client's `metin2.cfg` isn't read.

### `play --assets`
`openm2 play --assets DIR` (or `assets` in `openm2.toml`, [`client.md`](client.md)) plays with an asset folder, the only thing it plays with:
- **the world** from `assets.toml`'s maps that have a place, each found by its own areas' size; its walls from the areas' `attributes.png` (`render.md`, "Walking around walls"); the minimap from their `minimap.png`, the atlas from `map.toml`'s picture and marks;
- **characters and monsters** from their actors, with their motions' sounds; items on the ground and in hand from the item table's models;
- **the interface** with the folder's skin; items, skills and monsters' names from its tables;
- **effects** from its effect files: the maps', the actors' and their motions'; a hit's and the twinkle over items on the ground are the ones `[effects]` names, none without. `play --no-effects` leaves them all out.

## `openm2 assets import`
`openm2 assets import --client DIR --out DIR` converts the player's own client into an asset folder, on the player's machine:
- **Which actors and maps:** every player race's model script, every model script of the monster list's monsters, and every map folder (`--actor PATH`, `--map FOLDER` and `--effect PATH`, each repeatable, for only some, with what they use; given any, only what they name).
- **What `play` needs,** with none of them: the built-in skin as `skin.toml`, its art converted (a sub-image as its picture's region); the locale's item table and item list as `items.toml`, the icons and models converted, the client's bag as the bag; its skill descriptions and skill table as `skills.toml`, the icons by grade as regions, and its game texts' names of the jobs' skill groups (`locale_game.txt`'s `SKILL_GROUP_WARRIOR_1` to `SKILL_GROUP_SHAMAN_2`) as `[[group]]`; its monster table's names and levels in `assets.toml`'s `[[monster]]` entries; the screens' music, copied into `bgm/`; the client's three hit sounds and its potion sound (`sound/ui/eat_potion.wav`, as `potion_sound`); its hit, critical hit and dropped item effects, the loops its scripts register for poison, burning, slowness and stuns (`effect/hit/blow_poison/poison_loop.mse`, `effect/hit/blow_flame/flame_loop.mse`, `effect/affect/slow.mse`, `effect/etc/stun/stun_loop.mse`) and its rings for what the pointer is over and for the target (`effect/etc/click/click_select.mse`, `click_glow_select.mse`), what a potion's health and mana and a level gained show (`effect/etc/recuperation/drugup_red.mse`, `drugup_blue.mse`, `effect/etc/levelup_1/level_up.mse` with its sound), the fish its program shows over a fisher a fish bit (`effect/etc/emoticon/fish.mse`), and the glows of upgraded items (`pc/common/effect/armor/armor_7.mse` to `armor_9.mse`, `sword/sword_7.mse` to `sword_9.mse` and their `_s`, `_b` and `_f` for the weapon kinds), as `[effects]`; its sphere maps (`special/spheremap.jpg` for armours, `spheremap01.jpg` for weapons) as `[shine]`; its golden pieces of experience (`effect/etc/gathering/ga_piece_yellow_small2.msf`, as its scripts register them, with the effects beside it) as `[flies]` `experience`; its damage numbers (`effect/affect/damagevalue/`: `damage.mse`, `damage_1.mse`, `target.mse`, `miss.mse`, `target_miss.mse`, and `damage_0.dds` to `_9`, `target_0.dds` to `_9`) as `[numbers]`; for each skill its skill table names a state of (the sura's flame spirit, whose state is the original server's, too), the look its scripts register for that state, as `[skill.look]`; each item's `specular` as `shine`, and its glow level by the client's rule (`client_glow`: the last digit of its number plus its socket value, less 3) as `glow`; a fishing pole's (type 13) value 2 as `points`. Names are decoded from the locale's codepage into UTF-8; flags and weapons that aren't plain words are left out.
- **Their motions:** a player's, every motion script under the folder of its base model (`set` its subfolder, `name` the file's); a monster's, the lines of its motion list. Each motion's sound script gives its sounds.
- **Their maps:** each map folder's settings, texture set, environment script and atlas points (from the client's locale) as its `map.toml`, its atlas picture as `atlas.png`; each area's grids as its rasters, its objects (through their properties: buildings and dungeon blocks as models, trees by the recipe and season their file's name picks, effects) and sound spots as its `area.toml` (an area with none of them and no water gets none); the maps where the client's atlas puts them, in `assets.toml`.
- **Their effects:** every effect script the actors, motions, maps and `play` name, with the sounds of its sound script; its textures and mesh files where the client finds them (as named, else beside the script; a texture's `.dds` of the same name when it is named otherwise). **Their flying effects:** every flying effect script the motions' fly events name (and `--effect` a `.msf`), as flying effect files, and the effects they name: as named, else beside the script, where most of them are; one the client lacks is left out (an attached effect with it, the bomb effect).
- **What is written,** under the client's paths as safe paths (`m2_assets::safe_relative_path`: lowercase, `%XX` for odd bytes), with new extensions: model scripts as `.actor.toml`, motion scripts as `.motion.toml`, effect scripts as `.effect.toml`, mesh effects (`.mde`) as `.glb` (a scene a frame), GR2 files as `.glb` (every model of the file, its animations, and the floors of the model attributes beside it), DDS, TGA and JPEG textures as `.png` (their first level, RGBA; a second texture that would take the same name keeps its whole name: `x.tga.png`), sounds as they are; and `assets.toml`. Names that aren't UTF-8 are written with `%XX` ([`formats/gltf.md`](formats/gltf.md), "Writing").
- **Alpha:** a texture's alpha mode comes from its format, as the renderer reads the client's: DXT1 and 1-bit alpha, 32-bit TGA and 16-bit with an alpha bit are `MASK`; DXT3, DXT5 and other masks are opaque.
- **What didn't convert is left out,** and reported: a motion whose animation didn't, sounds, shapes, hair styles and motions whose files didn't, and skins whose textures didn't (the model's own shows, as in the client); objects whose model didn't, terrain textures, sky pictures, clouds and sound spots whose files didn't (the client draws nothing there either), and objects naming a property the client lacks or of a kind openm2 doesn't draw; the skin's art, items' icons and models, skills' icons, music and hit sounds the client lacks (their fallbacks show, as in the client); effects whose scripts the client lacks or that didn't convert (left out of the actors, motions, maps and `[effects]` that name them), an effect's meshes whose files the client lacks (nothing is drawn there either), and its textures the client lacks, which draw white as in the client. A texture the client lacks or openm2 can't read (`.psd`) leaves its group untextured, white, as the client draws it. So an import's folder names only files it has (`assets check`). Buildings' collision shapes aren't written: openm2 doesn't use them (the attributes block the buildings' footprints).
- **Checked on the way:** every GLB file is read back; its models, animations and floors must come back with the same tracks and triangles, a mesh effect with the same meshes, frames and corners, and the report gives the largest difference between a track's keys and its GR2 curve, at the knots and between them, relative to the curve's largest control. Every area's attributes are read back from their PNG and must be the same.
- **It writes nowhere else:** never outside `--out` (one folder level at a time, never through a link), and into a folder that isn't empty only with `--force`, which replaces the files an import writes.
- **On every processor:** it reads, converts and writes on one thread for each of the machine's processors (`--threads N` for another number). A kind of file is converted whole before the next, as each names only kinds after it: motions, then effects, models, mesh effects, textures and sounds; a model's and a mesh effect's textures with it. The files of a kind are read on the threads, given their places in the folder in their order, then written on the threads, so the folder and the report are the same, byte for byte, however many threads (the CI test imports on one and on four).
- **Its number** goes into `assets.toml` as `import` (`IMPORT` in `crates/m2-cli/src/assets.rs`, 10): raise it with any change to what the import writes from the same client.

**In a TMP4 client:** 496 actors (the 8 player races', and 488 for the monster list's 1,311 monsters) with 5,498 motions; all 86 maps, at 94 places in the world, with 1,192 areas, 51,232 objects (22,074 models, 12,780 trees, 16,378 effects) and 7 sound spots; the skin's 51 looks, 5,743 items, 66 skills, 1,334 monsters named, 3 pieces of music; 510 effects (44 with mesh effects). That is 6,601 GLB files, 12,083 PNG files and 1,387 sounds, 2.2 GB, in 38 seconds on a laptop with 20 threads (3½ minutes on one, release build); `assets check` reads it all back in 5 seconds. Left out, as the client lacks them: models, motions, sounds, textures, effects and objects its files name (`map check` counts the objects); two GR2 files of Granny 2.7, which openm2 doesn't read, and the motions whose animations are among them, don't convert. The tracks' keys are within 2.4 × 10⁻⁵ of their GR2 curves' largest control over 178 million values; the warrior, the shaman, the magic gnoll (whose bones shear) and the wolf skin over their runs within 0.00005 cm of the client's.

**Pictures:** what the import draws matches the client's own files: an actor's vertices to 0.00005 cm over a whole run, the same texture coordinates, normals and texture pixels; the terrain, water, shadows, sky, effects and the interface's art the same to the pixel. What differs is the mip levels: a DDS texture carries its own (DXT-compressed, and often fewer than a full chain), a PNG has one level and the renderer averages the rest (`TextureData::from_png`); drawn with mip levels made from the DDS's first level the same way, the pictures are the same. With solid textures (the CI test) they are the same.

## `openm2 assets current DIR`
Whether the folder holds this openm2's import, as the launcher asks before it plays ([`update.md`](update.md)): exit code 0 and `this openm2's import (1)` when its `assets.toml` has this build's `import`; else exit code 1 and why not, in one line: `no import yet` (no `assets.toml`), `import 0, not this openm2's (1)`, `not an import, or one from before imports had numbers`, or `it doesn't read: …`. It reads nothing but `assets.toml`, so it is quick: a launcher asks at every start. A whole import is the only kind there is: the import writes `assets.toml` last, and the launcher keeps an import only once `assets check` passes.

## `openm2 assets check DIR`
Reads a whole folder back, on the threads, as `vfs check` reads a client: `assets.toml` and the actors, maps, skin, item table, skills, music and hit sounds it names (every image's PNG must be there, and a region inside it); every actor, motion, effect, flying effect, map and area file, and every file they name, an effect's mesh effects as `play` reads them; every map's areas as `view map` reads them, each raster by its kind and sides; every GLB file, each of its models (its images beside it must be there), animations and floors; every PNG image and sound decoded. Paths an asset folder may not have are problems too. It prints what it counted, and every problem; the exit code is 1 when there is one.

## Imported files
Imported files stay on the player's machine (`docs/source-policy.md`). Every file `assets import` writes is marked:
- TOML files start with `# openm2 assets import: from the player's own client; never commit (docs/source-policy.md)`;
- PNG images carry the marker's text as their `Comment` (a `tEXt` chunk);
- GLB files name it as their `asset.generator`.

`just no-assets` refuses `.glb` and `.gltf` files (and PNG, WAV and MP3) and files starting with the TOML marker; `openm2 release manifest` refuses a folder with files either import marked (`m2_assets::folder::import_mark`), unless `--allow-imported` is given, which is for a channel on the player's own machine (`http://127.0.0.1`) only ([`update.md`](update.md)). Sounds carry no marker; the actor files that name them do.
