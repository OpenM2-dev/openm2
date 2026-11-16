# Proto tables: `item_proto` and `mob_proto`

The client's item and monster tables. They are files in the client's archives (`locale/<name>/item_proto`, `locale/<name>/mob_proto`). Quantum Core X reads the very same files from its `data/` folder.

**Verified against:**
- A TMP4 40250 client: the item and monster tables of all 15 locales parse. Items are version 1 with 156-byte records, and the decoded sizes match count × record size exactly, for both tables.
- Quantum Core X `007c7a6`: its item and monster managers and data models, and its test tables (header values and sizes only).
- The field meanings come from QCX. The record sizes are confirmed, but individual field meanings are not checked yet.

**Code:** `crates/m2-formats/src/proto.rs`.

## `item_proto`
All integers are little-endian.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `MIPX` |
| 4 | 4 | version (1) |
| 8 | 4 | record size (156) |
| 12 | 4 | record count |
| 16 | 4 | size of the container that follows |
| 20 | … | `MCOZ` container ([compression.md](compression.md)), encrypted with the `proto.item` key |

- The container holds the records back to back, with no padding. Its decoded size must be exactly count × record size.
- The layout is chosen by (version, record size). Only (1, 156) is known; any other combination is an error that names the known one.

### Item record, version 1 (156 bytes)
| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 4 | vnum | Item number |
| 4 | 4 | unknown | QCX calls it `Unknown` |
| 8 | 25 | name | NUL-terminated; often Korean (EUC-KR) |
| 33 | 25 | locale name | NUL-terminated; the client's language |
| 58 | 1 | type | |
| 59 | 1 | subtype | Meaning depends on the type |
| 60 | 1 | unknown | QCX calls it `Unknown2` |
| 61 | 1 | size | Inventory height in slots |
| 62 | 4 | anti flags | Who may not use it |
| 66 | 4 | flags | |
| 70 | 4 | wear flags | Where it is worn |
| 74 | 4 | immune flags | |
| 78 | 4 | buy price | |
| 82 | 4 | sell price | |
| 86 | 10 | limits | 2 × (type u8, value i32) |
| 96 | 15 | applies (bonuses) | 3 × (type u8, value i32) |
| 111 | 24 | values | 6 × i32 |
| 135 | 12 | sockets | 3 × i32 |
| 147 | 4 | upgrade vnum | Item it upgrades to |
| 151 | 2 | upgrade set | |
| 153 | 1 | magic item percent | |
| 154 | 1 | specular | |
| 155 | 1 | socket percent | |

Limit and apply values are read as signed. QCX reads them unsigned; the bytes are the same either way.

## `mob_proto`
| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `MMPT` |
| 4 | 4 | record count |
| 8 | 4 | size of the container that follows |
| 12 | … | `MCOZ` container, encrypted with the `proto.mob` key |

- There is no version or record size in the header. Records are 255 bytes; the decoded size must be count × 255.
- If it is a different multiple of the count, the error names the record size found.

### Monster record (255 bytes)
| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 4 | vnum | Monster number |
| 4 | 25 | name | Often Korean (EUC-KR) |
| 29 | 25 | locale name | |
| 54 | 1 | type | Monster, NPC, stone, … |
| 55 | 1 | rank | |
| 56 | 1 | battle type | |
| 57 | 1 | level | |
| 58 | 1 | size | |
| 59 | 4 | min gold | |
| 63 | 4 | max gold | |
| 67 | 4 | experience | |
| 71 | 4 | hp | |
| 75 | 1 | regen delay | |
| 76 | 1 | regen percent | |
| 77 | 2 | defence | |
| 79 | 4 | AI flags | |
| 83 | 4 | race flags | |
| 87 | 4 | immune flags | |
| 91 | 4 | st, dx, ht, iq | 1 byte each |
| 95 | 8 | damage range | min u32, max u32 |
| 103 | 2 | attack speed | i16 |
| 105 | 2 | move speed | i16 |
| 107 | 1 | aggressive percent | |
| 108 | 2 | aggressive sight | |
| 110 | 2 | attack range | |
| 112 | 6 | enchantments | 6 × u8 |
| 118 | 11 | resists | 11 × u8 |
| 129 | 4 | resurrection vnum | |
| 133 | 4 | drop item vnum | |
| 137 | 1 | mount capacity | |
| 138 | 1 | on click | |
| 139 | 1 | empire | |
| 140 | 65 | folder | Model folder, NUL-terminated |
| 205 | 4 | damage multiplier | f32 |
| 209 | 4 | summon vnum | |
| 213 | 4 | drain SP | |
| 217 | 4 | colour | |
| 221 | 4 | polymorph item vnum | |
| 225 | 25 | skills | 5 × (vnum u32, level u8) |
| 250 | 5 | berserk, stone skin, god speed, death blow, revive points | 1 byte each |

What some fields mean, from how Quantum Core X `007c7a6` uses them (`openm2 content import` reads them so, `docs/specs/server/content.md`):
- **type:** the entity kind EntitySpawn carries (`docs/specs/protocol/openm2/entities.md`, "Kinds and races"): 0 monster, 1 NPC, 2 metin stone, 3 warp, 4 door, 5 building, 6 player, 7 transformed player, 8 horse, 9 goto.
- **AI flags:** bit 0 aggressive (attacks players on sight), bit 1 never moves. Quantum Core X names ten more (coward, one "does not attack" bit per empire, attacks monsters, berserk, stone skin, god speed, death blow, revive) and uses none of them.
- **attack range** and **aggressive sight:** centimetres.
- **move speed** and **attack speed:** 100 is the speed of the monster's own motions; EntitySpawn carries them as one byte each.

## Tools
- `openm2 proto dump <file>`, or `openm2 proto dump --client <dir> locale/<name>/item_proto`, prints one JSON object per record. Names are escaped (`\xb0`).
- The tables are the user's data: dump them to your terminal or `local/`, never into the repo.
