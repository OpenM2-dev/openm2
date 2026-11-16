# Entities

The players, monsters and NPCs around our character: how they appear, change and leave, and our character's points.

## Ids
- `vid` is the server's id for an entity while it exists. Players, monsters, NPCs and items on the ground take theirs from one series ([`server/world.md`](../../server/world.md), "Entities and vids").
- Our own character's vid comes with MainCharacter ([`session.md`](session.md), "Entering the game").

## Kinds and races
EntitySpawn's `kind` says what an entity is, its `race` which one. The values are those the client's game files use:

| Kind | Entity | `race` is |
|---|---|---|
| 0 | monster | its number (vnum) in the monster table |
| 1 | NPC | its number |
| 2 | metin stone | its number |
| 3 | warp (portal) | its number |
| 4 | door | |
| 5 | building | |
| 6 | player | class and gender, 0 to 7 ([`session.md`](session.md), "Characters") |
| 7 | transformed player | |
| 8 | horse | |
| 9 | goto | |

- openm2's server sends 6 for players and, for monsters and NPCs, the kind their content gives ([`server/content.md`](../../server/content.md)).
- The client draws a player by its class's model script, and anything else by its race through the client's monster list ([`formats/script.md`](../../formats/script.md), "Monster list").

## Appearing and leaving
- **Range:** a player sees what comes within 100 m, and keeps seeing it until it is more than 120 m away ([`server/world.md`](../../server/world.md), "View").
- **Coming into view:**
  - a player: EntitySpawn, then EntityInfo;
  - an NPC: EntitySpawn, then EntityInfo with its name, empire 0 and level 0;
  - a monster or a stone: EntitySpawn only;
  - one that is walking: then an EntityMove with the rest of its walk ([`movement.md`](movement.md), "Other entities walk").
- **Going out of view,** or gone (it left the game, its corpse was removed): EntityRemove.
- **Entering the game:** our own EntitySpawn and EntityInfo first, then the players around, then the monsters and NPCs, then the items on the ground.
- **The dead:** a dead monster is not shown to players who come later; a dead player is shown lying (`state` 1).
- **Our own spawn again:** after a restart the server sends our EntityRemove, EntitySpawn and EntityInfo, with our new place ([`combat.md`](combat.md), "Dying"). The client takes the spawn as the truth: a walk in progress ends there.
- **EntityInfo again:** after a level-up, with the new level, to the player and to the players that see it.
- **EntityUpdate:** what a player wears, its speeds or its state changed ([`inventory.md`](inventory.md), [`skills.md`](skills.md)).
- **EntityDied:** an entity died; it stays until its EntityRemove ([`combat.md`](combat.md), "Dying").

## State and parts
- `state`: flag 1 is dead; flag 2 is a game master whose mark shows ([`../../server/session.md`](../../server/session.md), "Game masters"), only on players. openm2's server sends no other. `play` draws the mark over the name ([`../../render.md`](../../render.md), "Names over characters").
- `parts` (EntityInfo, EntityUpdate): body item, weapon item, head, hair.
  - openm2's server sends the item numbers of the worn armour and weapon, 0 for none, and 0 for head and hair.
  - The armour's item number becomes the model script's shape, the weapon's a model in the character's hand ([`formats/script.md`](../../formats/script.md), "Worn items").
  - The character list's `body_part` is the same armour item number.
