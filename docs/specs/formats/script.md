# Text scripts

The client describes models, motions, sounds, effects and maps in text files that share one syntax: model scripts (`.msm`), motion scripts (`.msa`), and others. This page covers the syntax, the kinds openm2 reads as data (race data, motion data with its combo, attack and event data, environments, sub-images), the monsters' motion lists, the monster list, the item list, the skill descriptions, the skill table, the maps' spawn lists and the locale setting, and where the scripts of player characters and monsters are, with the motions they fight with. Effect scripts (`.mse`) have a page of their own: [effect.md](effect.md); so do sound scripts (`.mss`): [sound.md](sound.md).

**Code:** `m2_formats::script` (syntax, tree, writer), `script::race` (`RaceData`), `script::motion` (`MotionData` with `ComboInput`, `Attacking` and `MotionEvent`, motion lists), `script::environment` (`Environment`), `script::npc_list` (the monster list), `script::item_list` (the item list), `script::skill_desc` (the skill descriptions), `script::skill_table` (the skill table), `script::spawn_list` (the spawn lists), `script::sub` (`SubImage`); `m2_assets::map::read_spawn_list` (a map's spawn list); `m2_assets::actor` (where the scripts of characters and monsters are, and their fight motions), `m2_assets::items` (the locale setting, items with their icons), `m2_assets::skills` (skills with their icons and motions). Commands: `openm2 script dump|motion|check`; `openm2 view model` reads model and motion scripts, `view map` environments, `openm2 play` all of them.

**Source:** the files of a TMP4 40250 client, examined with `openm2 script dump` and `script check --census` (`docs/source-policy.md`, "Allowed inputs"). No other source.

**Verified against:** that client; the motion scripts' combo, attack and event data with the hit positions compared to the bones of the posed animations (warrior sword combos, a fist, a monster's swing); the race data's collision spheres placed on the wild dog, a cyclops and a metin stone in their wait motions; its attached effects and the metin stones' smoke drawn on a metin stone. All 15,334 scripts parse (17,289 with the 1,955 sub-images), and the model, motion and environment scripts read as data except 5 (see the notes in each section). The counts below come from `script check`.

