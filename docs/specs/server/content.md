# Server: the content folder

What openm2's own server knows about its world, as a folder of TOML files and walk masks: the maps and where one may walk on them, the monsters and NPCs, the items, the skills, the player races' walking speeds, the experience each level takes, what monsters drop, what new characters start with, what NPCs sell, which NPCs teach skill groups, how skills train past a master's, which monsters live together in packs, and where monsters live. `m2-content` reads, checks and writes it; `openm2 content import` writes most of it from the operator's own client, the spawns its maps carry among them; the operator writes the packs and the rest of the spawns. How the server uses it: [`world.md`](world.md).

**Checked by:** `crates/m2-content/tests/` (a folder read field by field, every rule broken once with its line, written files read back the same, random and damaged folders; walk masks, straight walks and paths on hand-drawn grids, PNG files built byte by byte), `crates/m2-cli/tests/content.rs` (the import on a synthetic client, its spawn lists and the operator's packs among it), `crates/m2-server-core/tests/world.rs`, `packs.rs`, `combat.rs`, `items.rs`, `shops.rs`, `skills.rs`, `teachers.rs` and `training.rs`.

## The folder
| File | Written by | Holds |
|---|---|---|
| `content.toml` | the import or by hand | `format` and `name`, and from the import `import`, its number (below). Required: it tells a content folder from any other |
| `maps.toml` | the import | `[[map]]` entries |
| `monsters.toml` | the import | `[[monster]]` entries: monsters, NPCs, stones and other things that stand in the world |
| `items.toml` | the import | `[[item]]` entries |
| `races.toml` | the import | `[[race]]` entries: the player races' walking speeds |
| `skills.toml` | the import | `[[skill]]` entries: the skills, from a client with a skill table |
| `levels.toml` | the operator | `[[level]]` entries: the experience each player level takes. Optional: without it the server has a table of its own ([`world.md`](world.md), "Experience and levels") |
| `drops.toml` | the operator | `[[drop]]` and `[[gold]]` entries: what monsters leave on the ground when they die. Optional |
| `start.toml` | the operator | the level, gold and `[[item]]` entries new characters start with. Optional |
| `shops.toml` | the operator | `[[good]]` entries: what NPCs sell. Optional |
| `teachers.toml` | the operator | `[[teacher]]` entries: the NPCs where players choose their skill group. Optional |
| `training.toml` | the operator | `[books]`, `[stones]` and `[[help]]`: how skills train past a master's. Optional: without it books train with openm2's numbers, and there are no soul stones or items that help |
| `mounts.toml` | the operator | `[mount]`, `[[grade]]`, `[[level]]`, `[[feed]]` and `[[stable]]`: players' mounts, how they grow, what they eat and where they are trained. Optional |
| `upgrades.toml` | the operator | `[[smith]]`, `[[scroll]]` and `[[set]]`: the NPCs and scrolls that upgrade items, and what each upgrade set takes. Optional: without it nothing upgrades |
| `packs.toml` | the operator | `[[pack]]` entries: monsters that live together, a leader and the others. Optional |
| `spawns/imported.toml` | the import | `[[group]]` and `[[npc]]` entries: the spawn lists the client's maps carry ("The import") |
| `spawns/<name>.toml` | the operator | `[[group]]` and `[[npc]]` entries |
| `walk/<name>.png` | the import or by hand | a map's walk mask: where one may walk ("Walk masks" below). Optional |

- Every file starts with `format = 1`. Another number is refused as a format this server does not know; a file without the line is refused too.
- Files are optional except `content.toml`; a folder with only `content.toml` is an empty world.
- Spawn files are read in name order; their names are ASCII letters, digits, `_` and `-`, at most 64. Other `*.toml` files at the top are refused; files that are not `.toml` are ignored, so a README may sit beside them.
- Walk masks' names are ASCII letters, digits, `_`, `-` and `.`, at most 64, not starting with `.`, then `.png`; a `.png` file with another name in `walk/` is refused, other files there are ignored.
- Units: metres, seconds, degrees. Positions are **metres from the map's top-left corner** (east, south), as `/where` in the game ([`session.md`](session.md), "Chat"), `openm2 view map --at X,Y` and `openm2 map info` show them; the server adds the map's base. Directions: 0 faces south (+y), 90 east, as on the wire.
- Unknown keys are refused, so a typo never goes unnoticed. Keys left out take the defaults below.

## Entries
### Map
```toml
[[map]]
name = "metin2_map_a1"
base = [4096.0, 8960.0]   # the world position of its top-left corner, metres
size = [1024.0, 1280.0]   # width and height, metres
walk = "walk/metin2_map_a1.png"   # optional: its walk mask
water = "walk/metin2_map_a1.water.png"   # optional: its water mask
```
The name is the client's map folder: ASCII letters, digits, `_ - . /`, at most 64, no `..`. Without `walk`, every square of the map is open; without `water`, none is water.

### Walk masks
Where one may walk on a map, in `m2_content::WalkMap`:
- **The file:** a PNG, a pixel per **square a metre wide**, from the map's top-left corner; as many pixels as the map's size in metres, rounded up (a map of 5 × 3.5 m has a mask of 5 × 4). A pixel whose grey (or the mean of its red, green and blue) is under 128 is **blocked**; lighter ones are open; alpha is not read. Any colour type and bit depth; not interlaced. openm2 writes black and white masks, a bit a pixel, with the import's marker as their text when imported.
- **A straight walk is clear** (`WalkMap::clear`) when every square its line passes through is open, and where it passes exactly through a corner, the two squares beside it are open too (no slipping between two blocked squares that touch at a corner). Squares off the map are blocked. A walker standing in blocked squares may walk out of them, but the square it ends in must be open: the square its end point lies in (the one east and south of a grid line it ends on). Lines are followed in whole centimetres, so every machine agrees.
- **A path** (`WalkMap::find_path`) goes around blocked squares: A* over squares, eight ways, never cutting a blocked square's corner, inside a box of the start and the end and 32 squares around them, at most **512 squares** on a side; its squares are joined into the fewest legs that keep **30 cm** from blocked squares where they can, each leg clear. No path when the end is blocked or off the map, or when none is found in the box (a clear straight line is then the path, however long).
- `openm2 play` walks these paths on a mask made the same way from the client's attributes ([`../render.md`](../render.md), "Walking around walls"). The server checks players' moves and walks its monsters with these rules ([`session.md`](session.md), "Moving"; [`world.md`](world.md), "Walls").
- **Water masks**: the same file in the same folder, `walk/<name>.water.png`, its dark squares water (the client's attribute flag `0x02` under the square's centre). The import writes one for each map with water; the server checks a cast's float and a fire against it (`Content::is_water`), and `play` makes the same mask from its asset folder's attributes to find where to cast.

### Monster
```toml
[[monster]]
vnum = 101                # the client draws it by this number (EntitySpawn's race)
name = "Wild Dog"
kind = "monster"          # monster, npc, stone, warp, door, building, horse, goto
to = [4503, 9050]         # optional, warps and gotos only: where they take players, metres east and south
rank = 0
battle = 0
level = 1
hp = 87
hp_regen = { every = 10.0, percent = 1 }   # optional
defence = 2
experience = 15
gold = [5, 10]            # least, most
damage = [10, 14]         # least, most
damage_multiplier = 1.0
stats = { st = 1, dx = 1, ht = 1, iq = 1 }
attack_speed = 100
move_speed = 100          # 100 is its run motion's own speed
attack_range = 1.5        # metres
aggressive = false
aggressive_sight = 10.0   # metres
moves = true              # false: it never moves
run_speed = 3.0           # optional: metres a second its run motion covers at movement speed 100
calls = [104, 105, 106]   # optional, stones only: the packs it calls as it loses health
```
- Required: `vnum`, `name`, `kind`. Defaults: `level` 1, `hp` 1, `damage_multiplier` 1, `attack_speed` and `move_speed` 100, `moves` true, the rest 0, false or absent.
- `kind` decides EntitySpawn's kind ([`entities.md`](../protocol/openm2/entities.md), "Kinds and races"): monster 0, npc 1, stone 2, warp 3, door 4, building 5, horse 8, goto 9.
- Fights use `level`, `hp`, `hp_regen`, `defence`, `experience`, `damage`, `damage_multiplier`, the stats' `st`, `dx` and `ht`, `attack_speed`, `attack_range`, `aggressive` and `aggressive_sight` ([`world.md`](world.md), "Fighting"). `gold` is the gold it drops when its most is above 0 ([`world.md`](world.md), "A monster's drops"); `rank`, `battle` and `iq` keep the client's numbering for later.
- Only `monster` and `stone` entries of groups can be fought; only monsters fight back.
- Without `run_speed` the server times its walks with its own default (4.5 m/s).
- `calls`: a stone's packs (`packs.toml`), one of which comes each time its health falls into a lower tenth ([`world.md`](world.md), "Stones"); a number may be there more than once, to come more often.
- `to`: a warp takes the players who come within 3 m of it to this point of the world; a goto to this point of the map it stands on, from the map's corner ([`world.md`](world.md), "Warps"). Without it, they take nobody. `content check` counts those that take players somewhere and the warps among them that lead to no map.

### Item
```toml
[[item]]
vnum = 11299
name = "Black Steel Armour+9"
kind = 2
subtype = 0
size = 2                  # height in the bag, slots
anti_flags = 0
flags = 0
wear_flags = 1
immune_flags = 0
buy_price = 4000
sell_price = 400
limits = [{ kind = 1, value = 54 }]        # only those set
applies = [{ kind = 7, value = 15 }]
values = [0, 0, 0, 0, 0, 0]
sockets = [0, 0, 0]
upgrade = 0
upgrade_set = 0
magic_percent = 0
socket_percent = 0
specular = 0
```
- Required: `vnum`, `name`. Defaults: `size` 1, the rest 0 or empty.
- Kinds, flags, limits, applies and values keep the client's numbering (`docs/specs/formats/proto.md`): the client describes items from its own table, so the server must mean the same thing.
- What the server reads of them ([`world.md`](world.md), "Items"; `m2_content::Item` has a method for each):

| Field | What the server reads |
|---|---|
| `size` | the places it covers in the bag, 1 to 3 (the client draws none taller) |
| `flags` | 4: it stacks, up to 200 in one place |
| `wear_flags` | where it is worn: head 2, shoes 4, bracelet 8, weapon 16, necklace 32, earrings 64, body 1, shield 256, unique 128, arrows 512, tried in that order |
| `anti_flags` | who may not use it: female 1, male 2, warrior 4, ninja 8, sura 16, shaman 32 |
| `limits` | kind 1: the lowest level that may use it |
| `applies` | kinds 7 and 8: attack and movement speed in percent, while worn |
| `kind` 1 (weapon) | `values` 3 and 4: its damage roll; value 5: its bonus |
| `kind` 2 (armour) | `values` 1 + 2 × value 5: its defence |
| `kind` 3, `subtype` 0 (potion) | `values` 0 and 1: the health and mana it gives back |
| `buy_price`, `sell_price` | a good's price without one of its own (buy price × count); what one sells for, at most the buy price when there is one |
| `upgrade`, `upgrade_set` | the item it upgrades to, and the set of `upgrades.toml` that says what that takes; 0 for none. An item that names itself or an item that isn't there does not upgrade. Read backwards, `upgrade` also gives the item one goes down to (the lowest number when several name it) and an item's level: how many upgrades lead to it |

- Item 1 is gold on the ground (GroundItemAdd's number for it): drops and starting items give gold with their own fields.

### Race
```toml
[[race]]
race = 0                  # class and gender, 0 to 7, as on the wire
run_speed = 4.5           # metres a second at movement speed 100
```
A race without an entry walks at the server's default (4.5 m/s).

### Level
```toml
[[level]]
level = 1                 # from 1, in order, none missing
experience = 300          # what it takes to go from this level to the next
```
Players reach one level past the last entry, where they gather no more experience.

### Drop
```toml
[[drop]]
monster = 101             # who drops it; not an npc
item = 27001
count = 2                 # 1 for an item that does not stack; at most 200
chance = 12.5             # percent, above 0 and at most 100
```
Default: `count` 1. Each of a monster's drops is rolled on its own, in parts per million, every time it dies; its gold comes from its `gold` range ([`world.md`](world.md), "A monster's drops").

### Gold (`drops.toml`)
```toml
[[gold]]
monster = 101             # who drops it; not an npc
amount = [4, 9]           # least, most
chance = 60               # percent, above 0 and at most 100; 100 without it
```
The operator's gold for a monster: it takes the place of the monster's own `gold`, which the import writes from the client's table, and stays when the import runs again. Each time the monster dies its chance is rolled, in parts per million as a drop's, and the gold is a random amount from least to most. One `[[gold]]` a monster.

### Starting items (`start.toml`)
```toml
format = 1
level = 5                 # with the status and skill points of the level-ups skipped
gold = 500                # at most 2,000,000,000

[[item]]
item = 19
races = [0, 4]            # optional: the races (0 to 7) that get it; all without it
wear = true               # optional: worn from the start, if the race may wear it at level 1

[[item]]
item = 27001
count = 10
```
Defaults: `level` 1, `gold` 0, `count` 1, every race, not worn. A character starting past level 1 has 3 status points and 1 skill point for each level skipped, as if it had levelled up ([`world.md`](world.md), "Experience and levels"). Items go in order: worn ones at their position when it is free, the rest at the first free place in the bag (onto a stack of the same item first); what does not fit is left out.

### Shops (`shops.toml`)
```toml
[[good]]
npc = 9001                # the NPC whose shop sells it: an npc
item = 11200
count = 1                 # how many one purchase gives; at most the item's stack
price = 3000              # optional: the whole stack's price; the item's buy price × count without it
```
Defaults: `count` 1; `price` the item's `buy_price` × `count`. An NPC's goods go into its shop in the order written: each at the first free place, row by row, of a shop 5 places wide and 8 high, with room for its height (the item's `size`), covering the places under it ([`world.md`](world.md), "Shops"). Every NPC of that number has the shop.

### Teachers (`teachers.toml`)
```toml
[[teacher]]
npc = 9101                # the NPC that teaches: an npc without a shop
job = "warrior"           # whose skills: warrior, ninja, sura or shaman
group = 1                 # the skill group it teaches: 1 or 2
```
No defaults: every key is needed. Every NPC of that number teaches. Players of that job choose the group there ([`world.md`](world.md), "Skills"); with teachers in the folder, `/skillgroup` is a game master's command.

### Training (`training.toml`)
```toml
[books]                   # skill books: items of kind 17, the skill in values[0]
chance = 50               # percent a read teaches: 1 to 100
wait = 3600               # seconds that skill waits after a read: at most 30 days

[stones]                  # soul stones: they train a grand master's skill
items = [9513]            # the items that are soul stones
chance = 30               # percent a stone trains: 1 to 100
wait = 3600

[[help]]                  # an item that helps the next training, used from the bag
item = 9601
skip_wait = true          # every skill's wait ends
chance = 25               # percent added to the next read or stone, once: 0 to 100
```
Defaults (openm2's own): books `chance` 50 and `wait` 3,600; stones none, `chance` 30 and `wait` 3,600; a help item's `skip_wait` false and `chance` 0. Books that make a character forget its skills are items of kind 22, and need nothing here ([`world.md`](world.md), "Training").

### Mounts (`mounts.toml`)
```toml
[mount]
summon = 1101             # the item that calls the mount and sends it away; never used up
drain = 1                 # stamina a ridden mount loses each second it walks
rest = 2                  # stamina it gets back each second otherwise

[[grade]]
grade = 1                 # 1, 2, … in order
look = 1201               # the monster it looks like: a monster, an npc or a horse
fights = false            # whether its rider may fight from it
skills = false            # whether its rider may use horseback skills (job = "horse")

[[level]]
level = 1                 # 1, 2, … in order; level 1 is the first mount
grade = 1                 # its grade: never below the level before's
move_speed = 200          # the rider's movement speed (100 is the look's run motion's own speed)
stamina = 120             # the most stamina it has
rider_level = 10          # the rider's level training to it needs
gold = 5000               # what training to it costs
items = [{ item = 1102, count = 2 }]   # and the items it uses up
skill_points = 1          # horseback skill points reaching it gives

[[feed]]
item = 1102               # food: one is used up
stamina = 30              # the stamina it gives back, up to the most

[[stable]]
npc = 1301                # an NPC that trains mounts: an npc without a shop that does not teach
```
Defaults: `drain` and `rest` 0, `fights` and `skills` false, `rider_level`, `gold` and `skill_points` 0, `items` none, an item's `count` 1. A character has one mount, at a level from 0 (none) to the last; training at a stable raises it by one, at the costs of the level it reaches, and the first training gives the `summon` item ([`world.md`](world.md), "Mounts"). The look's run motion and the level's `move_speed` time a rider's walks.

### Upgrades (`upgrades.toml`)
```toml
[[smith]]
npc = 1401                # an NPC that upgrades: an npc without a shop that does not teach or keep a stable
on_fail = "destroyed"     # what a failure does: "destroyed", "down" or "kept"

[[scroll]]                # an item put onto another to upgrade it, in place of a smith; one is used up
item = 1501
on_fail = "down"
chance = 10               # percent added to the set's chance, up to 100
sure = false              # always works
up_to = 4                 # the highest level it upgrades an item to (+4); 0 for any

[[set]]
set = 1                   # the number items.toml's upgrade_set names, from 1
chance = 90               # percent an upgrade works: 1 to 100
gold = 500                # what it costs, whether it works or not
items = [{ item = 1601, count = 2 }]   # and the items it uses up, whether it works or not
```
Defaults: `on_fail` `"destroyed"`, a scroll's `chance` 0, `sure` false and `up_to` 0, a set's `gold` 0 and `items` none, an item's `count` 1. An item of the bag upgrades to its `upgrade` by its `upgrade_set` when this file has that set; `"down"` makes a failed item the one that upgrades to it, and keeps an item nothing upgrades to as it is ([`world.md`](world.md), "Upgrades"). The scrolls' kind and values in the client aren't read: the file names each one.

### Fishing (`fishing.toml`)
The operator's; the import never writes it. Without it no map has fish.
```toml
[rules]                   # optional; these are the defaults
bite = [10, 40]           # the shortest and longest wait for a bite, whole seconds (1 to 3600)
react = 6.0               # seconds a bite lasts (above 0, at most 60)
reach = 6.0               # metres from the fisher to the float (1 to 30)

[[fisher]]
npc = 9009                # an NPC that betters full poles: an npc that is no smith, teacher or stable; it may have a shop

[[timing]]
name = "middle"           # a word: lower-case letters, digits and _
points = [[0.0, 0.0], [1.0, 0.0], [3.0, 80.0], [5.0, 0.0]]   # [seconds after the bite, percent], times never going back

[[fish]]
item = 27803              # what may be caught: any item
difficulty = 30           # from 1: the pole's level and the bait's strength over it is the chance it holds, up to sure
timing = "middle"

[[water]]
maps = ["metin2_map_a1"]  # each map once among the waters
nothing = 3000            # the empty hook's weight
catch = [{ item = 27803, weight = 900 }]   # items of [[fish]], each once

[[open]]
item = 27987              # an item that, used, gives one of these (a live fish, a clam); no other use
gives = [{ item = 27990, count = 1, chance = 50.0 }]   # percent each, at most 100 in all; the rest is nothing

[fire]
item = 27600              # the item that lights it; no other use
npc = 12000               # the fire, an npc
seconds = 40              # how long it burns (1 to 3600)

[[grill]]
item = 27803              # an item a fire turns, the whole stack,
into = 27863              # into this
```
Each fish must be caught on some water, and each item that opens or lights the fire is named once. `content check` counts the poles and the bait of the item table. The item table says the rest ([`world.md`](world.md), "Fishing"): a **pole** is type 13 (values: level, 1 in N chance of a point, the most points, the fisher's percent, the pole a failure gives; `upgrade` the next pole); **bait** type 3 subtype 6 (value 0 its strength); a **potion** subtype 0 or 11 (11 also values 3 and 4 in percent); an item that **raises** a point for a while subtype 7 (value 0 the apply: 5 strength, 6 dexterity, 7 attack speed, 8 movement speed; value 1 seconds; value 2 how much), one that **hides** subtype 13, a **cure** subtype 12.

### Skill
```toml
[[skill]]
vnum = 1                  # 1 to 254
name = "Wide Swing"
job = "warrior"           # warrior, ninja, sura, shaman; passive, horse
level = 0                 # the lowest character level that may learn it
kind = "melee"            # normal, melee, range, magic
flags = ["attack", "use_melee_damage"]
point = "hp"              # what it changes; negative health is damage
amount = "-(2*atk+(0.5*atk+3*str)*k)"
duration = "60"           # seconds its change lasts (raised points)
point2 = "mov_speed"      # a second change, with amount2 and duration2
amount2 = "15*k"
duration2 = "60"
sp_cost = "10+20*k"      # its mana cost
duration_sp_cost = "5+20*k"  # what it costs every 5 s while on (toggle)
cooldown = "10"           # seconds before it can be used again
max_hit = 2               # the most monsters one use hits
target_range = 25         # metres; 0 for a hit's reach
splash_range = 1          # metres around that it hits
```
Defaults: `level` 0, `kind` normal, no flags, points or formulas, `max_hit` 0 (one monster), ranges 0. Flags and points keep the client's skill table's words in lower case; the server reads the flags `attack`, `selfonly`, `toggle`, `compute_magic_damage`, `attack_poison`, `attack_fire_cont`, `attack_slow`, `attack_stun`, `crush`, `penetrate` and `ignore_target_rating` and the points `hp` (damage, or a heal for a skill that doesn't attack), `att_speed`, `mov_speed`, `att_grade`, `def_grade`, `resist_normal`, `reflect_melee`, `critical`, `dodge`, `hit_hp_recover`, `manashield` and `casting_speed`, and keeps the rest for later ([`world.md`](world.md), "Skills"). `duration_sp_cost` is the import's from the skill table's eleventh value.

### Formulas
A skill's amounts, durations, mana cost and cooldown are formulas in a string: numbers (at most 3 decimals), `+ - * /`, brackets, and
- the names `k` (the skill's power at its level, 0 to 1), `lv`, `str`, `dex`, `con`, `iq`, `atk`, `mwep`, `ar`, `odef`, `maxhp`, `maxsp` and `chain` (what each is for a player: [`world.md`](world.md), "Skills");
- the functions `number(a, b)` (a whole number from a to b at random), `floor(x)`, `min(a, b)` and `max(a, b)`.

They are worked out in whole thousandths: every step is rounded towards zero, kept within ±10¹⁵, and a division by 0 gives 0, so every machine gets the same result (openm2's own reader, `m2_content::Formula`). A formula that does not read is a problem with its place, such as `skills.toml:7:11: "wisdom" at 4 is not a known name`. At most 256 bytes, nested at most 32 deep.

### Pack
```toml
[[pack]]
number = 7                # spawn files' groups name it
leader = 102              # a monster
members = [{ monster = 101, count = 3 }]   # optional: the others
```
- Monsters that live together: the server puts the leader at a point and the others around it, and they come back together ([`world.md`](world.md), "Entities and vids"). The leader only stands at the pack's point for now.
- Defaults: no `members`; a member's `count` 1.

### Group (spawn files)
```toml
[[group]]
map = "metin2_map_a1"
at = [250.0, 80.0]        # the area's centre, metres from the map's top-left
radius = 12.0             # metres: a circle
monster = 101
count = 5
respawn = 60.0            # seconds from the removal of its last monster to its return
rotation = 90.0           # optional: the way they face; random without it

[[group]]
map = "metin2_map_a1"
at = [400.0, 300.0]
size = [200.0, 40.0]      # a rectangle's width and height in metres, about `at`, instead of a circle
pack = 7                  # instead of a monster: packs.toml's pack
count = 12                # how many packs
```
- **Of one monster** (`monster`): `count` monsters at random points of the area; the whole group comes back once the last is removed.
- **Of packs** (`pack`): `count` packs, each at its own random point of the area with its monsters around it, each coming back on its own at a new point ([`world.md`](world.md), "A monster's death").
- **The area:** a circle of `radius` about `at`, or a rectangle of `size` about it.
- Defaults: `radius` 0 (they stand on the point and never stroll), no `size`, `count` 1, `respawn` 60.

### NPC (spawn files)
```toml
[[npc]]
map = "metin2_map_a1"
at = [244.5, 61.0]
monster = 20016           # must be an npc
rotation = 180.0
```

## Rules
Everything is checked before the server starts, and every problem is listed as `file:line:column: message` (up to 1,000), the line being the entry's (for a syntax error, the place of the error):
- **Every file:** `format = 1` first; no unknown keys; numbers of the right type and size (a speed past 255 does not fit).
- **Names** (content, monsters, items): not empty, at most 64 bytes, no control characters.
- **Maps:** names unique and as above; base from 0, size above 0, the far corner inside the world (21,474,836.47 m); `walk` a walk mask's path (`walk/<name>.png`) that is in the folder, as large as the map (above), and at most 8,192 m on a side.
- **Walk masks:** a PNG as above, at most 8,192 pixels on a side; every mask in `walk/` named by a map.
- **Monsters:** numbers unique, from 1 to 65,535 (EntitySpawn's race is 16 bits); `to` only on warps and gotos, each number from 0 to 21,474,836.47 m; `calls` only on stones, at most 64, each a pack of `packs.toml`; gold and damage least ≤ most; `damage_multiplier` 0 to 1,000; `attack_range` and `aggressive_sight` 0 to 1,000 m; `run_speed` above 0, at most 100 m/s; `hp_regen` every above 0 and at most an hour, percent at most 100.
- **Items:** numbers unique, not 0; `size` 1 to 9; at most 8 limits and 8 applies.
- **Races:** 0 to 7, unique; `run_speed` above 0, at most 100 m/s.
- **Levels:** 1, 2, 3, … in order with none missing; `experience` above 0; at most 254 (a level is a byte on the wire).
- **Drops:** the monster exists and is not an npc; the item exists and is not item 1; `count` from 1 to its stack (200 if it stacks, else 1); `chance` above 0 and at most 100.
- **Gold:** the monster exists and is not an npc, with one `[[gold]]` at most; `amount` least at most most, most from 1 to 2,000,000,000; `chance` above 0 and at most 100.
- **Starting items:** `level` from 1 to one past the last of `levels.toml` (254 without it); `gold` at most 2,000,000,000; each item exists, is not item 1, and its count fits its stack; races 0 to 7; `wear` only for an item with wear flags.
- **Shops:** the NPC exists and is an npc; the item exists, is not item 1, and its count fits its stack; the price (written or worked out) at most 2,000,000,000 and at least what the stack sells back for (its sell value × count: the sell price, at most the buy price when there is one), so nobody makes gold buying and selling it; the item at most 8 places high, and room left in its shop.
- **Teachers:** the NPC exists, is an npc and has no shop in `shops.toml`, and teaches once; `job` one of the four jobs (not `passive` or `horse`); `group` 1 or 2.
- **Training:** `chance` 1 to 100 and `wait` at most 2,592,000 s (30 days) for books and stones; a help item's `chance` at most 100, and `skip_wait`, a chance or both; each soul stone and help item an item of `items.toml` with no other use (not worn, a potion, a skill book or a book that forgets), once over the stones and the help.
- **Mounts:** with any grade, level, food or stable there is a `[mount]`, and with a `[mount]` level 1 at least. The `summon` item exists, is not item 1 and has no other use; grades and levels 1, 2, … in order with none missing (at most 16 grades and 100 levels); a grade's `look` exists and is a monster, an npc or a horse; a level's `grade` exists and is no lower than the level before's; `move_speed` 1 to 255; `stamina` 1 to 65,535; `gold` at most 2,000,000,000; at most 8 kinds of items, each once, existing, not item 1, its count fitting its stack; the levels' `skill_points` at most 255 in all. Food: at most 64 kinds, each item once, existing, not item 1, not the `summon` item and with no other use; `stamina` 1 to 65,535. Stables: the NPC exists, is an npc, has no shop and does not teach, once.
- **Upgrades:** each smith once: the NPC exists, is an npc, has no shop, does not teach and keeps no stable. Each scroll once: the item exists, is not item 1 and has no other use; `chance` at most 100, and none on a `sure` scroll. Each set once: from 1; `chance` 1 to 100; `gold` at most 2,000,000,000; at most 4 kinds of items, each once, existing, not item 1, its count fitting its stack. Items whose set is not in the file are not refused (they come from the import): they do not upgrade, and `content check` counts them.
- **Fishing**: `[rules]`' ranges; each fisher once, an npc that is no smith, teacher or stable; each timing once, a word, 1 to 64 points, times from 0 never going back, percents 0 to 100; each fish once, an item that exists, a difficulty from 1, a timing the file has, caught on some water; each water with maps (each existing, once among the waters), at most 256 catches, each a fish once, the weights not all 0; each item that opens or lights the fire once, existing, with no other use; an opening's gifts at most 32, each existing with a count fitting its stack and a chance above 0, at most 100 % in all; the fire's NPC an npc and its seconds 1 to 3600; each grilled item once, it and what it becomes existing. `content check` counts the maps with water and with fish, the poles, the bait, the fish, the fishers, the items that open and those grilled.
- **One use an item:** an item is worn (it has wear flags), a potion, a skill book, a book that forgets skills, a soul stone, an item that helps training, the mount's call, mount food or an upgrade scroll, never two of these (`Content::item_use`) ([`world.md`](world.md), "Items").
- **Skills:** numbers unique, from 1 to 254 (the messages carry a skill in a byte); a name as above; at most 32 flags; flags and points lower-case ASCII letters, digits and `_`, at most 32 bytes; formulas that read; ranges 0 to 1,000 m.
- **Packs:** numbers unique, from 1; the leader and every member a monster of kind `monster`; a member's `count` at least 1; at most 64 monsters in a pack, its leader included (and so at most 64 members).
- **Groups:** a `monster` or a `pack`, not both; the map exists; the monster exists and is not an npc (NPCs go under `[[npc]]`), or the pack exists; `count` 1 to 1,000; a `radius` or a `size`, not both: `radius` 0 to 500 m, `size` 0 to 8,192 m each way, and for a group of one monster half the rectangle's diagonal at most 500 m (packs may live in larger areas); the whole circle or rectangle inside the map; `respawn` 0 to a day; `rotation` from 0 to below 360.
- **NPCs:** the map exists; the monster exists and is an npc; the point inside the map; `rotation` as above.
- **Caps:** 32 MiB per file, 256 MiB, 1,024 spawn files and 1,024 walk masks per folder; 1,024 maps, 65,536 monsters, 131,072 items, 255 skills, 262,144 drops, 65,536 gold drops, 90 starting items, 65,536 goods, 65,536 teachers, 256 soul stones, 256 help items, 65,536 stables, 65,536 smiths, 256 upgrade scrolls, 4,096 upgrade sets, 65,536 packs, 65,536 groups, 65,536 NPCs; at most 200,000 monsters in the world from all groups (a group of packs counts each pack's monsters).
- A file that cannot be read gives one problem; references into it are not reported again.

## The import
`openm2 content import --client <dir> [--keys <file>] --out <dir> [--name <name>] [--force]` writes `content.toml`, `maps.toml`, `monsters.toml`, `items.toml`, `races.toml`, `skills.toml`, the walk masks in `walk/` and `spawns/imported.toml` from the operator's own client (`--keys` or `OPENM2_KEYS` for the proto keys):
- **Maps:** each entry of `atlasinfo.txt`; the size is its areas × the map's area size from its `setting.txt` (cell scale × 128), or 25,600 cm without one. A map the atlas lists again at another place (copies of a dungeon) keeps its first place.
- **Walk masks:** for each map with a `setting.txt` and attribute files, `walk/<last part of its name>.png` (numbered from `-2` when two maps would share a name, in any case). A square is blocked where the attributes (`attr.atr`) under its centre have flag **`0x01`** ([`map.md`](../formats/map.md), "Attributes"), and where no area or attribute file covers it. Maps without attribute files get no mask. In a TMP4 client: 67 masks, about 700 KB.
- **Monsters:** each `mob_proto` record. The name is the locale name, decoded with the locale's codepage (`locale.cfg`); characters the decoding cannot show become `?`, an empty name becomes `monster <number>`. The type becomes `kind`; numbers 6 and 7 (players) and unknown types are left out, as are numbers 0 and above 65,535. AI flag bit 0 becomes `aggressive`, bit 1 `moves = false` (QCX's meaning, `proto.md`). Gold and damage are put in order; speeds are clamped to 0–255; centimetres become metres; a damage multiplier that is not a number becomes 1.
- **Warps' places:** a warp's or goto's name is a word and two whole numbers, metres east and south (`Yongan_Area 4503 9050`): they become its `to`, as the original's server reads them, a point of the world for a warp and of its own map for a goto. A name without them gives none, and the summary counts those. In a TMP4 client: 213 of 217 warps and gotos (four names glue the word to the first number, as `.233 780`).
- **A stone's calls:** a stone whose attack speed is above 0 and below its move speed holds there the numbers of the groups it calls, from the first up to, not with, the second (QCX), at most 64: they become `calls`, kept where the folder's `packs.toml` has the pack (the summary lists the others, stone by stone), and its speeds are 100. In a TMP4 client: 40 stones, calling 4 to 13 groups each, 215 different numbers (the 72 speeds the import clamped before were theirs).
- **Run speeds:** for each monster, the motion `play` moves it with (`RUN` in its motion list, else `WALK`), and for each player race its `general/run.msa`: the motion's `Accumulation` over its `MotionDuration` (or its animation's length), to the centimetre. Monsters without a model or motion get none.
- **Items:** each `item_proto` record except number 0; the name as for monsters (`item <number>` when empty); size clamped to 1–9; limits and applies only where set.
- **Skills:** each line of the locale's `skilltable.txt` ([`script.md`](../formats/script.md), "Skill table"), numbered 1 to 254 (the client's monster skills, 256 and up, are left out) and of a known category (0 passive, 1 warrior, 2 ninja, 3 sura, 4 shaman, 5 horse). The name is the first name of the skill descriptions (`skilldesc.txt`), else the table's own, decoded as above (`skill <number>` when empty). Flags and points in lower case (`NONE` is none), the kind as is (`NORMAL` for anything else), formulas as they are; a formula openm2 cannot read, or a word that is not one, is left out and counted. Centimetres become metres. Without a skill table there is no `skills.toml`. In a TMP4 client: 70 skills, every formula read.
- **Spawns**: each map's spawn list in its folder ([`script.md`](../formats/script.md), "Spawn lists"), `regen.txt`, else `monsterareainfo.txt`, into `spawns/imported.toml`, maps by name and then each list's lines in order:
  - **A monster line** (`m`) of an NPC becomes an `[[npc]]` at its point; of any other kind, a group of `count` of that monster.
  - **A group line** (`g`) becomes a group of `count` packs of that number, if the folder's `packs.toml` (the operator's, read and never written) has it; else it is left out, and the summary lists the pack numbers it waits for, map by map. Write them, then import again with `--force`.
  - **The area:** a rectangle the line's range either way about its point, cut to the map, or a point (radius 0) when both ranges are 0; a group of one monster's is cut down to fit a circle of 500 m, keeping its shape.
  - **Its way:** direction 0 none (random); 1 to 8 south and then anticlockwise in steps of 45 degrees (QCX), so `rotation` 0, 45, … 315 (an NPC's 0 without one).
  - **Warps and gotos at a point**: each goes onto the centre of the portal its map draws nearest it within 10 m, to the centimetre, so that it takes the players who stand in the portal they see (a warp takes players within 3 m). The portals are the map's effect objects ([`map.md`](../formats/map.md), "Objects") whose effect file's name starts with `warpgate` (the client's swirls, `ymir work/effect/background/warpgate*.mse`); one outside the map does not count, and an area whose objects cannot be read has none. The summary counts the warps and gotos moved. In a TMP4 client the lists' points stand 1 to 8.6 m from the swirls' centres; all 26 of its warps move (its lists have no gotos), every one onto open ground, and none within 14 m of where a warp lands players.
  - **Its time:** the line's, cut to a day; 0 (`monsterareainfo.txt`'s) takes the default, 60 s (openm2's choice).
  - **Left out and counted:** collections of groups (`r`: openm2 has none yet), lines of kind `e` and `s`, monsters the table lacks, points outside the map. Counts are put between 1 and 1,000. An `a` after the kind (aggressive) and chances below 100 are counted and not followed: the monster's own `aggressive` holds, and they always come.
  - In a TMP4 client: 13 maps' lists; 27 groups (warps, and five monsters on `metin2_map_a1`) and 25 NPCs from monster lines; 443 group lines naming packs 1 to 68 (Chunjo's and Jinno's first maps 1 to 19, the three empires' third maps 20 to 44, the snow maps 45 to 55, the desert 62 to 65, the fire land 66 to 68); 668 collection lines of one dungeon.
- A number seen twice keeps its first record.
- Every file starts with `# openm2 content import: from the operator's own client; never commit (docs/source-policy.md)`. `just no-assets` refuses any file in the tree that starts so.
- It refuses to replace files (walk masks and `spawns/imported.toml` too) without `--force`, and never touches `levels.toml`, `drops.toml`, `start.toml`, `shops.toml`, `teachers.toml`, `training.toml`, `mounts.toml`, `upgrades.toml`, `packs.toml` or the other spawn files. With `--force`, an earlier import's `spawns/imported.toml` is removed when the client now carries no spawns.
- It prints what it wrote and what it left out.
- **The import's number:** `content.toml` gets `import = N`, a number openm2 raises whenever its import writes something else from the same client. The server reads nothing from it. `openm2 content current DIR` says whether a folder holds this openm2's import: exit code 0 when it does; 1, with a line saying why not (no import yet, an import of another number, a folder written by hand or before imports had numbers, a file that does not read). The launcher asks it before it hosts a save ([`../update.md`](../update.md), "Local saves").
