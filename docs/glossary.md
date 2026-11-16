# Glossary

Terms used in code and docs. Add a term when you meet one; don't guess.

| Term | Meaning |
|---|---|
| **40250 / "40k" client** | A widely shared client release (TMP4 "40250"). Its files are the asset source we support; openm2 speaks its own protocol, not the client's. |
| **Archive list (`pack/Index`)** | The client's list of archives in mount order (patches first), each with a path prefix. The VFS mounts in this order (`docs/specs/formats/eterpack.md`). |
| **Area** | A square of a map, 128 × 128 cells (256 m), stored in its own folder `{x:03}{y:03}` with its height, tile, attribute, water and shadow maps and its objects (`docs/specs/formats/map.md`). |
| **Atlas (`atlasinfo.txt`)** | The client's list of maps with the world position of each one's top left corner and its size in areas; how a world position is turned into a map (`docs/specs/formats/map.md`). Also the in-game window with a picture of the whole map and its NPCs and warps (`docs/specs/ui.md`, "The atlas"). |
| **Attribute map (`attr.atr`)** | One byte of flags per half cell of an area (a metre), for movement: `0x01` blocked and `0x02` water, seen in the client's maps; `0x04` perhaps no fighting; the rest unknown (`docs/specs/formats/map.md`). openm2's walk masks are made from `0x01`. |
| **Auth server** | First server the client talks to (port 11002). In openm2, the player signs in there with a provider, and it hands out a ticket. |
| **Bip01** | The root bone of the client's character skeletons (3D Studio MAX's Biped naming: `Bip01 Pelvis`, `Bip01 Spine`, …). Animation tracks name the bones they move. |
| **Capture (`.m2cap`)** | A recording of every frame of a session, as JSON Lines, with secret fields blanked. Written by `openm2 bot --capture`, read by `openm2 cap decode`. |
| **Cell** | The unit of a map's grid: `CellScale` cm wide (200 in every map). An area is 128 × 128 cells; a height map has a value per cell corner, a tile map two tiles per cell side. |
| **Character list** | The up-to-four characters of an account, sent in the select phase. |
| **Class** | A character's job: warrior, ninja (also called assassin), sura or shaman. On the wire one byte holds class and gender, 0–7 (`docs/specs/protocol/openm2/session.md`, "Characters"). |
| **Client folder** | The user's own installed game client (TMP4 40250). openm2 reads its `pack/` archives; nothing from it enters the repo. |
| **Codepage** | Strings in game files use legacy single-byte Windows codepages (1250, 1252, 1254, …) or EUC-KR, not UTF-8. openm2's protocol carries UTF-8. |
| **Combo** | A chain of attack motions a weapon plays one after another while the player keeps attacking (`combo_01.msa`, `combo_02.msa`, …). Each step is sent as a CharacterMove of kind 3, each hit as a Hit (`docs/specs/protocol/openm2/combat.md`). |
| **Container (`MCOZ`)** | A small header around LZO1X-compressed data, optionally XTEA-encrypted. Archive entries, encrypted indexes and proto tables use it (`docs/specs/formats/compression.md`). |
| **DDS / DXT** | The client's texture files (DirectDraw Surface) and their block compression: DXT1 (1-bit alpha), DXT3 (4-bit alpha), DXT5 (interpolated alpha). DXT3 alpha in character textures is a gloss mask, not transparency (`docs/specs/formats/dds.md`): where upgraded items shine. |