## Which files
| Files | `ScriptType` | In the client | What | Read as data |
|---|---|---|---|---|
| `.msm` | `RaceDataScript` | 605 | characters, monsters, NPCs, buildings: models, hair, collision | yes ([race data](#race-data-msm)) |
| `.msm` | `ItemDataScript` | 2,271 | items: model, icon, drop model, hit effects | no |
| `.msm` | `ObjectDataScript` | 1 | one object | no |
| `.msa` | `MotionData` | 6,027 | one motion: its GR2 animation, events, combo and attack timing | yes ([motion data](#motion-data-msa)), but `LoopData` |
| `.mss` | `CharacterSoundInformation` | 3,509 | sounds played during a motion or an effect | yes ([sound.md](sound.md), "Sound scripts") |
| `.mse` | none | 1,148 | effects (particles, meshes) | yes ([effect.md](effect.md)) |
| `.msf` | none | 98 | flying effects (projectiles) | yes ([effect.md](effect.md), "Flying effects") |
| `.msk` | `SkillDataScript` | 246 | skills (in the characters' `skill` folders, and guild skills): flags, weapons, cooldown and mana formulas, range, icon | no (openm2 reads the [skill descriptions](#skill-descriptions-skilldesctxt)) |
| `.msenv` | `EnvrionmentData` (sic) | 57 | lights, fog, sky | yes ([environment](#environment-msenv)) |
| `.txt` | `MapSetting`, `MapProperty`, `AreaProperty` | 86, 89, 1,197 | maps | `MapSetting` ([`map.md`](map.md)) |
| `.sub` | none (`title subImage`) | 1,955 | a rectangle of an interface image | yes ([sub-images](#sub-images-sub)) |

Not in this syntax:
- motion lists (`motlist.txt`), the monster list (`npclist.txt`), the item lists (`item_list.txt`), the skill descriptions (`skilldesc.txt`) and the maps' spawn lists (`regen.txt`, `monsterareainfo.txt`), line formats of their own ([motion lists](#motion-lists-motlisttxt), [monster list](#monster-list-npclisttxt), [item list](#item-list-item_listtxt), [skill descriptions](#skill-descriptions-skilldesctxt), [spawn lists](#spawn-lists-regentxt-monsterareainfotxt));
- the locale setting (`locale.cfg`), a loose file in the client folder ([locale setting](#locale-setting-localecfg));
- property files (`.prb`, `.prt`, `.pre`, …), which start with `YPRT`, and the map editor's block files (`areadata.txt`, texture sets) ([`map.md`](map.md));
- `.mde`, `.mdatr` and `.spt`, which are binary.

## Syntax
```text
ScriptType            RaceDataScript
BaseModelFileName     "d:/ymir work/pc/warrior/warrior_novice.gr2"

Group HairData
{
    PathName          "d:/ymir work/pc/warrior/"
    HairDataCount     1
    Group HairData00
    {
        HairIndex     0
        Model         "hair/hair_1_1.gr2"
    }
}
List HitEffectFileName
{
    "d:/ymir work/effect/hit-all/00000-3/00000-3.mse"
}
```

**Lines and values:**
- **Line ends:** `\r\n` in every file of the client; a bare `\n` is read the same. Blank lines, and lines of only spaces, are skipped.
- **Values:** separated by spaces and tabs, in any number. Files align their columns with either.
- **Quoted values:** a value in `"…"` keeps its spaces, and ends at the next `"` on the same line. There are no escapes. Paths are quoted, numbers and words mostly are not.
- **Entries:** any other line is an entry: a key, then its values. A key may have no value (`CycleLoopEnable`).
- **No comments:** no file has them.

**Blocks:**
- `Group Name` opens a block of entries, `List Name` a block of rows (one row of values per line).
- The next line must be `{`, and a line `}` closes the block. Text after `{` on its line is only whitespace in the client (`{` and a tab).
- Blocks nest; the client's go 3 deep.

**Keys, names and text:**
- **Case:** keys are not written consistently: 55 motion scripts write `flyfilename` where the others write `FlyFileName`. openm2 looks keys and block names up ignoring ASCII case; the first match counts.
- **Codepage keys:** keys may start with digits: `949Description` and `936Description` hold an item's text for one codepage.
- **Odd values:** some values are not plain numbers (`TailColor 1979708672d`). The tree keeps every value as text, and only the readers below convert them.
- **Codepage:** text is in the client's codepage (EUC-KR for Korean names). It stays bytes (`RawStr`).

**What openm2 refuses, with the line number:**
- a block that is not closed;
- a `}` outside a block;
- a `{` without `Group` or `List` before it, or inside a list;
- a missing `{` after a block name;
- an unclosed quote;
- a quoted key;
- blocks more than 16 deep;
- files over 4 MiB (the client's largest script is 92,135 bytes).

**Writing** (for test data):
- `\r\n` line ends, four spaces of indent per block, a tab between a key and its values.
- Values are quoted when they were quoted, or when they need it: empty, with spaces, starting with a brace.
- **Refused:**
  - a `"` or a line break inside a value;
  - a name that is not one word;
  - `Group` or `List` as a key with one value, which would read back as a block.

`openm2 script dump` prints a script one value per line with the block path in front: `HairData.HairData00.Model "hair/hair_1_1.gr2"`, with list rows as `HitEffectFileName[0] …`.

## Race data (`.msm`)
`ScriptType RaceDataScript`. A monster's script names its model; a player character's names one body model per armour ("shapes") and the hair styles. Each shape and hair style can swap the model's textures for others ("skins").

| Key | Value | openm2 |
|---|---|---|
| `BaseModelFileName` | the model, or the body before a shape is chosen (game path) | `base_model` |
| `AttributeFileName` | collision data of a building (`.mdatr`) | `attribute_file` |
| `MotionListFileName` | the motion list, when it is not `motlist.txt` beside the script (1 script) | `motion_list` |
| `Group ShapeData` | body models by shape index | `shapes` |
| `Group HairData` | hair models by hair index | `hairs` |
| `Group AttachingData` | collision spheres (blocks of `AttachingDataType` 1) and effects (2) | `collisions`, `effects` |
| `SmokeBoneName`, `List SmokeFileName` | a metin stone's smoke | `smoke` |

**`ShapeData` and `HairData`:**
- **The block:** `PathName` (a folder ending in `/`), `ShapeDataCount` or `HairDataCount`, then that many blocks `ShapeData00`, `ShapeData01`, … The number has at least two digits (`ShapeData100` after `ShapeData99`).
- **Each entry:**
  - `ShapeIndex` or `HairIndex`: the number the game picks it by.
  - `Model`: a file name in the folder.
  - `SpecialPath`: an optional folder used instead of `PathName`.
  - `SourceSkin`/`TargetSkin` and `SourceSkin2`/`TargetSkin2`: optional.

**`AttachingData`:**
- **The block:** `AttachingDataCount`, then that many blocks `AttachingData00`, … Each has an `AttachingDataType`: 1 for collision spheres, 2 for an effect on a bone (`EffectScriptName`, `EffectPosition`, `EffectRotation`).
- **Collision spheres (type 1):**
  - `CollisionType`: a number (`collision_type`, required).
  - `isAttaching`: 1 to place the spheres from the bone `AttachingBoneName`, 0 to place them from the model's origin (the bone is named but not used). Missing is 0.
  - `AttachingModelIndex`: 0 in every block; not read.
  - `SphereDataCount`, then that many blocks `SphereData00`, …, each with `Radius` (required) and `Position` (three numbers, in centimetres in the model's axes, forward being −y; missing is the origin).
- **Effects (type 2):** `EffectScriptName` (an effect script, required), `EffectPosition` (cm from the bone or the origin, in the model's axes), `EffectRotation` (degrees about x, y and z; in most), `isAttaching` and `AttachingBoneName` as for spheres. The effect plays for as long as the model shows ([`../render.md`](../render.md), "Effects").
- **How openm2 reads them:** blocks by count as below (at most 256 of each). A block without `CollisionType`, a sphere without `Radius` or an effect without `EffectScriptName` is an error.
- **Where a sphere on a bone is:** the bone's position plus the offset in the model's axes, not turned with the bone. That is openm2's reading. About two thirds of the client's spheres on a bone have no offset, and both readings agree. Most other offsets are along the height from `Bip01` (`0 0 -50`, `0 0 -60`, …), where both agree as long as the bone stands upright. The metin stones' `(-50, 0, 100)` on a bone tilted 30° lands near the middle of the stone this way, and 40 cm off it turned with the bone.
- **What the client has** (`script check`):
  - 1,219 collision blocks in 593 of the race data scripts it reads, each with one sphere; every monster's script has some. 52 effect blocks in 37 scripts: the six metin stones of `monster/metinstone_02/` (`metinstone_loop_bluegreen.mse` and others), bosses, a campfire, guild war flags, signal fires and warp gates. One names an effect the client lacks (`effect/monster2/yellowred1_big.mse`).
  - `CollisionType` is 1 (591 blocks) or 3 (628). Most scripts have one of each; some have several of each, or only type 3.
  - Counted in the scripts' text (not by `script check`): of the 298 monster scripts with one of each, 292 give the two the same centre and bone. Type 3 is the larger in 289 (a person: 40 and 70 cm; the wild dog: 60 and 90), the smaller in 6 (orcs) and the same in 3. What each type is for is open (`docs/roadmap.md`, "Hit bodies").
  - 744 blocks are on a bone (most on `Bip01`, others on objects' own bones), 475 placed from the origin.
  - The wild dog's two spheres are placed from its origin, 1 m and 80 cm up, and reach well over its back: it stands 89 cm high, in a box 50 by 140 cm.

**How openm2 reads them:**
- **Count and blocks:** the count says how many blocks there are, and they are read from `00` up. A missing block is an error; blocks past the count are ignored. No block lists an index twice in the client.
- **Paths:** a name resolves to the entry's folder followed by the name: `d:/ymir Work/pc/warrior/` + `hair/hair_1_1.gr2`. The VFS then normalizes it.
- **Skins:**
  - The model's texture whose file name equals `SourceSkin`'s file name (ignoring case) is drawn with `TargetSkin` instead, resolved in the same folder.
  - Without `TargetSkin` the texture stays.
  - The sura's cloaks use the second pair.
  - Matching by file name is openm2's rule. In the client, 6 of 1,885 sources are not among their model's textures.
- **Smoke:** `SmokeBoneName` names a bone; `List SmokeFileName` rows are a stage number and an effect script. The 20 scripts of `monster/metinstone/` (the metin stones `8001` and on) have four stages each, `metinstone_loop_1.mse` to `metinstone_loop_4.mse`, each stronger than the last. openm2 takes the stage to be how many quarters of its health the stone lost (openm2's reading, [effect.md](effect.md)).
- **Not read:**
  - `DefaultHitEffectFileName` (71 scripts, every list empty), `DefaultHitSoundFileName`;
  - `PartCount` (15 scripts), `ShapeSkinFileName` (2 tigers), `TreeFileName` (9 guild trees, which have no `BaseModelFileName`);
  - the `LOCAL_…` keys below.

**What the client has:**
- **Player characters:**
  - `<class>_m.msm` and `<class>_w.msm` sit at the root of the VFS for the four classes, with copies under `d:/ymir work/pc2/` and `season1/pc/`.
  - Each class keeps one gender's models in `pc/` and the other's in `pc2/`. Warriors and suras are male in `pc/`; assassins and shamans are female in `pc/`.
  - Their scripts name no motions: a character's motions lie in `general/` beside its base model (`d:/ymir work/pc/warrior/general/wait.msa`, `run.msa`). Every class and gender has `wait` and `run` there.
  - With a weapon in hand they come from the weapon's folder beside it instead: `onehand_sword/`, `twohand_sword/` (warriors), `dualhand_sword/`, `bow/` (assassins), `onehand_sword/` (assassins and suras), `bell/`, `fan/` (shamans); each has `wait` and `run`. See "Worn items".
  - The server's class and gender value picks the script ([`../protocol/openm2/session.md`](../protocol/openm2/session.md), "Characters"): 0 `warrior_m`, 1 `assassin_w`, 2 `sura_m`, 3 `shaman_w`, 4 `warrior_w`, 5 `assassin_m`, 6 `sura_w`, 7 `shaman_m` (`m2_assets::actor::player_script`).
- **Shape indexes** seen:
  - 0–30: armours;
  - 100–103: the polymorph animals (wolf, boar, bear, tiger), with a `SpecialPath` into the monster folders;
  - 201: wedding clothes;
  - 40031 and up: costumes.

  An armour's fourth item table value is its shape index (see "Worn items").
- **Counts:** 1,330 shapes and 617 hair styles. 77 of their 1,947 models and 122 of 1,920 skin textures are not in the client: event costumes that were not shipped, mostly.
- **`LOCAL_` keys:** the four `season1/pc/*.msm` scripts give some entries only `LOCAL_Model`, `LOCAL_SourceSkin` and `LOCAL_TargetSkin`, with paths from the client root. openm2 does not read them: those four scripts fail as unsupported.

## Motion data (`.msa`)
`ScriptType MotionData`. One motion of a character or monster.

| Key | Value | openm2 |
|---|---|---|
| `MotionFileName` | the GR2 file with the animation (game path; sometimes with `\` and in capitals) | `motion_file` |
| `MotionDuration` | length in seconds | `duration` |
| `Accumulation` | how far the motion moves the character, x y z in cm, forward being −y (walking, running and attacking, 1,734 scripts) | `accumulation` |
| `Group ComboInputData` | when a player's attack takes the next one of a combo | `combo` |
| `Group AttackingData` | when and where the motion hits | `attacking` |
| `Group MotionEventData` | timed events: effects, attack spheres, flying things | `events` |

Not read: `LoopData` (60 skill motions: a part that repeats; openm2 plays them once through), `CancelEnableSkill` (34 combo motions).

**In the client:**
- **Missing animations:** 31 of the 6,026 motion files are missing.
- **One damaged script:** `pc2/shaman/attractive.msa` writes its path without the opening quote, so it does not read.
- **`MotionDuration`** is not always the animation's length: 450 of 5,992 differ by more than 1 ms, the most by 33 s (`pc/sura/action/dance_4.msa` says 1 s, and its animation lasts 34.3 s).
  - openm2 plays the animation's own length, and `view model` notes the difference.
  - Which one the original client uses is open.
- **Moving motions do not move the model:** the animations play in place. In the warrior's sword `combo_01` (`Accumulation` 132 cm forward) and `combo_04`, `run` and `dead`, the root bone ends where it started, lunging at most 40 cm on the way. The client moves the character itself.

### Combo input (`ComboInputData`)
Four times in seconds into the motion: `PreInputTime`, `DirectInputTime`, `InputLimitTime`, and `LinkTime` (59 of 168). Only players' attack motions have it: the weapon folders' `combo_01.msa` … `combo_07.msa` (the ninja's daggers to `combo_08.msa`), the single attacks (`attack.msa` …) in the weapon folders and in `general/`.

**openm2's reading, not confirmed:** a press from `PreInputTime` to `InputLimitTime` asks for the next attack, which starts at `DirectInputTime` at the earliest (`ComboInput::accepts`). `LinkTime` is not used.

**In the client:** in every weapon folder `combo_04` and `combo_07` (the daggers' `combo_08` too) take no next attack: all three times are the motion's length, or `PreInputTime` comes after `InputLimitTime` (the warrior's sword `combo_04`). So they end a chain: `combo_01` to `combo_04`, and `combo_05` to `combo_07`. In which order the original client plays them is open (`docs/roadmap.md`, "Combo steps"). The single attacks in `general/` take none either.

### Attack data (`AttackingData`)
How the motion hits, in one of two layouts:
- **Older** (47 scripts): `AttackingType`, then one hit's keys in the group itself.
- **Newer** (371): `AttackType`, `MotionType`, `HitLimitCount` (most), and `HitDataCount` blocks `HitData00`, … (0 to 3).

Both have `HittingType`, `StiffenTime`, `InvisibleTime` and `ExternalForce`. What `AttackType` (0, 1), `AttackingType` (0–2), `MotionType` (0–3, 2 in the players' combos) and `HittingType` (0–2) mean is not known; openm2 keeps them as numbers.

**A hit:** `AttackingStartTime` and `AttackingEndTime` (seconds), `AttackingBone` (the bone that strikes: `equip_right_hand`, `Bip01 R Hand`, empty for none), `WeaponLength` (cm, 0 for a fist or a bite), and `List HitPosition`. A hit whose times are both 0 strikes nothing: those motions hit with an attack event instead (the warriors' `combo_04`).

**Hit positions:** one row per 1/120 s of the hit, seven numbers: the time, then two points in the model's space (cm, forward −y, up +z):
- the first is where the striking bone is at that time in the motion's animation: in the warrior's sword combos within 0.5 cm of where openm2 poses `equip_right_hand`, in a fist's and a monster's motions within about 30 cm;
- the second lies along the bone's z axis, `WeaponLength` further: the weapon's tip. 8,689 of the client's 9,265 rows are `WeaponLength` apart within 1 cm; the others are in some monsters' and the warriors' `combo_05` and `combo_06`.

openm2 reads the line between the two points at any time of the hit by blending the rows around it (`Hit::segment_at`), and how far a motion reaches along the ground from the character (`MotionData::reach`: the farthest point of its rows and attack spheres). The warrior's first sword swing hits from 0.192 to 0.315 s and reaches 2 m.

**In the client:** 418 scripts, 462 hits (446 with a time), 9,265 rows. One row (`monster2/ch_general/25.msa`) has text run into a number (`148.058609monster2`); openm2 leaves rows that are not seven numbers out.

### Motion events (`MotionEventData`)
`MotionEventDataCount` blocks `Event00`, …, each with `MotionEventType` and `StartingTime` (seconds), then keys by type:

| Type | Events | Keys | openm2 |
|---|---|---|---|
| 1 | 1,362 | `EffectFileName` (`.mse`), `EffectPosition`, `AttachingEnable` and `AttachingBoneName`, `FollowingEnable`, `IndependentFlag` (most) | `EventKind::Effect`: an effect on a bone or at the character |
| 2 | 218 | `DuringTime`, `Power`, `AffectingRange` | `EventKind::Shake`; what it does is openm2's guess: it comes just before the ground-stomping hits of `combo_04` and `combo_07` |
| 4 | 662 | `DuringTime`, `EnableHitProcess` (most), `AttackType`, `HittingType`, `StiffenTime`, `InvisibleTime`, `ExternalForce`, `HitLimitCount` (most), `CollisionType`, `SphereDataCount` blocks `SphereData00`, … with `Radius` and `Position` | `EventKind::Attack`: spheres around the character that hit for `DuringTime` |
| 6 | 234 | `FlyFileName` (`.msf`; 55 scripts write `flyfilename`), `FlyPosition`, `AttachingEnable`, `AttachingBoneName` | `EventKind::Fly`: an arrow, a spell |
| 10 | 131 | `EffectFileName`, `EffectPosition`, `FollowingEnable`, `FishingEffectFlag` (most) | `EventKind::SpotEffect`: an effect on no bone |
| 7, 8, 9 | 37 each | none | `EventKind::Other`: in the ninja's dash skills and some bosses' special attacks |

- Flags are 0 or 1; three `EnableHitProcess` are written `0.000000`, which reads as 0.
- `CollisionType` is 4 but in 4 events (0).
- Two attack events (`pc*/assassin/onehand_sword/combo_04.msa`) also carry the older layout's hit keys (`AttackingType`, `AttackingStartTime`, …); openm2 does not read those.
- The warriors' `combo_04` and `combo_07` stomp: a sphere of 1 m, 1.2 m ahead, for 0.2 s, reaching 2.2 m.

**In the client:** 2,718 events in 1,138 scripts, 662 attack spheres; at most 10 events (or hits, or spheres) in one block. 115 of the 1,727 effect and flying files the events name are not in the client. The effects of types 1 and 10 are mostly in skills, fishing and bosses' attacks: no warrior's combo step plays one. How `openm2 play` plays them: [`../render.md`](../render.md), "Effects".

**What openm2 refuses, with the block:**
- a group missing a key the client always writes: `ComboInputData` without one of its first three times, a hit without `AttackingStartTime` or `AttackingEndTime`, `AttackingData` without `HittingType`, `StiffenTime`, `InvisibleTime` or `ExternalForce`, an event without `MotionEventType` or `StartingTime`, an event of type 2 or 4 without its times and numbers, a sphere without `Radius`;
- a count above 256, or a block missing below its count (`HitDataCount is 2, but there is no block HitData01`).

**Writing** (for test data): the older layout when `attacking_type` is set (one hit), else the newer; the keys in the client's order.

### Attack motions
Where the motions a character fights with are (`m2_assets::actor`):
- **Players:** the combo steps `combo_01.msa` … in the held weapon's motion folder (none bare-handed); the single attacks `attack.msa`, `attack_1.msa`, `attack_2.msa`, the flinches `damage.msa` … `damage_3.msa`, each from the weapon's folder or else `general/`; the fall `dead.msa` in `general/`. A warrior with a sword has 7 combo steps, 3 single attacks, 4 flinches and a fall.
- **Monsters:** in their motion list, `NORMAL_ATTACK` with its numbered variants, `FRONT_DAMAGE` (else `DAMAGE`) and `FRONT_DEAD` (else `DEAD`, 54 lists). The wild dog has two attacks of weight 50, a flinch and a fall.
- How `openm2 play` plays them: [`../render.md`](../render.md), "Playing"; how our character swings through a combo by its windows and hits by its hit positions and spheres: "Our fighting" there.

## Motion lists (`motlist.txt`)
Monsters, NPCs and guild objects list their motions in a `motlist.txt` beside their model script, one motion per line:

```text
GENERAL WAIT 00.msa 65
GENERAL WAIT1 00_1.msa 35
GENERAL RUN 03.msa 100
```

**Fields:** the mode, the motion's name, the motion script (a file name beside the list), and a weight. Values are separated by spaces or tabs, blank lines are skipped, and a line with another number of values is an error.

**In the client:**
- **Files:** 378 lists with 4,649 motions; 4 of the motion scripts are missing.
- **Mode:** always `GENERAL`.
- **Names:**
  - `WAIT`, `WALK`, `RUN`, `NORMAL_ATTACK`, `COMBO_ATTACK`, `SPECIAL`, `SPAWN`, `SKILL2`, `COMMAND1`–`3`;
  - `FRONT_`/`BACK_` + `DAMAGE`, `DEAD`, `KNOCKDOWN`, `STANDUP`;
  - variants with a number after the name (`WAIT1`, `WAIT2`).
- **Weights:** 3,155 of 3,437 motions add up to 100 with their numbered variants (`WAIT 65`, `WAIT1 35`). The others do not (`BACK_DAMAGE 100`, `BACK_DAMAGE1 100`). What the client does with the weight is open.

`view model <script.msm> --motion RUN` finds `RUN` (ignoring case) in the script's list, then the motion script, then the animation. `openm2 play` stands monsters with `WAIT` and runs them with `RUN` (else `WALK`).

## Monster list (`npclist.txt`)
At the root of the VFS: which model each monster number (the `race` of an entity, its vnum in `mob_proto`) uses. One entry per line:

```text
101	stray_dog
104	wolf_blue
0	wolf_blue 	wolf
693	#season1/monster/orc_lord_e/
```

- **A number and a name:** the monster uses that model. Its script is `<name>.msm` in a folder named like the model, under `d:/ymir work/monster/`, `monster2/`, `npc/` or `npc2/`: `d:/ymir work/monster/stray_dog/stray_dog.msm`.
- **`0`, a name and a folder:** the model lies in that folder instead: `wolf_blue` is `d:/ymir work/monster/wolf/wolf_blue.msm`.
- **A name starting with `#`:** a folder path from the client root; the script there is `shape.msm` (`season1/monster/orc_lord_e/shape.msm`).
- Values are separated by tabs or spaces (some lines have a space before a tab, or a tab at the end); blank lines are skipped. A line with another number of values, a number that is not one, or a folder on a line whose number is not 0 is an error.
- The monster's motion list is the `motlist.txt` beside its script.

**openm2's rules, not confirmed:** the folders are tried in the order above, and the first line of a number wins.

**In the client:** 1,473 lines: 1,343 monsters, 130 folder lines, 76 `#` paths. 7 numbers are listed twice (6 patrol guards with a name and then a `#` path, one seal stone twice). 1,313 of the 1,336 numbers lead to a script in the client (377 in `monster/`, 384 in `monster2/`, 410 in `npc/`, 66 in `npc2/`, 76 by path); the 23 others name models the client does not ship (gnolls, trolls, nagas, …). Taking the last line of a number instead finds 2 fewer. `script check` counts them.

### Worn items
What a character wears changes its model. The server sends the item numbers of the body armour and the weapon (Quantum Core X: the first two of an entity's parts, `../protocol/openm2/entities.md`); the client's own files say how they look.
- **Body armour** (wear flag 1): its fourth value in the item table (`values[3]`, [`proto.md`](proto.md)) is the `ShapeIndex` of the character's model script. In a TMP4 client the warrior plate armours count up from Monk Plate Armour 3 to Black Steel Armour 12, and the script's shapes 3 to 12 are the warrior's armour models (`warrior_nahan.gr2` for 3 to 5, `warrior_saja.gr2` 6 to 8, `warrior_cheongrin.gr2` 9 and 10, `warrior_yongsin.GR2` 11, `warrior_4-1.GR2` 12); the Wild Boar Armour's value is 101, the boar shape. A shape the script lacks leaves the base outfit (openm2's choice).
- **Weapon** (item type 1): its model is the one the item list names for it (see "Item list"), a rigid mesh on one bone of its own (`d:/ymir work/item/weapon/00010.gr2`). The character holds it at a bone of its body model: warriors' skeletons have `equip_right_hand`, the other classes' `equip_right` (and assassins and shamans `equip_left`). openm2 puts the model's origin at the bone, the right hand's for most weapons, the left's for bows (subtype 2), both for daggers (subtype 1); which hand a bell or a fan goes to is openm2's guess (the right).
- **Motions:** the weapon's subtype (Quantum Core X's weapon types: 0 sword, 1 dagger, 2 bow, 3 two-handed, 4 bell, 5 fan) picks the motion folder of the same name: `onehand_sword`, `dualhand_sword`, `bow`, `twohand_sword`, `bell`, `fan`. A class without that folder keeps `general/` (openm2's choice).
- Not drawn yet: helmets and the rest (the original shows none of them on the model, as far as openm2 knows), costumes (shapes from 40031), the shininess of upgraded items (`specular` in the item table).

## Item list (`item_list.txt`)
In each locale folder (`locale/en/item_list.txt`): the icon of each item number (its vnum in `item_proto`, [`proto.md`](proto.md)) and the model it shows on the ground. One item per line, values separated by tabs (shown as `→`):

```text
10→WEAPON→icon/item/00010.tga→d:/ymir work/item/weapon/00010.gr2
8000→WEAPON→icon/item/08000.tga
18000→ETC→icon/item/18000.tga→→
```

- **Values:** the item number, a kind word (`WEAPON`, `ARMOR`, `ETC`), the icon's game path, and on some lines the ground model's game path. A weapon's model is also the one a character holds ("Worn items"): the client names no other model for it.
- **Items without a ground model** (armour, most others): the client's program names `d:/ymir work/item/etc/item_bag.gr2`, a bag that no line of the list names; openm2 shows those items as the bag (its reading, `docs/specs/render.md`, "Items on the ground").
- **Separators:** tabs only; paths may hold spaces (`d:/ymir work/…`). Spaces around a value are not part of it. Empty values at the end of a line (two tabs in a row) are left out; an empty value in the middle is an error.
- Lines end with `\n` (the locales' lists) or `\r\n`; blank lines are skipped. A line with fewer than three or more than four values, or a number that is not a plain decimal, is an error.
- **openm2's rules, not confirmed:** the first line of a number wins (its icon and its model); paths are looked up like any game path (lower case, `/`).
- **Icons:** TGA images, 32 px wide and 32 px per inventory cell tall (an item of size 2 has a 32 × 64 icon), drawn whole.

**In the client:** the 15 locales' lists are the same file: 6,437 lines, 203 numbers listed twice (29 of them with another icon the second time, `71002` with `70003.tga` and `71002.tga`), 1,535 with a ground model. Every ground model is in the client; 4 icons are not (3 files: `icon/item/31021.tga`, `31054.tga`, `51549.tga`). 386 of the 5,743 items of the English item table have no line, and 877 lines name items the table does not have. Two more lists, `d:/ymir work/item/936item_list.txt` and `949item_list.txt` (4,129 lines each, `\r\n`), name `.sub` icons under `d:/ymir work/ui/items/` that the client does not have; openm2 does not read them. `script check` counts all this.

## Skill descriptions (`skilldesc.txt`)
In each locale folder (`locale/en/skilldesc.txt`): each skill's job, names, description, how it is used, and the name of its icon and motion. One skill per line, values separated by tabs (shown as `→`), many empty:

```text
1→WARRIOR→Three-Way Cut→Triple Cut→Triple Slash→Cut the enemy with lightning speed.→Frontal Slash Attack 3 times→Attack on Multiple Targets→→→ATTACK_SKILL|NEED_TARGET|WEAPON_LIMITATION→SWORD|TWO_HANDED→samyeon→1→4→→→Total Attack Power %.0f-%.0f→(1.1*MinATK + …)→(1.1*MaxATK + …)→→
121→SUPPORT→Leadership→→→Ability to lead partners.→→→→→PASSIVE→→tongsol→→→→→→→→→
```

| Value | What | openm2 |
|---|---|---|
| 1 | the skill's number, as the server numbers skills ([`skills.md`](../protocol/openm2/skills.md)) | `vnum` |
| 2 | the job: `WARRIOR`, `ASSASSIN` (the ninja), `SURA`, `SHAMAN`, or `SUPPORT`, `HORSE`, `GUILD` for any job's | `job` |
| 3–5 | three names: the first is the skill's; the 44 job skills have a second and a third, what for is not known | `names` (the first is shown) |
| 6 | a sentence about it | `description` |
| 7–10 | up to four short lines about it | `notes` |
| 11 | flags joined by a bar (`\|`): `ATTACK_SKILL`, `NEED_TARGET`, `CAN_USE_FOR_ME`, `ONLY_FOR_ALLIANCE`, `WEAPON_LIMITATION`, `STANDING_SKILL`, `PASSIVE`, `TOGGLE`, `CANNOT_LEVEL_UP`, … (26 combinations) | `flags` |
| 12 | with `WEAPON_LIMITATION`, the weapons it needs joined by a bar: `SWORD`, `TWO_HANDED`, `DAGGER`, `DOUBLE_SWORD`, `BOW`, `FAN`, `BELL` | `weapons` |
| 13 | the name of its icon and of its motion script (`samyeon`) | `icon` |
| 14 | its motion number | `motion` |
| 15 on | a number (4, 1 or empty), two formulas, then lines with a format and one or two formulas each (what a level of the skill does, shown in the original's tooltip) | kept as they are (`rest`) |

- **Separators:** tabs only; spaces around a value are not part of it (68 values of the English file end with one). Empty values at the end of a line are left out. A flag or weapon list is split at `|`.
- Lines end with `\n`; blank lines are skipped. A line needs a number, a job and a name; a number or motion number that is not a plain decimal is an error.
- **openm2's rules:** the first line of a number wins (no number is listed twice); names and descriptions are decoded with the locale's codepage (`locale.cfg`).
- **Icons:** a job's skill has three, one per grade: `d:/ymir work/ui/skill/<job>/<icon>_01.sub` to `_03.sub` (`<job>` the job word in lower case), sub-images of `d:/ymir work/ui/skill<job>.dds`. openm2 shows them for normal, master and grand master (perfect masters show the third): its reading of the three pictures, the same symbol on a different ground. Support, horse and guild skills have one, `d:/ymir work/ui/skill/common/<job>/<icon>.sub` (`riding` has three).
- **Motions:** a job's skill plays `<icon>.msa` in the `skill` folder beside the character's base model (`d:/ymir work/pc/warrior/skill/samyeon.msa`, and in `pc2/` for the other gender), and `<icon>_2.msa` to `<icon>_4.msa` at master, grand master and perfect master: the same animation, other effects (Three-Way Cut's grand master plays `samyeon_3.mse` where normal plays `samyeon_d.mse`). Five shaman skills have no `<icon>.msa` but `<icon>_me.msa` and `<icon>_target.msa` (used on oneself or another, by their names), two skills (ninja 50, sura 81) only `<icon>.msa`. openm2 plays the grade's own, else the lower grades', else the higher ones', else `<icon>_me.msa` (`m2_assets::skills::skill_motion_paths`).
- **The motion number** of every job skill is its place in its job's numbering, skill − 30 × job: 1 to 6 and 16 to 21. It is the number the move packet carries ([`skills.md`](../protocol/openm2/skills.md), "Using a skill").
- **Beside it:** `locale/<x>/skilltable.txt`, the skill table ([below](#skill-table-skilltabletxt)). The motion folders also hold a `.msk` per skill (`SkillDataScript`: flags, weapons, `CoolTimeFormula`, `NeedSPFormula`, `Range`, a `MotionIndex` that is not the motion number, Korean names), which openm2 does not read either.

**In the client:** the 15 locales' files read: 66 lines each, 22 values (the French file has 2 lines of 23), the same numbers, jobs, flags, icons and motion numbers for every job skill. 44 skills belong to a job: the warrior's and the ninja's first five of each group (their sixth skills, 6, 21, 36 and 51, are not in this client), the sura's and the shaman's six. Every one of their icons is in the client, and a motion for every grade in both genders' folders: 5,280 grade motions in all, 330 of them another grade's or `_me` (`script check`). The Polish file gives guild skill 152 the values of horse skill 140, the Russian one leaves guild skill 151's icon name empty. A `936skilldesc.txt` at the archive root has another layout (20 to 29 values, `\r\n`); openm2 does not read it.

## Skill table (`skilltable.txt`)
In each locale folder (`locale/en/skilltable.txt`), beside the descriptions: each skill's job, levels, what it changes and by how much, its mana cost, cooldown and duration as formulas, its flags, kind and reach. Quantum Core X reads a file of this layout as its skill table (`data/skilltable.txt`); `openm2 content import` turns it into `skills.toml` ([`content.md`](../server/content.md), "The import"). One skill per line, 27 values separated by tabs, lines ending with `\r\n`. A made-up line:

```text
1→Test Strike→1→1→1→0→HP→-(2*atk+3*k)→10+20*k→→→10→→→ATTACK,USE_MELEE_DAMAGE→→NONE→→→→0→0→MELEE→2→1→0→100
```

| Value | What (Quantum Core X's reading) | openm2 |
|---|---|---|
| 1 | the skill's number | `vnum` |
| 2 | its name, in the client's codepage (Korean in the client's files) | `name` |
| 3 | whose it is: 0 every job's (passive), 1 warrior, 2 ninja, 3 sura, 4 shaman, 5 horse | `category` |
| 4 | the levels a step takes | `level_step` |
| 5 | its highest level (Quantum Core X reads it for passive skills only) | `max_level` |
| 6 | the lowest character level that may learn it | `level_limit` |
| 7 | the point it changes: `HP`, `ATT_SPEED`, `MOV_SPEED`, `ATT_GRADE`, `DEF_GRADE`, … or `NONE` | `point` |
| 8 | by how much, a formula (negative health is damage) | `amount` |
| 9 | its mana cost, a formula | `sp_cost` |
| 10 | how long its change lasts in seconds, a formula | `duration` |
| 11 | the mana a lasting change costs, a formula | `duration_sp_cost` |
| 12 | its cooldown in seconds, a formula | `cooldown` |
| 13 | a master's bonus, a formula | `master_bonus` |
| 14 | its attack grade, a formula | `attack_grade` |
| 15 | flags joined by commas: `ATTACK`, `SELFONLY`, `SPLASH`, `USE_MELEE_DAMAGE`, `USE_ARROW_DAMAGE`, `COMPUTE_MAGIC_DAMAGE`, `TOGGLE`, … | `flags` |
| 16 | the state it puts on its target | `affect` |
| 17–20 | a second point, its amount and duration, a second state | `point2`, `amount2`, `duration2`, `affect2` |
| 21, 22 | a skill to learn first and its level (0 for none) | `prerequisite`, `prerequisite_level` |
| 23 | `NORMAL`, `MELEE`, `RANGE` or `MAGIC` | `kind` |
| 24 | the most it hits | `max_hit` |
| 25 | the damage around its target, a formula | `splash_damage` |
| 26 | how far its target may be, in centimetres | `target_range` |
| 27 | how far around it hits, in centimetres | `splash_range` |

- **Reading:** only lines that start with a digit are skills (as Quantum Core X reads them); others, blank ones among them, are skipped. Tabs separate values, spaces around a value are not part of it; values left out at the end are empty, empty numbers are 0; more than 27 values, or a number that is not a plain decimal, is an error. `\n` alone ends a line too.
- **Writing** (for test data) puts all 27 values on every line with `\r\n`; a value that holds a tab or a line break or starts or ends with a space, or a flag that is empty or holds a comma, is refused.
- The formulas are kept as text here; openm2's server reads them with its own formula reader ([`content.md`](../server/content.md), "Formulas").

**In the client:** the English file has 77 lines of 27 values: 70 skills numbered 1 to 254 of the six categories, and 7 monster skills numbered 256 and up. Its formulas use the names `k`, `lv`, `str`, `dex`, `con`, `iq`, `atk`, `mwep`, `ar`, `odef`, `maxhp`, `maxsp` and `chain` and the functions `number` and `floor`; the import read every one. The other locales' files were not compared.

## Spawn lists (`regen.txt`, `monsterareainfo.txt`)
In some maps' folders (`metin2_map_c1/regen.txt`, `metin2_map_a1/monsterareainfo.txt`): where monsters, groups of monsters and NPCs live on the map. `openm2 content import` turns them into `spawns/imported.toml` ([`content.md`](../server/content.md), "The import"). Quantum Core X reads files of this layout as its spawn points (`maps/<map>/regen.txt`, `npc.txt`, `stone.txt`, `boss.txt`), which openm2 does not have. One entry per line, 11 values separated by tabs (spaces in some lines), lines ending with `\r\n`; the files' own first line names the values:

```text
//type→cx→cy→sx→sy→z→dir→time→percent→count→vnum
m→430→608→0→0→0→6→1m→100→1→1001
g→371→446→102→20→0→0→1m→100→12→7
```

| Value | What | Reading |
|---|---|---|
| type | `m` a monster, `g` a group of monsters, `r` a collection of groups, `e` and `s` (named by QCX, not used by it); an `a` after the letter makes its monsters aggressive (QCX) | `kind`, `aggressive` |
| cx, cy | the centre, in metres from the map's top-left corner (QCX: 100 cm a unit from the map's base) | `x`, `y` |
| sx, sy | how far either way of the centre its monsters may stand, in metres (QCX: a random point up to that far) | `range_x`, `range_y` |
| z | 0 on every line of the client; not used | `z` |
| dir | the way they face: 0 a random way, 1 south and then around the compass anticlockwise (2 south-east, 3 east, …, 8 south-west) in steps of 45 degrees (QCX, which turns only monsters that do not move) | `direction` |
| time | when they come back: a number and `s`, `m` or `h` (QCX); a bare number (`0` in `monsterareainfo.txt`) openm2 reads as seconds | `respawn`, in seconds |
| percent | a chance (QCX reads it and does not use it) | `chance` |
| count | how many (QCX reads it and does not use it). openm2 reads it as the number of monsters of an `m` line and of groups of a `g` line (on the client's maps, a group to every 3,700 m² of its area, the median) | `count` |
| vnum | the monster's number in `mob_proto`, or the group's or collection's number: those are the server's own, not in the client | `vnum` |

- **Reading:** lines starting with `//` are comments (a `//MonsterAreaInfoCount 96` line too), blank lines are skipped; values are separated by any run of tabs and spaces; a line without exactly 11 values, an unknown kind, a number that is not one (a negative count, direction or number among them), a direction past 255 or a time that is not as above is an error naming its line. `\n` alone ends a line too.
- **Writing** (for test data) puts the comment line first, then every line with tabs, `\r\n` and the time in seconds (`60s`).
- **Which file:** `regen.txt` if it has an entry, else `monsterareainfo.txt` (openm2's choice; no map of the client has entries in both).

**In the client:** 35 files in 33 map folders, 21 of them empty or with only the comment line. 1,259 lines: 52 monster lines (warps, NPCs, five monsters on `metin2_map_a1`), 443 group lines naming groups 1 to 68, 668 collection lines in `metin2_map_spiderdungeon_02`, and 96 monster lines in `metin2_map/monsterareainfo.txt`, a folder the atlas does not list. Every line has 11 values and a chance of 100; times are `1m`, `60s` or `0`; directions are set only on monster lines.

## Locale setting (`locale.cfg`)
A loose file in the client folder, not in an archive: three values separated by spaces, `10002 1252 en`.

| Value | Meaning |
|---|---|
| first | a number; its meaning is not known |
| second | the Windows codepage of the locale's texts (item names) |
| third | the locale's folder under `locale/` |

openm2 takes the folder only if it is letters, digits, `_` and `-`. An English client has `10002 1252 en` (and a `locale_<name>.cfg` per other locale beside it, which openm2 does not read). `openm2 play` reads the item table and item list of that folder for the inventory and decodes item names with codepage 1252; names in other codepages are shown as ASCII with `?` (`docs/roadmap.md`, "Open questions", "Names in other codepages").

## Environment (`.msenv`)
`ScriptType EnvrionmentData`, spelled so in every file. A map's light, material, fog, sky and lens flare; a map's `setting.txt` names one in `d:/ymir work/environment/` ([`map.md`](map.md)).

| Key | Values | openm2 |
|---|---|---|
| `DirectionalLight.Direction` | 3 numbers: the way the light travels (z negative, towards the ground) | `light_direction` |
| `DirectionalLight.Background.Enable`, `.Diffuse`, `.Ambient` | the light on terrain and objects: a flag, two RGBA colours | `background` |
| `DirectionalLight.Character.…` | the same for characters | `character` |
| `Material.Diffuse`, `.Ambient`, `.Emissive` | RGBA colours of the material the light falls on | `material` |
| `Fog.Enable`, `.NearDistance`, `.FarDistance`, `.Color` | distance fog: a flag, start and end in cm, RGBA | `fog` |
| `SkyBox.bTextureRenderMode` | 1: the sky is the box's pictures instead of the gradient | `sky.textured` |
| `SkyBox.Scale` | 3 numbers: the box's size in cm (3,500 in every file) | `sky.scale` |
| `SkyBox.GradientLevelUpper`, `.GradientLevelLower` | how many colour bands lie above and below the horizon | `sky.upper_bands`, `sky.lower_bands` |
| `SkyBox.FrontFaceFileName`, `.BackFaceFileName`, `.LeftFaceFileName`, `.RightFaceFileName`, `.TopFaceFileName`, `.BottomFaceFileName` | the box's six pictures, quoted; empty for none | `sky.faces` |
| `SkyBox.CloudScale`, `.CloudHeight` | the cloud layer's size (2 numbers) and height, in cm | `sky.cloud_scale`, `sky.cloud_height` |
| `SkyBox.CloudTextureScale`, `.CloudSpeed` | 2 numbers each: how often the clouds' picture repeats across the layer, and how fast it drifts | `sky.cloud_texture_scale`, `sky.cloud_speed` |
| `SkyBox.CloudTextureFileName` | the clouds' picture, quoted; empty for none | `sky.cloud_texture` |
| `SkyBox` `List CloudColor` | RGBA rows (two in every file) | `sky.cloud_colours` |
| `SkyBox` `List Gradient` | RGBA rows, two per band (its top, then its bottom): the upper bands from the highest down, then the lower ones from the horizon down. The client's files put a blank line between the bands | `sky.gradient` |

A row of `CloudColor` or `Gradient` that is not four numbers is an error. In the writer, the texture mode and the faces are written only when the sky is textured or names a face, as in the client's files.

Not read yet: `Filter` (a colour over the screen), `LensFlare`, `ScriptVersion`.

**In the client:** all 57 read. Every one has a light direction, background light, a material, fog and a sky; the fog is enabled in 43. The skies have 0 to 7 bands above the horizon and 0 to 2 below (one has none at all), always two gradient colours per band; 4 are drawn with pictures, five each (none has a bottom one); 43 name a picture of clouds. Of the 63 pictures named, one is not in the client (`clouds_zone06.tga`); `script check` counts them. How `view map` and `play` use them is in [`../render.md`](../render.md), "Maps: light and fog" and "Maps: the sky".

## Sub-images (`.sub`)
A rectangle of a larger image: the interface packs buttons, icons and digits into one texture, and each piece has a `.sub` file. Six keys, no blocks:

```text
title subImage
version 1.0
image "Public.dds"
left 194
top 142
right 255
bottom 163
```

| Key | Values | openm2 |
|---|---|---|
| `title` | `subImage` | checked |
| `version` | `1.0` or `2.0` | `version`; anything else is an unknown layout |
| `image` | a file name | `image` |
| `left`, `top` | the rectangle's first column and row, in pixels | `left`, `top` |
| `right`, `bottom` | the column and row just past it | `right`, `bottom`: the width is `right − left` |

- **Where the image is** (`m2_assets::sub_image_path`): version 1.0 names a file in `d:/ymir work/ui/`, wherever the `.sub` file is (`d:/ymir work/ui/public/close_button_01.sub` cuts from `d:/ymir work/ui/public.dds`, `metin2_map_milgyo/atlas.sub` from `d:/ymir work/ui/metin2_map_milgyo_atlas.dds`); version 2.0 names a file in the `.sub` file's own folder (`d:/ymir work/uiloading/catacomb/loading0.sub` cuts from `loading0.jpg` beside it).
- **Right and bottom are exclusive:** 125 rectangles end exactly on their image's right or bottom edge (`right` equals the image's width), which an inclusive reading would put one pixel outside; the version-2.0 loading screens are `0 0 1024 768` for 1024 × 768 pictures.
- An empty or inside-out rectangle is refused.

**In the client:** 1,955 files: 796 of version 1.0 (all under `d:/ymir work/ui/` but 7 map atlases), 1,159 of version 2.0 (hair icons, the locales' interface art, loading screens). Every image they name is in the client. Of the 1,884 that cut from a DDS or TGA image, 1,738 lie inside it, 125 end on its edge and 21 go past it: the six guild tabs (up to column 376 of the 256 × 32 `guild.dds`) and the locales' `login/title.sub` (443 × 206 in a 512 × 128 image). Those files look unused; openm2 cuts such a rectangle to the image. `script check` counts all this.

## Tools
| Command | What it does |
|---|---|
| `openm2 script dump <file>` | A script one value per line (`--client` for a game path) |
| `openm2 script motion <file>` | What openm2 reads from a motion script: the animation, how far it moves, the combo window, each hit (time, bone, weapon, reach), the events, and when and how far the motion hits |
| `openm2 script check --client <dir>` | Parse every script and motion list; follow the files that race and motion data name (animations, effects, flying effects, attached effects and smoke); count what is missing; count the combo, attack and event data, and the hit positions a weapon's length apart; read the environments; find each monster's script from the monster list; find the icons and ground models of the item lists; find each sub-image's image and compare its rectangle with the image's size; read the effect scripts and mesh effects ([effect.md](effect.md)) |
| `… --census` | Every key path of each script type, with the number of files that have it |
| `… --find <path>` | The files that have one key path (`ShapeData.ShapeData##.SpecialPath`) |
| `openm2 view model <x.msm> --shape N --hair N --motion NAME` | Draw a character or monster from its model script (`docs/specs/render.md`) |