- `move_speed`: 100 is the speed of the entity's run motion. Players have 150, changed by what they wear and by their skills ([`movement.md`](movement.md), "Speed").
- `attack_speed`: 100 is normal.
- `affects`: one bit per state shown on the entity (openm2's own numbering): bit 0 poisoned, 1 burning, 2 slowed, 3 stunned (monsters), 4 hidden (players). A monster's change comes as its EntityUpdate. `play` draws the states ([`../../render.md`](../../render.md), "Effects") and leaves a hidden player out.

## Points
- **Points**, while loading: every point of our character that is not 0, as `point` and `value` pairs. Points left out are 0.
- **PointsChanged**, afterwards: only the points that changed, with their new values; a point that became 0 comes with 0. The server sends it whenever a point changes: damage, regeneration, experience, gold, what we wear, skills, a restart ([`server/world.md`](../../server/world.md), "Points").
- Point numbers keep the values the client's game files use. openm2's server fills these:

| Point | Meaning |
|---|---|
| 1 | level |
| 3, 4 | experience, experience needed for the next level |
| 5, 6 | health, maximum health |
| 7, 8 | mana, maximum mana |
| 11 | gold |
| 12, 13, 14, 15 | strength, vitality, dexterity, intelligence |
| 16 | defence grade |
| 17 | attack speed |
| 18 | attack grade |
| 19 | movement speed |
| 20 | defence |
| 26, 27, 28 | status points, sub-skill points, skill points |
| 31 | play time, in minutes |

## Spending a status point
- **RaiseStatus** `point` spends one status point (point 26) on strength (12), vitality (13), dexterity (14) or intelligence (15).
- The server takes it while there are status points and the stat is below its highest. It raises the stat by one and works out the maxima, defence and attack grade again, without healing. It answers PointsChanged ([`server/world.md`](../../server/world.md), "Points").
- Refusals get an info line: no status points left, the stat at its highest. Another point number is ignored.
- `Command::RaiseStatus { point }` sends it, in the game only, for points 12 to 15; another number is an `Ignored` event.

## Effects
- **EntityEffect** {`kind`, `vid`} (s2c 55): something happened to an entity that shows on it, to the player it happened to and to the players that see it, as the original server sends its effects (its source). Kind 0 is the health a potion gave back, 1 the mana a potion gave back, 2 a level gained.
  - **A potion** sends 0 for health and 1 for mana after its ItemSet, PointsChanged and TargetHealth ([`inventory.md`](inventory.md)); one that gives both sends 0 then 1; one that gives nothing back is kept and sends none.
  - **A level-up** sends 2 after the player's PointsChanged, before its EntityInfo: one for experience that brings one level or more. `/level` sends it when the level goes up ([`../../server/session.md`](../../server/session.md), "Game masters").
- The client turns it into an `EntityEffect` event (`EntityEffectKind::HealthPotion`, `ManaPotion`, `LevelUp`) when it sees the entity; a kind it doesn't know, or an entity it doesn't see, is ignored. `openm2 play` plays the asset folder's `[effects]` `health_potion`, `mana_potion` or `level_up` once on it, and our own potions' `potion_sound` ([`../../render.md`](../../render.md), "Effects"; [`../../audio.md`](../../audio.md)).

## Skills on players
- **EntitySkills** {`vid`, `skills`} (s2c 56): the skills on a player now, by number (up to 16): those its raised points come from and those it has on ([`skills.md`](skills.md), "What skills raise"). The whole list whenever it changes, empty when the last ends, to the player and the players that see it; to a player who comes to see it, after its EntitySpawn and EntityInfo (and EntityMount), when skills are on it. The server sends numbers, not looks: the client's assets say how each skill looks while it is on, as the original client shows the states the server tells it of.
- The client keeps the list as the entity's `skills` when it sees the entity (one it doesn't see is ignored), over a spawn of an entity it knows too. `openm2 play` plays each skill's look from the asset folder's skills file on the player ([`../../render.md`](../../render.md), "Effects"; [`../../assets.md`](../../assets.md), "Skills").

## What the client keeps
- `m2_core::World::entities`: every entity by vid, our own too: kind, race, where it is over time (a `Track`, [`movement.md`](movement.md)), the direction it faces, its last move's kind and argument, speeds, state, parts, its health while it is our target, and EntityInfo's name, empire, level and guild.
- A spawn of an entity the client knows replaces it, but keeps its parts and what its EntityInfo said.
- EntityRemove of our own vid is ignored: our character stays. Messages for a vid the client does not know are ignored (an `Ignored` event).
- Phase 1 or 2 clears the entities.
- `m2_core::World::points`: the points that are not 0. Points replaces them; PointsChanged changes those it names and drops those that became 0.