| **Direction** | `c2s` (client → server) or `s2c` (server → client). Packets are keyed by direction *and* header. |
| **Dungeon block** | A map object drawn with a GR2 model of a dungeon's walls and floor (`propertytype DungeonBlock`, `.prd`). |
| **BGM** | The client's music: MP3 pieces in its `BGM` folder (`m2bg.mp3`, `login_window.mp3`, …). Which plays where is chosen by the original's interface scripts; openm2's choice is in `docs/specs/audio.md`. |
| **Billboard** | A flat particle or mesh turned towards the camera; an effect script's `BillboardType` says how (facing the camera, upright, flat on the ground; `docs/specs/formats/effect.md`). |
| **Effect (core)** | What core asks its caller to do: connect, send, disconnect, report an event. |
| **Effect script (`.mse`)** | A visual effect: particle emitters and mesh effects with curves over time, played on map objects, actors' bones, hits and items (`docs/specs/formats/effect.md`; how it is drawn: `docs/specs/render.md`, "Effects"). |
| **Empire** | One of the three kingdoms: Shinsoo (red), Chunjo (yellow), Jinno (blue). Chosen per account. |
| **Entity** | Anything with a VID in the world: a player, a monster, an NPC, a metin stone, a portal. The server shows the ones within 100 m (`docs/specs/protocol/openm2/entities.md`). |
| **Entry type** | How an archive entry is stored: 0 as is, 1 LZO1X, 2 LZO1X + XTEA; 3–5 need keys from the server and are not supported (`docs/specs/formats/eterpack.md`). |
| **Environment (`.msenv`)** | A script with a map's light direction and colours, material, fog and sky (`docs/specs/formats/script.md`). |
| **EterPack** | The archive format of the TMP4 client: an index file (`.eix`) and a data file (`.epk`), with LZO1X compression and XTEA encryption. |
| **Game path** | How game data names a file: lowercase, `/`, often with a drive prefix (`d:/ymir work/pc/…`) left from the original developers' machines. The VFS looks files up by game path. |
| **Game server** | Server that runs the world (port 13001). The client logs in there with the ticket from the auth server. |
| **Ground item** | An item lying on the ground: an entity of its own (a VID) with an item number, a position and maybe an owner, the character who may pick it up (`docs/specs/protocol/openm2/ground-items.md`). |
| **GR2** | Granny 3D model and animation files: a memory image with pointers, describing its own layouts. Sections are Oodle1-compressed (`docs/specs/formats/gr2.md`). |
| **Frame** | One message on a connection: its length as a varint, then the message (`docs/specs/protocol/openm2/README.md`). |
| **Hello** | The client's first message on every connection: the protocol version it speaks. The server answers Welcome, or Refused and closes. |
| **Emitter / particle** | An effect script's `Particle` block makes particles (textured quads) for a cycle: where they are born, how fast, how long they live, how they look over their life (`docs/specs/formats/effect.md`). |
| **Height map (`height.raw`)** | An area's terrain heights: 131 × 131 `u16` values, the 129 × 129 vertices plus a border copied from the neighbours, in steps of `HeightScale` cm (`docs/specs/formats/map.md`). |
| **Height plane** | Triangles in a building's or dungeon block's model attributes (`.mdatr`) that are its floors: a platform, a bridge's deck, ramps over steps. `play` stands characters on the highest one over them (`docs/specs/formats/mdatr.md`, `docs/specs/render.md`, "Floors"). |
| **`keys.toml`** | The user's file with their client's XTEA keys (archive index, archive entries, item and monster tables). Never committed (`docs/specs/formats/keys.md`). |
| **LOD** | Level of detail: `_lod_01` … `_lod_03` beside a model are simpler versions of it for distant drawing. |
| **Loose file** | A game file lying in a folder instead of an archive. In the VFS, loose folders override archives. |
| **LZO1X** | The compression the client uses inside containers. openm2 has its own decoder. |
| **Message** | One unit of openm2's protocol: a varint id (per direction), then its fields (`packets/openm2.toml`). |
| **Main character** | The character the player plays, sent by the game server while loading (`MainCharacter`). |
| **Map** | A folder (`metin2_map_a1`) with `setting.txt` and a grid of areas. The server names maps by index; the client finds the map from a world position through the atlas (`docs/specs/formats/map.md`). |
| **Mesh effect (`.mde`)** | A binary file of small meshes animated frame by frame (rings, flags, waterfalls) that an effect script's `Mesh` block plays (`docs/specs/formats/effect.md`). |
| **Metin stone** | A large stone the game drops into the world (monsters `8001` and on): it does not move, monsters gather around it, and it smokes more as it loses health (its model script's `SmokeFileName` stages). |
| **Model attributes (`.mdatr`)** | A binary file beside a building's or dungeon block's model: its collision shapes (planes, spheres, cylinders) and its height planes (`docs/specs/formats/mdatr.md`). |
| **Model script (`.msm`)** | A text script naming the models of a character, monster or NPC (race data: base model, shapes, hair styles, skins) or of an item (`docs/specs/formats/script.md`). |
| **Motion list (`motlist.txt`)** | A monster's or NPC's list of motions: mode, name (`WAIT`, `RUN`, …), motion script and weight, one per line (`docs/specs/formats/script.md`). |
| **Monster list (`npclist.txt`)** | The client's list of which model each monster number uses (`docs/specs/formats/script.md`, "Monster list"). |
| **Motion script (`.msa`)** | A text script for one motion: the GR2 file with its animation, its length, and timed events; attacks also say when the next combo step may be asked for and when and where they hit (`docs/specs/formats/script.md`). |
| **Oodle1** | The compression of GR2 sections: LZ77 with adaptive range coding, three streams per section (`docs/specs/formats/oodle1.md`). Not RAD's later Oodle compressors. |
| **Movement speed** | An entity's speed value from the server: 100 is its run motion's own speed; players default to 150. A walk's duration follows from it (`docs/specs/protocol/openm2/movement.md`). |
| **Secret field** | A message field holding a ticket, a provider's code or a PKCE verifier (`secret = true` in the TOML): shown as `<redacted>` and emptied in captures. |
| **Script** | The client's text files that share the `Group`/`List` syntax: `.msm`, `.msa`, `.mss`, `.mse`, `.msf`, `.msk`, `.msenv`, map settings. `ScriptType` names the kind (`docs/specs/formats/script.md`). |
| **Shape** | A body model of a player character, picked by shape index: one per armour or costume, plus polymorph animals (`docs/specs/formats/script.md`). |
| **Shop** | What an NPC sells: 40 places, 5 wide and 8 high, opened by talking to the NPC; the server sells an item for its price and buys items of the bag (`docs/specs/protocol/openm2/shops.md`). Players' own shops are not handled. |
| **Skill** | Something a character does by its job: a motion with its own hits and effects, used from a quickslot or the skills window. Numbered 30 × job + 1 to 6 and 16 to 21; levels 1 to 40 (`docs/specs/protocol/openm2/skills.md`). |
| **Skill descriptions (`skilldesc.txt`)** | The locale's table of skills: job, names, description, flags, weapons, icon and motion name, motion number (`docs/specs/formats/script.md`). |
| **Skill table (`skilltable.txt`)** | The locale's table of what skills do: job, levels, the point each changes, its amount, mana cost, duration and cooldown as formulas (in `k`, the skill's power, and the character's numbers), flags, kind and reach; the server's skill table has this layout. openm2's import turns it into `skills.toml` (`docs/specs/formats/script.md`, `docs/specs/server/content.md`). |
| **Skill grade** | How far a skill is mastered: normal (levels 1 to 19), master (M1 to M10, levels 20 to 29), grand master (G1 to G10, 30 to 39), perfect master (40). Each grade has its own icon and may have its own motion (other effects); Quantum Core X calls it the master type. |
| **Skill group** | Which six of its job's twelve skills a character uses: group 1 the first six, group 2 the six from 15 further on; 0 before one is chosen (`docs/specs/protocol/openm2/skills.md`). |
| **Skill motion** | A skill's motion script (`samyeon.msa` in the character's `skill` folder) and its number in move messages: the skill's place in its job's numbering, sent as the move's kind with bit 7 set (`docs/specs/protocol/openm2/movement.md`). |
| **Skill points** | Point 28: points to spend on skills, one a level, until a skill becomes a master (`/skillup` on Quantum Core X). |
| **Skin** | A texture of a model swapped for another by a model script (`SourceSkin` → `TargetSkin`): armour colours, hair colours. In the interface, a named look in openm2's skin file (a board, a bar, a button; `docs/specs/ui.md`). |
| **Slot** | Position 0–3 of a character in the character list. |
| **Sound script (`.mss`)** | The sounds a motion or an effect plays and when (`ScriptType CharacterSoundInformation`), at the path of the motion or effect under `sound/` (`docs/specs/formats/sound.md`). |
| **Sound spot** | A place on a map where a sound plays around it, looped: a line of an area's `areaambiencedata.txt` naming an `Ambience` property (waterfalls; `docs/specs/formats/sound.md`). |
| **Sub-image (`.sub`)** | A text file naming a rectangle of a larger interface image (buttons, icons and digits packed in one texture); version 1.0 cuts from a file in `d:/ymir work/ui/`, 2.0 from one beside it (`docs/specs/formats/script.md`). |
| **Palette** | The skinning matrices of one part, one per bone of its skeleton plus an identity, uploaded for each frame (`docs/specs/render.md`). |
| **Part** | One model drawn as part of a character: the body (armour), the hair, later weapons. Parts follow the main skeleton by bone name (`docs/specs/render.md`). |
| **Points** | Our character's numbered values (level, health, mana, gold, stats, speeds), sent whole in Points while loading, then only those that changed in PointsChanged (`docs/specs/protocol/openm2/entities.md`). |
| **Phase** | The game connection's stage, set by the server: the character list (1), loading (2), the game (3). |
| **Property (`.prb`, `.prt`, …)** | A file saying what a map object is, found by the id the object names: a building or dungeon block (a GR2 model), a tree (a SpeedTree file, whose name picks one of openm2's own recipes), an effect, a sound (`docs/specs/formats/map.md`). |
| **Proto** | Item and monster tables (`item_proto`, magic `MIPX`; `mob_proto`, magic `MMPT`) shared by client and server. QCX reads the client's own files (`docs/specs/formats/proto.md`). |
| **QCX** | Quantum Core X, an open-source (MPL-2.0) Metin2 server; many of openm2's server rules follow its behaviour, noted "(QCX)" in the specs. |
| **Ticket** | 32 random bytes the auth server gives after a sign-in; the client presents it to the game server. Single use, 30 s. A secret. |
| **Target** | The entity the player chose to fight or look at (SetTarget); the server sends its health in percent, and the client shows it in the target window. |
| **Texture set** | A map's list of terrain textures (`textureset/*.txt`), numbered from 1; tile maps name textures by these numbers (`docs/specs/formats/map.md`). |
| **Tile (`tile.raw`)** | A square half a cell wide; an area's tile map gives each tile a texture number of the map's texture set. The renderer blends the textures of neighbouring tiles (`docs/specs/render.md`, "Terrain"). |
| **Track** | Where an entity is over time in `m2_core`: walking in a straight line from one point to another, starting at a time and taking a duration. |
| **TMP4** | A community name for a widely shared set of client and server files. |
| **VFS** | Virtual file system (`m2_assets::Vfs`): one namespace over loose folders and the client's archives, looked up by game path. |
| **VID** | Virtual id: the server's id for an entity (player, monster, NPC) currently in view. |
| **Vnum** | Static id of an item or monster type in the proto tables. |
| **Walk mask** | openm2's own: a map's squares a metre wide, open or blocked, as a PNG in the content folder (`docs/specs/server/content.md`). |
| **Water layer** | A water height in an area's `water.wtr`; each cell is under one layer or none (`docs/specs/formats/map.md`). |
| **Yang** | The game's gold: point 11, and item number 1 when it lies on the ground. |
| **XTEA** | The block cipher the client uses for encrypted containers. The keys belong to the user (`keys.toml`). |
