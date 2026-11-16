# Combat

Choosing a target, attacking it, the damage both ways, dying and coming back.

## Choosing a target
- When the player picks an entity, the client sends **SetTarget** with its `vid`, in the game only.
- The server answers at once with **TargetHealth**: `percent`, the entity's health over its maximum, 0 to 100, rounded down. A player's own health counts too; what cannot be fought shows 100.
- SetTarget 0 looks at nothing. A vid the player does not see, or an item on the ground, is ignored, and the old target stays.
- After each hit an entity takes, every player looking at it gets TargetHealth again.
- A player stops looking at what goes out of its view.

## Attacking
The client leads its own attacks, as it leads its walks: it plays the motion and says when it hits.
1. **Motion:** CharacterMove `kind` 3 (combo) with the step in `argument`, from 1, or `kind` 2 (a single attack) with 0, where the character stands, facing the target. A walk in progress is first stopped with `kind` 0 at the point reached, as the server would otherwise walk the character on to the walk's end.
2. **Hit:** when the motion's hit lands, **Hit** with the `vid` hit and `skill` 0. A motion can hit more than once, and more than one entity.

What the server does with them:
- CharacterMove `kind` 2 and 3: passed on to the players around ([`movement.md`](movement.md)).
- Hit: it checks the hit (the player in the game and alive, a living monster or stone it sees, within 5 m, not too soon after its last hit on it) and ignores one that breaks a rule, without an answer ([`server/world.md`](../../server/world.md), "Hits"). There are no fights between players.
- A hit that counts: **Damage** to the attacker and TargetHealth to everyone looking at the target. The target fights back or dies.

**openm2:** `openm2 play` swings with `kind` 3 and the step (1 to 4) with a weapon, `kind` 2 bare-handed, and sends Hit when the motion's hit data touches a monster ([`render.md`](../../render.md), "Our fighting"); `openm2 bot` hits 200 ms after each step, without motion data.

## Damage
- **Damage** carries the entity hit (`vid`), `flags` and `amount`, the health taken. It goes to the attacker and, when a player is hit, to that player.
- `flags` may be combined: 1 normal, 2 poison, 4 dodged, 8 blocked, 16 piercing, 32 critical, 64 a heal (`amount` the health given), 128 burning. openm2's server sends 1, 2 (a poisoned monster's loss), 4 (a shot dodged, with 0), 8 (a melee bite blocked, with 0), 16 (a skill that pierces), 32 (a critical hit, doubled), 64 (a skill's heal, to the healer and the healed) and 128 (a burning monster's loss).
- How much a hit takes is the server's rule ([`server/world.md`](../../server/world.md), "Damage").

## Monsters fight back
A monster that is hit fights the player who hit it: it chases it to its reach and bites ([`server/world.md`](../../server/world.md), "Monsters fighting back"). Aggressive monsters also turn on a player who comes near. A bite sends, in this order:
1. Damage and PointsChanged to the player bitten, and TargetHealth to the players looking at it;
2. EntityMove `kind` 2 to the players that see the monster: where it stands, facing its victim, `duration` 0. The client plays its attack motion there.

A monster that stops without biting sends EntityMove `kind` 0 where it stands.

### Monsters that shoot
A monster that shoots (battle type 1 or 2, [`server/world.md`](../../server/world.md), "Monsters fighting back") bites the same way, then sends:

3. **Projectile** {`shooter` its vid, `target` the player's vid, `x`, `y` where the player is} to the players that see the monster.

The client turns it into a `Shot` event; `openm2 play` flies the flying effect of the attack motion's fly events from the shooter at the target ([`render.md`](../../render.md), "Flying effects"). A Projectile from an entity the client doesn't see is ignored.

### Flights
**Flight** {`kind`, `from`, `to`} (s2c 54): something flies from one entity to another, to the players that see `from`. Kind 0 is the experience a kill gave: from the monster killed to the player who gained it, right after that player's PointsChanged, as the original server sends it (its source). It flies for every kill that rewards the killer, however much experience that was: none (a monster far below the player gives 1 %, rounded down) or at the highest level, as the original's does.

The client turns it into a `Flight` event (`FlightKind::Experience`) when it sees both entities; a kind it doesn't know, or an entity it doesn't see, is ignored. `openm2 play` flies the asset folder's `[flies]` `experience` from the monster's feet at the player ([`render.md`](../../render.md), "Flying effects").

## Dying
- **A monster:** its killing blow sends Damage, TargetHealth 0, then EntityDied to the players that see it, then the killer's PointsChanged with its experience, then Flight of its experience to the players that see it. It lies there 5 s, then the players that see it get EntityRemove. What it leaves on the ground comes after ([`ground-items.md`](ground-items.md)).
- **Our character:** EntityDied with our own `vid`, to us and to the players that see us. The client ends its walk there, and while we are dead it ignores our walks, attacks, hits, skills, picking up, dropping and shopping. `openm2 play` shows the restart window ([`ui.md`](../../ui.md), "The in-game interface"). The server ignores the moves and hits of the dead.
- **Restart** comes back to life: with `town` false where we fell, with `town` true at our empire's start. The server takes it only while we are dead (a living player's restart is ignored); outside the game it closes the connection. It then:
  1. brings the character back with 50 health and mana, at most its maxima;
  2. sends us our own EntityRemove, EntitySpawn (`state` 0) and EntityInfo;
  3. sends what comes into and goes out of our view: the players that still see us get our EntityRemove, EntitySpawn and EntityInfo, those near our new place our EntitySpawn and EntityInfo;
  4. sends PointsChanged.
- The client keeps our character through the EntityRemove (it never removes its own). The spawn with `state` 0 says it lives, and `openm2 play` closes the restart window.
- A character that leaves the game dead enters dead, with the restart window ([`server/world.md`](../../server/world.md), "Dying").

## What the client keeps
- `World::target`: the entity chosen with `Command::Target`, until it leaves.
- `Entity::health`: the last percent TargetHealth gave for an entity.
- `Entity::argument`: an entity's last move's `argument`, the step of another player's combo, which `openm2 play` plays.
- `Entity::dead`: set by EntityDied, cleared by a spawn without `state` 1.
- Events: `Damaged` (vid, amount, flags), `Died` (vid), `Shot` (shooter, target, where), `Flight` (kind, from, to).
- `Command::Target { vid }`, `Attack { angle, combo }`, `Hit { vid, skill }` and `Restart { town }` work in the game only (an `Ignored` event otherwise); `Attack` and `Hit` not while we are dead.

## Not covered yet
- Players' arrows and spells, fights between players. Skills (Hit with a skill number, CharacterMove with bit 7) are in [`skills.md`](skills.md).
